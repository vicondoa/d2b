//! Checked-in Cargo production-closure and advisory-policy authority.
//!
//! Cargo.lock remains the only resolution authority.  This module asks Cargo
//! for locked metadata, traverses only the selected context roots, and writes
//! deterministic projections for consumers that need a smaller, reviewed
//! inventory.  The filtered lock projections are audit inputs only: no Cargo
//! invocation is ever pointed at one of them.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const OUTPUT_ROOT: &str = "packages/policy-inputs";
pub const ADVISORY_POLICY_PATH: &str = "packages/policy-inputs/advisory-policy.json";
pub const APPROVAL_MARKER: &str = "d2b-production-closure-approved/v1";
const PRODUCT_LOCK: &str = "Cargo.lock";
const CODEOWNERS_PATH: &str = ".github/CODEOWNERS";
const PROTECTED_OWNER: &str = "@vicondoa";
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
    "/packages/d2b-contract-tests/tests/policy_production_closure.rs @vicondoa",
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
type LockChecksumIndex = BTreeMap<(String, String, Option<String>), String>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextSpec {
    pub system: String,
    pub target: String,
    pub name: String,
    pub roots: Vec<String>,
    pub features: Vec<String>,
    pub default_features: bool,
    pub source_authority: String,
    pub lock_path: String,
}

impl ContextSpec {
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.system, self.target, self.name)
    }

    fn production_kinds(&self) -> BTreeSet<&'static str> {
        ["normal", "build", "proc-macro"].into_iter().collect()
    }

    fn policy_kinds(&self) -> BTreeSet<&'static str> {
        [
            "normal",
            "build",
            "proc-macro",
            "dev",
            "test",
            "example",
            "bench",
        ]
        .into_iter()
        .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct PackageRecord {
    id: String,
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    target: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct EdgeRecord {
    from: String,
    to: String,
    kind: String,
    target: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Closure {
    schema_version: u32,
    authority: String,
    context: String,
    system: String,
    target: String,
    roots: Vec<String>,
    features: Vec<String>,
    default_features: bool,
    source_authority: String,
    lock_sha256: String,
    packages: Vec<PackageRecord>,
    edges: Vec<EdgeRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval: Option<ApprovalProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ApprovalProjection {
    marker: String,
    owner: String,
    expires_at: String,
    rationale: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ComputedContext {
    spec: ContextSpec,
    production: Closure,
    policy: Closure,
    metadata: Value,
}

#[derive(Debug)]
struct AdvisoryContext {
    approval: ApprovalProjection,
}

pub fn run_cli(root: &Path, args: &[String]) -> Result<Vec<PathBuf>, String> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        [flag] if flag == "--write" => false,
        _ => {
            return Err(
                "usage: cargo xtask gen-package-policy-inputs [--check|--write]".to_owned(),
            );
        }
    };
    if check {
        check_outputs(root)
    } else {
        generate_outputs(root)
    }
}

pub fn context_specs(root: &Path) -> Result<Vec<ContextSpec>, String> {
    let metadata = cargo_metadata(root, "x86_64-unknown-linux-gnu", &[], true)?;
    let product_roots = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "Cargo metadata has no package array".to_owned())?
        .iter()
        .filter_map(|package| package.get("name").and_then(Value::as_str))
        .filter(|name| {
            !matches!(
                *name,
                "d2b-priv-broker" | "d2b-guest-shell-runner" | "d2b-contract-tests" | "xtask"
            ) && (*name == "d2b" || name.starts_with("d2b-"))
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if product_roots.is_empty() {
        return Err("main product context has no Cargo roots".to_owned());
    }

    let mut contexts = Vec::new();
    for (system, arch) in [("x86_64-linux", "x86_64"), ("aarch64-linux", "aarch64")] {
        let gnu = format!("{arch}-unknown-linux-gnu");
        let musl = format!("{arch}-unknown-linux-musl");
        contexts.push(ContextSpec {
            system: system.to_owned(),
            target: gnu.clone(),
            name: "main-product".to_owned(),
            roots: product_roots.clone(),
            features: Vec::new(),
            default_features: true,
            source_authority: "Cargo.lock".to_owned(),
            lock_path: PRODUCT_LOCK.to_owned(),
        });
        contexts.push(ContextSpec {
            system: system.to_owned(),
            target: gnu.clone(),
            name: "broker-production".to_owned(),
            roots: vec!["d2b-priv-broker".to_owned()],
            features: Vec::new(),
            default_features: false,
            source_authority: "Cargo.lock".to_owned(),
            lock_path: PRODUCT_LOCK.to_owned(),
        });
        for (name, feature) in [
            ("broker-default-tests", None),
            ("broker-layer1-bootstrap-tests", Some("layer1-bootstrap")),
            ("broker-fake-backends-tests", Some("fake-backends")),
        ] {
            contexts.push(ContextSpec {
                system: system.to_owned(),
                target: gnu.clone(),
                name: name.to_owned(),
                roots: vec!["d2b-priv-broker".to_owned()],
                features: feature.into_iter().map(str::to_owned).collect(),
                default_features: false,
                source_authority: "Cargo.lock".to_owned(),
                lock_path: PRODUCT_LOCK.to_owned(),
            });
        }
        contexts.push(ContextSpec {
            system: system.to_owned(),
            target: musl.clone(),
            name: "guest-shell-runner-static".to_owned(),
            roots: vec!["d2b-guest-shell-runner".to_owned()],
            features: vec!["real-libshpool".to_owned()],
            default_features: false,
            source_authority: "Cargo.lock".to_owned(),
            lock_path: PRODUCT_LOCK.to_owned(),
        });
        contexts.push(ContextSpec {
            system: system.to_owned(),
            target: musl,
            name: "guestd-static".to_owned(),
            roots: vec!["d2b-guestd".to_owned()],
            features: Vec::new(),
            default_features: true,
            source_authority: "packages/Cargo.guest.lock".to_owned(),
            lock_path: "packages/Cargo.guest.lock".to_owned(),
        });
    }
    Ok(contexts)
}

fn generate_outputs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let contexts = context_specs(root)?;
    let policy = read_advisory_policy(root, false)?;
    let mut written = Vec::new();
    for spec in &contexts {
        let computed = compute_context(root, spec.clone())?;
        let approval = policy
            .get(&spec.key())
            .map(|context| context.approval.clone());
        write_context(root, &computed, approval, &mut written)?;
    }
    if !root.join(ADVISORY_POLICY_PATH).exists() {
        write_advisory_skeleton(root, &contexts, &mut written)?;
    }
    Ok(written)
}

fn check_outputs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let contexts = context_specs(root)?;
    let policy = read_advisory_policy(root, true)?;
    let expected_keys = contexts
        .iter()
        .map(ContextSpec::key)
        .collect::<BTreeSet<_>>();
    let actual_keys = policy.keys().cloned().collect::<BTreeSet<_>>();
    if actual_keys != expected_keys {
        return Err(format!(
            "advisory policy contexts differ: expected {expected_keys:?}, found {actual_keys:?}"
        ));
    }

    let mut checked = Vec::new();
    for spec in contexts {
        let computed = compute_context(root, spec.clone())?;
        let advisory = policy
            .get(&spec.key())
            .ok_or_else(|| format!("missing advisory context {}", spec.key()))?;
        let approval = advisory.approval.clone();
        let expected_production = serde_json::to_string_pretty(&with_approval(
            &computed.production,
            Some(approval.clone()),
        ))
        .map_err(|error| format!("serialize production closure: {error}"))?
            + "\n";
        let expected_policy = serde_json::to_string_pretty(&computed.policy)
            .map_err(|error| format!("serialize policy closure: {error}"))?
            + "\n";
        let directory = root
            .join(OUTPUT_ROOT)
            .join(&spec.system)
            .join(&spec.target)
            .join(&spec.name);
        compare_file(
            &directory.join("production/closure.json"),
            &expected_production,
        )?;
        compare_file(&directory.join("policy/closure.json"), &expected_policy)?;
        compare_file(
            &directory.join("production/metadata.json"),
            &(serde_json::to_string_pretty(&computed.metadata)
                .map_err(|error| format!("serialize metadata: {error}"))?
                + "\n"),
        )?;
        compare_file(
            &directory.join("policy/metadata.json"),
            &(serde_json::to_string_pretty(&computed.metadata)
                .map_err(|error| format!("serialize metadata: {error}"))?
                + "\n"),
        )?;
        check_audit_lock(
            &directory.join("production/Cargo.lock"),
            &computed.production,
        )?;
        check_audit_lock(&directory.join("policy/Cargo.lock"), &computed.policy)?;
        reject_extra_files(&directory)?;
        checked.push(directory);
    }
    Ok(checked)
}

fn compute_context(root: &Path, spec: ContextSpec) -> Result<ComputedContext, String> {
    if spec.roots.is_empty() {
        return Err(format!("context {} has no roots", spec.key()));
    }
    if spec.lock_path != PRODUCT_LOCK {
        return compute_lock_context(root, spec);
    }
    let metadata = cargo_metadata(root, &spec.target, &spec.features, spec.default_features)?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{} metadata has no packages", spec.key()))?;
    let mut by_id = BTreeMap::<String, Value>::new();
    let mut by_name = BTreeMap::<String, Vec<String>>::new();
    for package in packages {
        let id = package
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{} package has no id", spec.key()))?
            .to_owned();
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{} package has no name", spec.key()))?
            .to_owned();
        by_name.entry(name).or_default().push(id.clone());
        by_id.insert(id, package.clone());
    }
    let mut root_ids = BTreeSet::new();
    for root_name in &spec.roots {
        let ids = by_name
            .get(root_name)
            .ok_or_else(|| format!("{} root package is missing: {root_name}", spec.key()))?;
        if ids.len() != 1 {
            return Err(format!(
                "{} root package {root_name} is ambiguous: {ids:?}",
                spec.key()
            ));
        }
        root_ids.insert(ids[0].clone());
    }
    let resolve_nodes = metadata
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{} metadata has no resolve nodes", spec.key()))?;
    let mut nodes = BTreeMap::<String, Value>::new();
    for node in resolve_nodes {
        let id = node
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{} resolve node has no id", spec.key()))?;
        nodes.insert(id.to_owned(), node.clone());
    }
    let production_kinds = spec.production_kinds();
    let policy_kinds = spec.policy_kinds();
    let (production_ids, production_edges) =
        traverse(&root_ids, &nodes, &by_id, &production_kinds, &spec.target)?;
    let (policy_ids, policy_edges) =
        traverse(&root_ids, &nodes, &by_id, &policy_kinds, &spec.target)?;
    if production_ids.is_empty() || policy_ids.is_empty() {
        return Err(format!("{} produced an empty closure", spec.key()));
    }
    let lock_sha256 = sha256_file(&root.join(PRODUCT_LOCK))?;
    let lock_packages = lock_packages(root, PRODUCT_LOCK)?;
    let production = make_closure(
        &spec,
        &by_id,
        &production_ids,
        &production_edges,
        &lock_sha256,
        &lock_packages,
        None,
    )?;
    let policy = make_closure(
        &spec,
        &by_id,
        &policy_ids,
        &policy_edges,
        &lock_sha256,
        &lock_packages,
        None,
    )?;
    let metadata = metadata_projection(&spec, &production, &policy, &metadata);
    Ok(ComputedContext {
        spec,
        production,
        policy,
        metadata,
    })
}

#[derive(Clone, Debug)]
struct LockPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    dependencies: Vec<String>,
}

fn compute_lock_context(root: &Path, spec: ContextSpec) -> Result<ComputedContext, String> {
    let lock_packages = parse_lock_packages(root.join(&spec.lock_path))?;
    let mut by_name = BTreeMap::<String, Vec<String>>::new();
    let mut packages = BTreeMap::<String, Value>::new();
    for package in &lock_packages {
        let value = json!({
            "name": package.name,
            "version": package.version,
            "source": package.source,
            "targets": [{"kind": ["lib"]}]
        });
        let id = stable_id(&value);
        by_name
            .entry(package.name.clone())
            .or_default()
            .push(id.clone());
        packages.insert(id, value);
    }
    let root_ids = spec
        .roots
        .iter()
        .map(|name| {
            let ids = by_name
                .get(name)
                .ok_or_else(|| format!("{} root package is missing: {name}", spec.key()))?;
            if ids.len() != 1 {
                return Err(format!(
                    "{} root package {name} is ambiguous: {ids:?}",
                    spec.key()
                ));
            }
            Ok(ids[0].clone())
        })
        .collect::<Result<BTreeSet<_>, String>>()?;
    let records = lock_packages
        .iter()
        .map(|package| {
            let value = json!({
                "name": package.name,
                "version": package.version,
                "source": package.source,
                "targets": [{"kind": ["lib"]}]
            });
            (stable_id(&value), package)
        })
        .collect::<BTreeMap<_, _>>();
    let mut selected = root_ids.clone();
    let mut queue = root_ids.iter().cloned().collect::<VecDeque<_>>();
    let mut edges = Vec::new();
    while let Some(from) = queue.pop_front() {
        let package = records
            .get(&from)
            .ok_or_else(|| format!("lock package is missing for {from}"))?;
        for dependency in &package.dependencies {
            let mut fields = dependency.split_whitespace();
            let name = fields.next().unwrap_or_default();
            let requested_version = fields.next();
            let candidates = by_name
                .get(name)
                .ok_or_else(|| format!("{} dependency is missing: {dependency}", spec.key()))?;
            let to = candidates
                .iter()
                .find(|candidate| {
                    requested_version.is_none_or(|version| {
                        records
                            .get(*candidate)
                            .is_some_and(|package| package.version == version)
                    })
                })
                .ok_or_else(|| {
                    format!("{} dependency version is missing: {dependency}", spec.key())
                })?
                .clone();
            edges.push(EdgeRecord {
                from: from.clone(),
                to: to.clone(),
                kind: "normal".to_owned(),
                target: None,
            });
            if selected.insert(to.clone()) {
                queue.push_back(to);
            }
        }
    }
    let lock_sha256 = sha256_file(&root.join(&spec.lock_path))?;
    let production = make_lock_closure(&spec, &records, &selected, &edges, &lock_sha256)?;
    let policy = production.clone();
    let metadata = metadata_projection(
        &spec,
        &production,
        &policy,
        &json!({ "resolve": { "nodes": [] } }),
    );
    Ok(ComputedContext {
        spec,
        production,
        policy,
        metadata,
    })
}

fn traverse(
    roots: &BTreeSet<String>,
    nodes: &BTreeMap<String, Value>,
    packages: &BTreeMap<String, Value>,
    allowed_kinds: &BTreeSet<&'static str>,
    target: &str,
) -> Result<(BTreeSet<String>, Vec<EdgeRecord>), String> {
    let mut selected = roots.clone();
    let mut queue = roots.iter().cloned().collect::<VecDeque<_>>();
    let mut edges = BTreeSet::<(String, String, String, Option<String>)>::new();
    while let Some(from_id) = queue.pop_front() {
        let node = nodes
            .get(&from_id)
            .ok_or_else(|| format!("resolve node missing for {from_id}"))?;
        let dependencies = node
            .get("deps")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("resolve node {from_id} has no deps"))?;
        for dependency in dependencies {
            let to_id = dependency
                .get("pkg")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("dependency from {from_id} has no package id"))?
                .to_owned();
            if !packages.contains_key(&to_id) {
                return Err(format!("dependency points outside metadata: {to_id}"));
            }
            let dep_kinds = dependency
                .get("dep_kinds")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("dependency from {from_id} has no dep_kinds"))?;
            for dep_kind in dep_kinds {
                let raw_kind = dep_kind
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("normal");
                let cfg = dep_kind
                    .get("target")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if !allowed_kinds.contains(raw_kind) || !target_matches(cfg.as_deref(), target) {
                    continue;
                }
                let kind = if package_is_proc_macro(packages.get(&to_id).expect("checked")) {
                    "proc-macro"
                } else {
                    raw_kind
                }
                .to_owned();
                edges.insert((from_id.clone(), to_id.clone(), kind, cfg));
                if selected.insert(to_id.clone()) {
                    queue.push_back(to_id.clone());
                }
            }
        }
    }
    Ok((
        selected,
        edges
            .into_iter()
            .map(|(from, to, kind, target)| EdgeRecord {
                from,
                to,
                kind,
                target,
            })
            .collect(),
    ))
}

fn make_closure(
    spec: &ContextSpec,
    packages: &BTreeMap<String, Value>,
    selected: &BTreeSet<String>,
    edges: &[EdgeRecord],
    lock_sha256: &str,
    lock_packages: &LockChecksumIndex,
    approval: Option<ApprovalProjection>,
) -> Result<Closure, String> {
    let mut package_records = selected
        .iter()
        .map(|id| {
            package_record(
                packages.get(id).expect("selected package"),
                &spec.target,
                lock_packages,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    package_records.sort_by(|left, right| left.id.cmp(&right.id));
    let mut edge_records = edges
        .iter()
        .filter(|edge| selected.contains(&edge.from) && selected.contains(&edge.to))
        .map(|edge| EdgeRecord {
            from: stable_id(packages.get(&edge.from).expect("edge source package")),
            to: stable_id(packages.get(&edge.to).expect("edge target package")),
            kind: edge.kind.clone(),
            target: edge.target.clone(),
        })
        .collect::<Vec<_>>();
    edge_records.sort_by(|left, right| {
        (&left.from, &left.to, &left.kind, &left.target).cmp(&(
            &right.from,
            &right.to,
            &right.kind,
            &right.target,
        ))
    });
    Ok(Closure {
        schema_version: 1,
        authority: "cargo-locked-metadata".to_owned(),
        context: spec.name.clone(),
        system: spec.system.clone(),
        target: spec.target.clone(),
        roots: spec.roots.clone(),
        features: spec.features.clone(),
        default_features: spec.default_features,
        source_authority: spec.source_authority.clone(),
        lock_sha256: lock_sha256.to_owned(),
        packages: package_records,
        edges: edge_records,
        approval,
    })
}

fn make_lock_closure(
    spec: &ContextSpec,
    packages: &BTreeMap<String, &LockPackage>,
    selected: &BTreeSet<String>,
    edges: &[EdgeRecord],
    lock_sha256: &str,
) -> Result<Closure, String> {
    let mut package_records = selected
        .iter()
        .map(|id| {
            let package = packages
                .get(id)
                .ok_or_else(|| format!("lock package is missing for {id}"))?;
            Ok(PackageRecord {
                id: id.clone(),
                name: package.name.clone(),
                version: package.version.clone(),
                source: package.source.clone(),
                checksum: package.checksum.clone(),
                target: spec.target.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    package_records.sort_by(|left, right| left.id.cmp(&right.id));
    let mut edge_records = edges
        .iter()
        .filter(|edge| selected.contains(&edge.from) && selected.contains(&edge.to))
        .cloned()
        .collect::<Vec<_>>();
    edge_records.sort_by(|left, right| {
        (&left.from, &left.to, &left.kind, &left.target).cmp(&(
            &right.from,
            &right.to,
            &right.kind,
            &right.target,
        ))
    });
    Ok(Closure {
        schema_version: 1,
        authority: "cargo-locked-metadata".to_owned(),
        context: spec.name.clone(),
        system: spec.system.clone(),
        target: spec.target.clone(),
        roots: spec.roots.clone(),
        features: spec.features.clone(),
        default_features: spec.default_features,
        source_authority: spec.source_authority.clone(),
        lock_sha256: lock_sha256.to_owned(),
        packages: package_records,
        edges: edge_records,
        approval: None,
    })
}

fn package_record(
    package: &Value,
    target: &str,
    lock_packages: &LockChecksumIndex,
) -> Result<PackageRecord, String> {
    let name = package
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "package has no name".to_owned())?
        .to_owned();
    let version = package
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| "package has no version".to_owned())?
        .to_owned();
    let source = package
        .get("source")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(PackageRecord {
        id: stable_id(package),
        name: name.clone(),
        version: version.clone(),
        source: source.clone(),
        checksum: lock_packages
            .get(&(name, version, source))
            .filter(|checksum| !checksum.is_empty())
            .cloned(),
        target: target.to_owned(),
    })
}

fn parse_lock_packages(path: PathBuf) -> Result<Vec<LockPackage>, String> {
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("read guest lock {}: {error}", path.display()))?;
    let mut packages = Vec::new();
    let mut current_name = None;
    let mut current_version = None;
    let mut current_source = None;
    let mut current_checksum = None;
    let mut current_dependencies = Vec::new();
    let mut in_dependencies = false;
    let finish = |packages: &mut Vec<LockPackage>,
                  current_name: &mut Option<String>,
                  current_version: &mut Option<String>,
                  current_source: &mut Option<String>,
                  current_checksum: &mut Option<String>,
                  current_dependencies: &mut Vec<String>| {
        if let (Some(name), Some(version)) = (current_name.take(), current_version.take()) {
            packages.push(LockPackage {
                name,
                version,
                source: current_source.take(),
                checksum: current_checksum.take(),
                dependencies: std::mem::take(current_dependencies),
            });
        } else {
            current_name.take();
            current_version.take();
            current_source.take();
            current_checksum.take();
            current_dependencies.clear();
        }
    };
    for line in text.lines() {
        if line == "[[package]]" {
            finish(
                &mut packages,
                &mut current_name,
                &mut current_version,
                &mut current_source,
                &mut current_checksum,
                &mut current_dependencies,
            );
            in_dependencies = false;
            continue;
        }
        if line == "dependencies = [" {
            in_dependencies = true;
            continue;
        }
        if in_dependencies {
            if line == "]" {
                in_dependencies = false;
            } else if let Some(value) = line
                .trim()
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix("\","))
                .or_else(|| {
                    line.trim()
                        .strip_prefix('"')
                        .and_then(|value| value.strip_suffix('"'))
                })
            {
                current_dependencies.push(value.to_owned());
            }
            continue;
        }
        for (key, slot) in [
            ("name", &mut current_name),
            ("version", &mut current_version),
            ("source", &mut current_source),
            ("checksum", &mut current_checksum),
        ] {
            let prefix = format!("{key} = \"");
            if let Some(value) = line.strip_prefix(&prefix).and_then(|v| v.strip_suffix('"')) {
                *slot = Some(value.to_owned());
            }
        }
    }
    finish(
        &mut packages,
        &mut current_name,
        &mut current_version,
        &mut current_source,
        &mut current_checksum,
        &mut current_dependencies,
    );
    if packages.is_empty() {
        return Err(format!("lock file {} has no packages", path.display()));
    }
    Ok(packages)
}

fn stable_id(package: &Value) -> String {
    let name = package
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let version = package
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let source = package
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("path");
    format!("{name}@{version}#{source}")
}

fn lock_packages(root: &Path, relative_lock: &str) -> Result<LockChecksumIndex, String> {
    let text = fs::read_to_string(root.join(relative_lock))
        .map_err(|error| format!("read {relative_lock}: {error}"))?;
    let mut result = BTreeMap::new();
    let mut current = BTreeMap::<String, String>::new();
    let mut finish = |current: &mut BTreeMap<String, String>| {
        let Some(name) = current.remove("name") else {
            current.clear();
            return;
        };
        let Some(version) = current.remove("version") else {
            current.clear();
            return;
        };
        let source = current.remove("source");
        let checksum = current.remove("checksum");
        result.insert((name, version, source), checksum.unwrap_or_default());
        current.clear();
    };
    for line in text.lines() {
        if line == "[[package]]" {
            finish(&mut current);
            continue;
        }
        for key in ["name", "version", "source", "checksum"] {
            let prefix = format!("{key} = \"");
            if let Some(value) = line.strip_prefix(&prefix).and_then(|v| v.strip_suffix('"')) {
                current.insert(key.to_owned(), value.to_owned());
            }
        }
    }
    finish(&mut current);
    Ok(result)
}

fn metadata_projection(
    spec: &ContextSpec,
    production: &Closure,
    policy: &Closure,
    metadata: &Value,
) -> Value {
    let package_ids = policy
        .packages
        .iter()
        .map(|package| package.id.clone())
        .collect::<Vec<_>>();
    let node_count = metadata
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    json!({
        "schemaVersion": 1,
        "authority": "cargo-locked-metadata",
        "target": spec.target,
        "system": spec.system,
        "roots": spec.roots,
        "features": spec.features,
        "defaultFeatures": spec.default_features,
        "sourceAuthority": spec.source_authority,
        "lockSha256": production.lock_sha256,
        "resolveNodeCount": node_count,
        "productionPackageCount": production.packages.len(),
        "policyPackageCount": policy.packages.len(),
        "policyPackageIds": package_ids
    })
}

fn write_context(
    root: &Path,
    computed: &ComputedContext,
    approval: Option<ApprovalProjection>,
    written: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let directory = root
        .join(OUTPUT_ROOT)
        .join(&computed.spec.system)
        .join(&computed.spec.target)
        .join(&computed.spec.name);
    fs::create_dir_all(directory.join("production"))
        .map_err(|error| format!("create production policy directory: {error}"))?;
    fs::create_dir_all(directory.join("policy"))
        .map_err(|error| format!("create policy directory: {error}"))?;
    let production = with_approval(&computed.production, approval);
    write_json(
        &directory.join("production/closure.json"),
        &production,
        written,
    )?;
    write_json(
        &directory.join("policy/closure.json"),
        &computed.policy,
        written,
    )?;
    write_json(
        &directory.join("production/metadata.json"),
        &computed.metadata,
        written,
    )?;
    write_json(
        &directory.join("policy/metadata.json"),
        &computed.metadata,
        written,
    )?;
    write_text(
        &directory.join("production/Cargo.lock"),
        &filtered_lock(root, &computed.spec.lock_path, &computed.production)?,
        written,
    )?;
    write_text(
        &directory.join("policy/Cargo.lock"),
        &filtered_lock(root, &computed.spec.lock_path, &computed.policy)?,
        written,
    )?;
    Ok(())
}

fn with_approval(closure: &Closure, approval: Option<ApprovalProjection>) -> Closure {
    let mut result = closure.clone();
    result.approval = approval;
    result
}

fn write_advisory_skeleton(
    root: &Path,
    contexts: &[ContextSpec],
    written: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let context_values = contexts
        .iter()
        .map(|context| {
            (
                context.key(),
                json!({
                    "approval": {
                        "marker": APPROVAL_MARKER,
                        "owner": "",
                        "expiresAt": "1970-01-01",
                        "rationale": ""
                    },
                    "advisories": []
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let value = json!({
        "schemaVersion": 1,
        "authority": "context-scoped-advisory-policy",
        "requiresProtectedReview": true,
        "recomputation": {
            "command": "cargo xtask gen-package-policy-inputs --check",
            "mode": "independent-locked-metadata-recompute",
            "trustedApprovalRequired": true
        },
        "protectedOwnership": {
            "path": CODEOWNERS_PATH,
            "owner": "@vicondoa"
        },
        "contexts": context_values
    });
    write_json(&root.join(ADVISORY_POLICY_PATH), &value, written)
}

fn read_advisory_policy(
    root: &Path,
    require_approved: bool,
) -> Result<BTreeMap<String, AdvisoryContext>, String> {
    let path = root.join(ADVISORY_POLICY_PATH);
    if !path.exists() {
        if require_approved {
            return Err(format!("missing approval metadata: {}", path.display()));
        }
        return Ok(BTreeMap::new());
    }
    let bytes = fs::read(&path).map_err(|error| format!("read advisory policy: {error}"))?;
    let leaked = Box::leak(bytes.into_boxed_slice());
    let value: &'static Value = Box::leak(Box::new(
        serde_json::from_slice(leaked)
            .map_err(|error| format!("parse advisory policy: {error}"))?,
    ));
    validate_advisory_policy(root, value, require_approved)?;
    let contexts = value
        .get("contexts")
        .and_then(Value::as_object)
        .ok_or_else(|| "advisory policy contexts must be an object".to_owned())?;
    contexts
        .iter()
        .map(|(key, context)| {
            Ok((
                key.clone(),
                AdvisoryContext {
                    approval: parse_approval(context)?,
                },
            ))
        })
        .collect()
}

fn validate_advisory_policy(
    root: &Path,
    value: &Value,
    require_approved: bool,
) -> Result<(), String> {
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || value.get("authority").and_then(Value::as_str) != Some("context-scoped-advisory-policy")
    {
        return Err("advisory policy has an unknown schema or authority".to_owned());
    }
    if value
        .get("requiresProtectedReview")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("advisory policy must require protected review".to_owned());
    }
    let recomputation = value
        .get("recomputation")
        .and_then(Value::as_object)
        .ok_or_else(|| "advisory policy is missing recomputation metadata".to_owned())?;
    if recomputation.get("command").and_then(Value::as_str)
        != Some("cargo xtask gen-package-policy-inputs --check")
        || recomputation.get("mode").and_then(Value::as_str)
            != Some("independent-locked-metadata-recompute")
        || recomputation
            .get("trustedApprovalRequired")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return Err("advisory policy recomputation metadata is invalid".to_owned());
    }
    // This checks the checked-in ownership declaration only. GitHub branch
    // protection and the resulting review are external merge prerequisites.
    let ownership = value
        .get("protectedOwnership")
        .and_then(Value::as_object)
        .ok_or_else(|| "advisory policy is missing protected ownership metadata".to_owned())?;
    let owner = ownership
        .get("owner")
        .and_then(Value::as_str)
        .filter(|owner| *owner == PROTECTED_OWNER)
        .ok_or_else(|| "protected ownership owner is missing".to_owned())?;
    let ownership_path = ownership
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| *path == CODEOWNERS_PATH)
        .ok_or_else(|| "protected ownership path is missing".to_owned())?;
    let codeowners = fs::read_to_string(root.join(ownership_path))
        .map_err(|error| format!("read protected ownership file: {error}"))?;
    let rules = codeowners
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<BTreeSet<_>>();
    for required_rule in PROTECTED_CODEOWNERS_RULES {
        if !rules.contains(required_rule) {
            return Err(format!(
                "protected ownership file {} is missing exact rule {required_rule}",
                ownership_path
            ));
        }
    }
    if value.get("ignore").is_some() || value.get("ignores").is_some() {
        return Err("global advisory ignores are forbidden".to_owned());
    }
    let contexts = value
        .get("contexts")
        .and_then(Value::as_object)
        .ok_or_else(|| "advisory policy contexts must be an object".to_owned())?;
    let mut advisory_owners = BTreeMap::<String, String>::new();
    for (key, context) in contexts {
        let approval = parse_approval(context)?;
        if require_approved && approval.marker != APPROVAL_MARKER {
            return Err(format!("context {key} has an invalid approval marker"));
        }
        if require_approved && approval.owner != owner {
            return Err(format!(
                "context {key} approval owner {} is not protected owner {owner}",
                approval.owner
            ));
        }
        if require_approved && approval.rationale.trim().is_empty() {
            return Err(format!("context {key} approval rationale is empty"));
        }
        if approval.expires_at < today_iso() {
            return Err(format!("context {key} approval is expired"));
        }
        let advisories = context
            .get("advisories")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("context {key} advisories must be an array"))?;
        for advisory in advisories {
            let id = advisory
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("context {key} advisory has no id"))?;
            if advisory_owners.insert(id.to_owned(), key.clone()).is_some() {
                return Err(format!("advisory {id} is ignored in more than one context"));
            }
            for field in ["rationale", "owner", "expiresAt", "approvalMarker"] {
                if advisory.get(field).and_then(Value::as_str).is_none() {
                    return Err(format!("context {key} advisory {id} is missing {field}"));
                }
            }
            if advisory
                .get("expiresAt")
                .and_then(Value::as_str)
                .is_some_and(|expiry| expiry < today_iso().as_str())
            {
                return Err(format!("context {key} advisory {id} is expired"));
            }
            if advisory.get("owner").and_then(Value::as_str) != Some(owner) {
                return Err(format!(
                    "context {key} advisory {id} owner is not protected"
                ));
            }
            if require_approved
                && advisory.get("approvalMarker").and_then(Value::as_str) != Some(APPROVAL_MARKER)
            {
                return Err(format!("context {key} advisory {id} is not approved"));
            }
        }
    }
    Ok(())
}

fn parse_approval(value: &Value) -> Result<ApprovalProjection, String> {
    let approval = value
        .get("approval")
        .and_then(Value::as_object)
        .ok_or_else(|| "context is missing approval metadata".to_owned())?;
    Ok(ApprovalProjection {
        marker: approval
            .get("marker")
            .and_then(Value::as_str)
            .ok_or_else(|| "approval marker is missing".to_owned())?
            .to_owned(),
        owner: approval
            .get("owner")
            .and_then(Value::as_str)
            .ok_or_else(|| "approval owner is missing".to_owned())?
            .to_owned(),
        expires_at: approval
            .get("expiresAt")
            .and_then(Value::as_str)
            .ok_or_else(|| "approval expiry is missing".to_owned())?
            .to_owned(),
        rationale: approval
            .get("rationale")
            .and_then(Value::as_str)
            .ok_or_else(|| "approval rationale is missing".to_owned())?
            .to_owned(),
    })
}

fn cargo_metadata(
    root: &Path,
    target: &str,
    features: &[String],
    default_features: bool,
) -> Result<Value, String> {
    let mut command = Command::new("cargo");
    command.current_dir(root).args([
        "metadata",
        "--locked",
        "--offline",
        "--format-version",
        "1",
        "--manifest-path",
        "Cargo.toml",
        "--filter-platform",
        target,
    ]);
    if !default_features {
        command.arg("--no-default-features");
    }
    if !features.is_empty() {
        command.arg("--features").arg(features.join(","));
    }
    let output = command
        .output()
        .map_err(|error| format!("start locked Cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "locked Cargo metadata failed for {target}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("parse Cargo metadata for {target}: {error}"))
}

fn package_is_proc_macro(package: &Value) -> bool {
    package
        .get("targets")
        .and_then(Value::as_array)
        .is_some_and(|targets| {
            targets.iter().any(|target| {
                target
                    .get("kind")
                    .and_then(Value::as_array)
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == "proc-macro"))
            })
        })
}

fn target_matches(expression: Option<&str>, target: &str) -> bool {
    let Some(expression) = expression else {
        return true;
    };
    if expression == target {
        return true;
    }
    let (arch, os, env) = if target.starts_with("x86_64-unknown-linux-gnu") {
        ("x86_64", "linux", "gnu")
    } else if target.starts_with("aarch64-unknown-linux-gnu") {
        ("aarch64", "linux", "gnu")
    } else if target.starts_with("x86_64-unknown-linux-musl") {
        ("x86_64", "linux", "musl")
    } else if target.starts_with("aarch64-unknown-linux-musl") {
        ("aarch64", "linux", "musl")
    } else {
        return false;
    };
    eval_cfg(expression, arch, os, env)
}

fn eval_cfg(expression: &str, arch: &str, os: &str, env: &str) -> bool {
    let expression = expression.trim();
    if expression == "cfg(unix)" {
        return true;
    }
    if expression == "cfg(windows)" {
        return false;
    }
    if let Some(inner) = expression
        .strip_prefix("cfg(")
        .and_then(|v| v.strip_suffix(')'))
    {
        return eval_cfg_expr(inner, arch, os, env);
    }
    false
}

fn eval_cfg_expr(expression: &str, arch: &str, _os: &str, env: &str) -> bool {
    let expression = expression.trim();
    if let Some(inner) = expression
        .strip_prefix("all(")
        .and_then(|v| v.strip_suffix(')'))
    {
        return split_cfg_args(inner)
            .into_iter()
            .all(|part| eval_cfg_expr(part, arch, _os, env));
    }
    if let Some(inner) = expression
        .strip_prefix("any(")
        .and_then(|v| v.strip_suffix(')'))
    {
        return split_cfg_args(inner)
            .into_iter()
            .any(|part| eval_cfg_expr(part, arch, _os, env));
    }
    if let Some(inner) = expression
        .strip_prefix("not(")
        .and_then(|v| v.strip_suffix(')'))
    {
        return !eval_cfg_expr(inner, arch, _os, env);
    }
    let normalized = expression.replace(' ', "");
    matches!(
        normalized.as_str(),
        "unix" | "target_os=\"linux\"" | "target_family=\"unix\""
    ) || normalized == format!("target_arch=\"{arch}\"")
        || normalized == format!("target_env=\"{env}\"")
}

fn split_cfg_args(input: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (index, character) in input.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                result.push(input[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < input.len() {
        result.push(input[start..].trim());
    }
    result
}

fn filtered_lock(root: &Path, lock_path: &str, closure: &Closure) -> Result<String, String> {
    let lock = fs::read_to_string(root.join(lock_path))
        .map_err(|error| format!("read Cargo.lock for audit projection: {error}"))?;
    let selected = closure
        .packages
        .iter()
        .map(|package| {
            (
                package.name.as_str(),
                package.version.as_str(),
                package.source.as_deref(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut header = Vec::new();
    let mut blocks = Vec::<Vec<String>>::new();
    let mut current = Vec::new();
    for line in lock.lines() {
        if line == "[[package]]" {
            if !current.is_empty() {
                blocks.push(current);
            }
            current = vec![line.to_owned()];
        } else if current.is_empty() {
            header.push(line.to_owned());
        } else {
            current.push(line.to_owned());
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    let mut selected_indices = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| {
            let name = lock_block_field(block, "name = \"").unwrap_or_default();
            let version = lock_block_field(block, "version = \"").unwrap_or_default();
            let source = lock_block_field(block, "source = \"");
            selected
                .contains(&(name.as_str(), version.as_str(), source.as_deref()))
                .then_some(index)
        })
        .collect::<BTreeSet<_>>();
    expand_audit_block_dependencies(&blocks, &mut selected_indices);
    let mut output = vec![
        "# Audit-only projection. Never use this file for Cargo resolution.".to_owned(),
        String::new(),
    ];
    output.extend(header);
    if !output.last().is_some_and(String::is_empty) {
        output.push(String::new());
    }
    for (index, block) in blocks.into_iter().enumerate() {
        if selected_indices.contains(&index) {
            output.extend(block);
        } else {
            continue;
        }
        output.push(String::new());
    }
    Ok(output.join("\n"))
}

fn lock_block_field(block: &[String], prefix: &str) -> Option<String> {
    block
        .iter()
        .find_map(|line| line.strip_prefix(prefix))
        .and_then(|line| line.strip_suffix('"'))
        .map(str::to_owned)
}

fn lock_block_dependencies(block: &[String]) -> Vec<(String, Option<String>)> {
    let mut dependencies = Vec::new();
    let mut in_dependencies = false;
    for line in block {
        if line == "dependencies = [" {
            in_dependencies = true;
            continue;
        }
        if !in_dependencies {
            continue;
        }
        if line == "]" {
            break;
        }
        let Some(dependency) = line
            .trim()
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix("\","))
        else {
            continue;
        };
        let mut fields = dependency.split_whitespace();
        let Some(name) = fields.next() else {
            continue;
        };
        dependencies.push((name.to_owned(), fields.next().map(str::to_owned)));
    }
    dependencies
}

fn expand_audit_block_dependencies(blocks: &[Vec<String>], selected_indices: &mut BTreeSet<usize>) {
    let mut changed = true;
    while changed {
        changed = false;
        let current = selected_indices.iter().copied().collect::<Vec<_>>();
        for index in current {
            for (dependency_name, dependency_version) in lock_block_dependencies(&blocks[index]) {
                for (candidate_index, candidate) in blocks.iter().enumerate() {
                    let candidate_name = lock_block_field(candidate, "name = \"");
                    let candidate_version = lock_block_field(candidate, "version = \"");
                    if candidate_name.as_deref() == Some(dependency_name.as_str())
                        && dependency_version
                            .as_deref()
                            .is_none_or(|version| candidate_version.as_deref() == Some(version))
                        && selected_indices.insert(candidate_index)
                    {
                        changed = true;
                    }
                }
            }
        }
    }
}

fn check_audit_lock(path: &Path, closure: &Closure) -> Result<(), String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("read audit-only lock {}: {error}", path.display()))?;
    for package in &closure.packages {
        let needle = format!("name = \"{}\"", package.name);
        if !text.contains(&needle) {
            return Err(format!(
                "audit-only lock {} is missing {}",
                path.display(),
                package.name
            ));
        }
    }
    Ok(())
}

fn reject_extra_files(directory: &Path) -> Result<(), String> {
    let allowed = [
        "production/closure.json",
        "production/metadata.json",
        "production/Cargo.lock",
        "policy/closure.json",
        "policy/metadata.json",
        "policy/Cargo.lock",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let mut found = BTreeSet::new();
    collect_relative_files(directory, directory, &mut found)?;
    if found != allowed {
        return Err(format!(
            "context output {} has unexpected files: expected {allowed:?}, found {found:?}",
            directory.display()
        ));
    }
    Ok(())
}

fn collect_relative_files(
    root: &Path,
    directory: &Path,
    found: &mut BTreeSet<String>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("read output directory {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("read output entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_relative_files(root, &path, found)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| format!("strip output path: {error}"))?
                .to_string_lossy()
                .replace('\\', "/");
            found.insert(relative);
        }
    }
    Ok(())
}

fn compare_file(path: &Path, expected: &str) -> Result<(), String> {
    let actual = fs::read_to_string(path)
        .map_err(|error| format!("read generated policy input {}: {error}", path.display()))?;
    if actual != expected {
        return Err(format!(
            "generated policy input is stale: {}",
            path.display()
        ));
    }
    Ok(())
}

fn write_json<T: Serialize>(
    path: &Path,
    value: &T,
    written: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize {}: {error}", path.display()))?
        + "\n";
    write_text(path, &text, written)
}

fn write_text(path: &Path, text: &str, written: &mut Vec<PathBuf>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    fs::write(path, text).map_err(|error| format!("write {}: {error}", path.display()))?;
    written.push(path.to_path_buf());
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    digest.update(bytes);
    Ok(format!("{:x}", digest.finalize()))
}

fn today_iso() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}")
}

use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_kinds_exclude_dev_edges() {
        let spec = ContextSpec {
            system: "x86_64-linux".to_owned(),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            name: "test".to_owned(),
            roots: vec!["root".to_owned()],
            features: Vec::new(),
            default_features: false,
            source_authority: "Cargo.lock".to_owned(),
            lock_path: PRODUCT_LOCK.to_owned(),
        };
        assert!(spec.production_kinds().contains("normal"));
        assert!(spec.production_kinds().contains("build"));
        assert!(spec.production_kinds().contains("proc-macro"));
        assert!(!spec.production_kinds().contains("dev"));
    }

    #[test]
    fn cfg_target_matching_handles_common_target_changes() {
        assert!(target_matches(
            Some("cfg(target_os = \"linux\")"),
            "x86_64-unknown-linux-gnu"
        ));
        assert!(target_matches(
            Some("cfg(target_arch = \"aarch64\")"),
            "aarch64-unknown-linux-musl"
        ));
        assert!(!target_matches(
            Some("cfg(target_arch = \"x86_64\")"),
            "aarch64-unknown-linux-gnu"
        ));
        assert!(target_matches(
            Some("cfg(any(unix, target_arch = \"wasm32\"))"),
            "x86_64-unknown-linux-gnu"
        ));
    }

    #[test]
    fn filtered_lock_omits_unselected_lock_only_packages() {
        let root = std::env::current_dir().expect("current directory");
        let closure = Closure {
            schema_version: 1,
            authority: "cargo-locked-metadata".to_owned(),
            context: "test".to_owned(),
            system: "x86_64-linux".to_owned(),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            roots: vec!["root".to_owned()],
            features: Vec::new(),
            default_features: false,
            source_authority: "Cargo.lock".to_owned(),
            lock_sha256: String::new(),
            packages: vec![PackageRecord {
                id: "path+file:///root#root".to_owned(),
                name: "root".to_owned(),
                version: "1.0.0".to_owned(),
                source: None,
                checksum: None,
                target: "x86_64-unknown-linux-gnu".to_owned(),
            }],
            edges: Vec::new(),
            approval: None,
        };
        let _ = filtered_lock(&root, PRODUCT_LOCK, &closure);
    }

    #[test]
    fn audit_projection_keeps_optional_lock_dependencies_parseable() {
        let blocks = vec![
            vec![
                "[[package]]".to_owned(),
                "name = \"root\"".to_owned(),
                "version = \"1.0.0\"".to_owned(),
                "dependencies = [".to_owned(),
                " \"optional-dependency\",".to_owned(),
                "]".to_owned(),
            ],
            vec![
                "[[package]]".to_owned(),
                "name = \"optional-dependency\"".to_owned(),
                "version = \"2.0.0\"".to_owned(),
            ],
            vec![
                "[[package]]".to_owned(),
                "name = \"unrelated\"".to_owned(),
                "version = \"3.0.0\"".to_owned(),
            ],
        ];
        let mut selected = BTreeSet::from([0]);
        expand_audit_block_dependencies(&blocks, &mut selected);
        assert_eq!(selected, BTreeSet::from([0, 1]));
    }
}
