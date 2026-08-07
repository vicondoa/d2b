#![allow(dead_code)]

//! The package-policy input boundary used by the Bazel migration.
//!
//! This module deliberately keeps Cargo's three views separate.  Metadata
//! owns identities and candidate edges, the product lock owns checksums, and
//! package-selected `cargo tree` output owns the selected root and features.
//! Joining those views here is more verbose than reading one convenient Cargo
//! output, but it prevents a workspace feature union or a dev-edge filter
//! from silently changing a policy context.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const PRODUCT_MANIFEST: &str = "packages/Cargo.toml";
pub const PRODUCT_LOCK: &str = "packages/Cargo.lock";
pub const POLICY_PREVIEW_ROOT: &str = ".scratch/bazel/policy-inputs";
pub const POLICY_DRIFT_REMEDIATION: &str = "\
D2B-BZLDRIFT-PACKAGE-POLICY: package-policy output is stale.
From the repository root, run: nix develop
Then run: cd packages
cargo xtask gen-package-policy-inputs
Review and commit the generated changes under packages/policy-inputs/.
Rerun cargo xtask gen-package-policy-inputs --check, then rerun the failed command.";

const GIT_ARCHIVE_SHA256: &str = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
const GIT_ARCHIVE_REV: &str = "072945b59fef21a2a8166460454280d543f48772";
const GIT_ARCHIVE_URL: &str = "https://github.com/vicondoa/wl-proxy.git";
const TREE_FORMAT: &str = "|{p}|{f}|";

pub trait CargoExecutor {
    fn run(&mut self, root: &Path, args: &[String]) -> Result<String, PolicyError>;
}

struct ProcessCargoExecutor;

impl CargoExecutor for ProcessCargoExecutor {
    fn run(&mut self, root: &Path, args: &[String]) -> Result<String, PolicyError> {
        let mut command = Command::new(
            args.first()
                .ok_or_else(|| PolicyError::Io("cargo".to_owned(), "empty command".to_owned()))?,
        );
        command
            .current_dir(root.join("packages"))
            .args(&args[1..])
            .env("CARGO_NET_OFFLINE", "true")
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped());
        let output = command.output().map_err(|error| {
            let _ = error;
            PolicyError::Io(
                "cargo".to_owned(),
                "locked offline Cargo command could not start".to_owned(),
            )
        })?;
        if !output.status.success() {
            return Err(PolicyError::Io(
                "cargo".to_owned(),
                "locked offline Cargo command failed".to_owned(),
            ));
        }
        String::from_utf8(output.stdout).map_err(|error| {
            let _ = error;
            PolicyError::Io(
                "cargo".to_owned(),
                "locked offline Cargo command returned non-UTF-8 output".to_owned(),
            )
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PolicyContext {
    BrokerProduction,
    GuestProduction,
}

impl PolicyContext {
    pub const ALL: [Self; 2] = [Self::BrokerProduction, Self::GuestProduction];

    pub const fn package(self) -> &'static str {
        match self {
            Self::BrokerProduction => "d2b-priv-broker",
            Self::GuestProduction => "d2b-guest-shell-runner",
        }
    }

    pub const fn feature(self) -> &'static str {
        match self {
            Self::BrokerProduction => "",
            Self::GuestProduction => "real-libshpool",
        }
    }

    pub const fn target_suffix(self) -> &'static str {
        match self {
            Self::BrokerProduction => "gnu",
            Self::GuestProduction => "musl",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::BrokerProduction => "broker-production",
            Self::GuestProduction => "guest-real-libshpool",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedContext {
    pub system: String,
    pub target: String,
    pub context: PolicyContext,
    pub package: String,
    pub features: Vec<String>,
    pub default_features: bool,
}

impl SelectedContext {
    pub fn for_system(system: &str, context: PolicyContext) -> Result<Self, PolicyError> {
        let target = native_target(system, context)?;
        let features = if context.feature().is_empty() {
            Vec::new()
        } else {
            vec![context.feature().to_owned()]
        };
        Ok(Self {
            system: system.to_owned(),
            target,
            context,
            package: context.package().to_owned(),
            features,
            default_features: false,
        })
    }

    pub fn production_edges(&self) -> &'static str {
        "normal,build"
    }

    pub fn policy_edges(&self) -> &'static str {
        "normal,build,dev"
    }

    pub fn preview_dir(&self) -> String {
        format!(
            "packages/policy-inputs/{}/{}/{}",
            self.system,
            self.target,
            self.context.name()
        )
    }
}

pub fn native_target(system: &str, context: PolicyContext) -> Result<String, PolicyError> {
    let prefix = match system {
        "x86_64-linux" => "x86_64",
        "aarch64-linux" => "aarch64",
        _ => return Err(PolicyError::WrongSystem(system.to_owned())),
    };
    Ok(format!(
        "{prefix}-unknown-linux-{}",
        context.target_suffix()
    ))
}

pub fn policy_contexts() -> Result<Vec<SelectedContext>, PolicyError> {
    ["x86_64-linux", "aarch64-linux"]
        .into_iter()
        .flat_map(|system| {
            PolicyContext::ALL
                .into_iter()
                .map(move |context| SelectedContext::for_system(system, context))
        })
        .collect()
}

pub fn metadata_command(target: &str) -> Vec<String> {
    vec![
        "cargo".to_owned(),
        "metadata".to_owned(),
        "--locked".to_owned(),
        "--offline".to_owned(),
        "--format-version".to_owned(),
        "1".to_owned(),
        "--filter-platform".to_owned(),
        target.to_owned(),
    ]
}

pub fn cargo_tree_command(context: &SelectedContext, edges: &str) -> Vec<String> {
    let features = context.features.join(",");
    vec![
        "cargo".to_owned(),
        "tree".to_owned(),
        "--locked".to_owned(),
        "--offline".to_owned(),
        "--manifest-path".to_owned(),
        "Cargo.toml".to_owned(),
        "-p".to_owned(),
        context.package.clone(),
        "--target".to_owned(),
        context.target.clone(),
        "--no-default-features".to_owned(),
        "--features".to_owned(),
        features,
        "--edges".to_owned(),
        edges.to_owned(),
        "--charset".to_owned(),
        "ascii".to_owned(),
        "--prefix".to_owned(),
        "depth".to_owned(),
        "--no-dedupe".to_owned(),
        "--format".to_owned(),
        TREE_FORMAT.to_owned(),
    ]
}

pub fn production_tree_command(context: &SelectedContext) -> Vec<String> {
    cargo_tree_command(context, context.production_edges())
}

pub fn policy_tree_command(context: &SelectedContext) -> Vec<String> {
    cargo_tree_command(context, context.policy_edges())
}

pub fn validate_tree_command(args: &[String]) -> Result<(), PolicyError> {
    let edges = value_after(args, "--edges")
        .ok_or_else(|| PolicyError::UnpinnedTreeArgument("--edges".to_owned()))?;
    if !matches!(edges, "normal,build" | "normal,build,dev") {
        return Err(PolicyError::InvalidEdgeKinds(edges.to_owned()));
    }
    validate_tree_command_for(args, edges)
}

pub fn validate_tree_command_for(args: &[String], expected_edges: &str) -> Result<(), PolicyError> {
    let required = [
        "--locked",
        "--offline",
        "--manifest-path",
        "-p",
        "--target",
        "--no-default-features",
        "--features",
        "--edges",
        "--charset",
        "--prefix",
        "--no-dedupe",
        "--format",
    ];
    for flag in required {
        if !args.iter().any(|arg| arg == flag) {
            return Err(PolicyError::UnpinnedTreeArgument(flag.to_owned()));
        }
    }
    if !args.iter().any(|arg| arg == "ascii") {
        return Err(PolicyError::UnpinnedTreeArgument("ascii".to_owned()));
    }
    if !args.iter().any(|arg| arg == "depth") {
        return Err(PolicyError::UnpinnedTreeArgument("depth".to_owned()));
    }
    if !args.iter().any(|arg| arg == TREE_FORMAT) {
        return Err(PolicyError::UnpinnedTreeFormat);
    }
    let edges = value_after(args, "--edges")
        .ok_or_else(|| PolicyError::UnpinnedTreeArgument("--edges".to_owned()))?;
    if edges != expected_edges {
        return Err(PolicyError::InvalidEdgeKinds(edges.to_owned()));
    }
    Ok(())
}

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PackageIdentity {
    pub name: String,
    pub version: String,
    pub source: Option<String>,
}

impl PackageIdentity {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        source: Option<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            source,
        }
    }

    fn key(&self) -> String {
        format!(
            "{} {} {}",
            self.name,
            self.version,
            self.source.as_deref().unwrap_or("path")
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateEdge {
    pub from: PackageIdentity,
    pub to_name: String,
    pub to_package_id: String,
    pub kind: String,
    pub cfg: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MetadataView {
    pub packages: Vec<PackageIdentity>,
    pub candidate_edges: Vec<CandidateEdge>,
    pub package_ids: BTreeMap<String, PackageIdentity>,
}

pub fn parse_metadata(document: &str) -> Result<MetadataView, PolicyError> {
    let value: Value = serde_json::from_str(document)
        .map_err(|error| PolicyError::MalformedMetadata(error.to_string()))?;
    let packages = value
        .get("packages")
        .and_then(Value::as_array)
        .ok_or(PolicyError::MetadataPackagesMissing)?;
    let mut view = MetadataView::default();
    for package in packages {
        let identity = package_identity(package)?;
        let id = package
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| identity.key());
        view.package_ids.insert(id, identity.clone());
        view.packages.push(identity);
    }

    let by_id = view.package_ids.clone();
    if let Some(nodes) = value
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(Value::as_array)
    {
        for node in nodes {
            let from_id = node.get("id").and_then(Value::as_str).ok_or_else(|| {
                PolicyError::MalformedMetadata("resolve node has no id".to_owned())
            })?;
            let from = by_id.get(from_id).ok_or_else(|| {
                PolicyError::IdentityMismatch(format!("resolve node {from_id} is not a package"))
            })?;
            if let Some(deps) = node.get("deps").and_then(Value::as_array) {
                for dependency in deps {
                    let to_id = dependency
                        .get("pkg")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let to_name = dependency
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let dep_kinds = dependency
                        .get("dep_kinds")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            PolicyError::MalformedMetadata(format!(
                                "dependency {to_name} has no dep_kinds"
                            ))
                        })?;
                    for dep_kind in dep_kinds {
                        let kind = dep_kind
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("normal")
                            .to_owned();
                        let cfg = dep_kind
                            .get("target")
                            .and_then(|target| target.as_str().map(str::to_owned));
                        view.candidate_edges.push(CandidateEdge {
                            from: from.clone(),
                            to_name: to_name.clone(),
                            to_package_id: to_id.clone(),
                            kind,
                            cfg,
                        });
                    }
                }
            }
        }
    }
    view.packages.sort();
    view.candidate_edges.sort_by(|left, right| {
        (
            &left.from,
            &left.to_name,
            &left.kind,
            &left.cfg,
            &left.to_package_id,
        )
            .cmp(&(
                &right.from,
                &right.to_name,
                &right.kind,
                &right.cfg,
                &right.to_package_id,
            ))
    });
    Ok(view)
}

fn package_identity(package: &Value) -> Result<PackageIdentity, PolicyError> {
    let name = package
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| PolicyError::MalformedMetadata("package has no name".to_owned()))?;
    let version = package
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| PolicyError::MalformedMetadata(format!("{name} has no version")))?;
    let source = package
        .get("source")
        .and_then(|source| source.as_str().map(str::to_owned));
    Ok(PackageIdentity::new(name, version, source))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockPackage {
    pub identity: PackageIdentity,
    pub checksum: Option<String>,
    pub dependencies: Vec<String>,
}

pub fn parse_product_lock(lock: &str) -> Result<Vec<LockPackage>, PolicyError> {
    let mut packages = Vec::new();
    for block in lock.split("[[package]]").skip(1) {
        let name = toml_value(block, "name")
            .ok_or_else(|| PolicyError::MalformedLock("package has no name".to_owned()))?;
        let version = toml_value(block, "version")
            .ok_or_else(|| PolicyError::MalformedLock(format!("{name} has no version")))?;
        let source = toml_value(block, "source");
        let checksum = toml_value(block, "checksum");
        let dependencies = toml_array(block, "dependencies");
        packages.push(LockPackage {
            identity: PackageIdentity::new(name, version, source),
            checksum,
            dependencies,
        });
    }
    if packages.is_empty() {
        return Err(PolicyError::EmptyLock);
    }
    packages.sort_by(|left, right| left.identity.cmp(&right.identity));
    Ok(packages)
}

fn toml_value(block: &str, key: &str) -> Option<String> {
    block.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        if name.trim() != key {
            return None;
        }
        let value = value.trim();
        let value = value.strip_prefix('"')?;
        Some(value[..value.find('"')?].to_owned())
    })
}

fn toml_array(block: &str, key: &str) -> Vec<String> {
    let Some(line) = block
        .lines()
        .find(|line| line.trim_start().starts_with(key))
    else {
        return Vec::new();
    };
    line.split('"')
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, value)| value.to_owned())
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitArchivePin {
    pub url: String,
    pub rev: String,
    pub sha256: String,
}

pub fn committed_git_archive_pins() -> Vec<GitArchivePin> {
    vec![GitArchivePin {
        url: GIT_ARCHIVE_URL.to_owned(),
        rev: GIT_ARCHIVE_REV.to_owned(),
        sha256: GIT_ARCHIVE_SHA256.to_owned(),
    }]
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCensus {
    pub identities: BTreeSet<PackageIdentity>,
    pub checksums: BTreeMap<PackageIdentity, String>,
    pub digest: String,
}

pub fn selected_source_census(
    selected: &BTreeSet<PackageIdentity>,
    lock: &[LockPackage],
    source_paths: &BTreeSet<String>,
) -> Result<SourceCensus, PolicyError> {
    if selected.is_empty() {
        return Err(PolicyError::EmptySourceCensus);
    }
    let lock_identities = lock
        .iter()
        .map(|package| package.identity.clone())
        .collect::<BTreeSet<_>>();
    let missing = selected
        .difference(&lock_identities)
        .map(PackageIdentity::key)
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(PolicyError::MetadataLockMismatch(missing.join(", ")));
    }
    let extra = lock_identities
        .difference(selected)
        .map(PackageIdentity::key)
        .collect::<Vec<_>>();
    if !extra.is_empty() {
        return Err(PolicyError::ExtraSource(extra.join(", ")));
    }

    let mut checksums = BTreeMap::new();
    for package in lock {
        if !selected.contains(&package.identity) {
            continue;
        }
        if let Some(source) = package.identity.source.as_deref() {
            if source.starts_with("registry+") {
                let checksum = package
                    .checksum
                    .clone()
                    .ok_or_else(|| PolicyError::ChecksumMissing(package.identity.key()))?;
                checksums.insert(package.identity.clone(), checksum);
            } else if source.starts_with("git+") {
                verify_git_archive_pin(&package.identity, &committed_git_archive_pins())?;
                let pins = committed_git_archive_pins();
                let pin = pins
                    .iter()
                    .find(|pin| source.contains(&format!("?rev={}", pin.rev)))
                    .ok_or_else(|| PolicyError::GitArchivePinMissing(package.identity.key()))?;
                checksums.insert(package.identity.clone(), pin.sha256.clone());
            } else {
                return Err(PolicyError::ChecksumMissing(package.identity.key()));
            }
        }
        let source = package.identity.source.as_deref().unwrap_or("path");
        if !source_paths.is_empty() && !source_paths.contains(source) {
            return Err(PolicyError::SourceMissing(source.to_owned()));
        }
    }
    let mut canonical = String::new();
    for identity in selected {
        canonical.push_str(&identity.key());
        canonical.push('\n');
        if let Some(checksum) = checksums.get(identity) {
            canonical.push_str(checksum);
            canonical.push('\n');
        }
    }
    let digest = hex_digest(canonical.as_bytes());
    Ok(SourceCensus {
        identities: selected.clone(),
        checksums,
        digest,
    })
}

pub fn verify_git_archive_pin(
    identity: &PackageIdentity,
    pins: &[GitArchivePin],
) -> Result<(), PolicyError> {
    let Some(source) = identity.source.as_deref() else {
        return Ok(());
    };
    if !source.starts_with("git+") {
        return Ok(());
    }
    let url = source
        .strip_prefix("git+")
        .and_then(|value| value.split_once("?rev="))
        .map(|(url, _)| url)
        .ok_or_else(|| PolicyError::GitArchivePinMissing(identity.key()))?;
    let rev = source
        .split_once("?rev=")
        .and_then(|(_, value)| value.split('#').next())
        .unwrap_or_default();
    if !pins.iter().any(|pin| pin.url == url && pin.rev == rev) {
        return Err(PolicyError::GitArchivePinMissing(identity.key()));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeRow {
    pub depth: usize,
    pub package: String,
    pub features: BTreeSet<String>,
}

impl TreeRow {
    pub fn package_name(&self) -> &str {
        self.package
            .split_once(" v")
            .map(|(name, _)| name)
            .unwrap_or(&self.package)
            .trim()
    }

    pub fn package_version(&self) -> Option<&str> {
        self.package
            .split_once(" v")
            .and_then(|(_, version)| version.split_whitespace().next())
    }
}

pub fn parse_tree_rows(output: &str) -> Result<Vec<TreeRow>, PolicyError> {
    let mut rows = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split('|');
        let depth = fields
            .next()
            .ok_or(PolicyError::MalformedTree)?
            .trim()
            .parse::<usize>()
            .map_err(|_| PolicyError::MalformedTree)?;
        let package = fields.next().ok_or(PolicyError::MalformedTree)?.trim();
        let features = fields
            .next()
            .ok_or(PolicyError::MalformedTree)?
            .split(',')
            .filter(|feature| !feature.trim().is_empty())
            .map(|feature| feature.trim().to_owned())
            .collect();
        if package.is_empty() {
            return Err(PolicyError::MalformedTree);
        }
        rows.push(TreeRow {
            depth,
            package: package.to_owned(),
            features,
        });
    }
    if rows.is_empty() {
        return Err(PolicyError::EmptyTree);
    }
    Ok(rows)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedGraph {
    pub edge_kinds: String,
    pub rows: Vec<TreeRow>,
    pub identities: BTreeSet<PackageIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedContextOracle {
    pub context: SelectedContext,
    pub production: SelectedGraph,
    pub policy: SelectedGraph,
    pub source_census: SourceCensus,
}

pub fn selected_context_oracle(
    context: &SelectedContext,
    metadata: &MetadataView,
    production_output: &str,
    policy_output: &str,
    lock: &[LockPackage],
    source_paths: &BTreeSet<String>,
) -> Result<SelectedContextOracle, PolicyError> {
    validate_context(context)?;
    let production = selected_graph(
        context,
        metadata,
        production_output,
        context.production_edges(),
    )?;
    let policy = selected_graph(context, metadata, policy_output, context.policy_edges())?;
    let production_root = production
        .identities
        .iter()
        .find(|identity| identity.name == context.package)
        .ok_or_else(|| PolicyError::WrongRoot(context.package.clone()))?;
    let policy_root = policy
        .identities
        .iter()
        .find(|identity| identity.name == context.package)
        .ok_or_else(|| PolicyError::WrongRoot(context.package.clone()))?;
    validate_candidate_edge_join_with_root(
        metadata,
        &production.identities,
        context.production_edges(),
        production_root,
    )?;
    validate_candidate_edge_join_with_root(
        metadata,
        &policy.identities,
        context.policy_edges(),
        policy_root,
    )?;
    if production.rows.len() > policy.rows.len() {
        return Err(PolicyError::PolicyClosureShrank);
    }

    let selected = policy.identities.clone();
    let filtered_lock = lock
        .iter()
        .filter(|package| selected.contains(&package.identity))
        .cloned()
        .collect::<Vec<_>>();
    let source_census = selected_source_census(&selected, &filtered_lock, source_paths)?;
    for identity in &selected {
        verify_git_archive_pin(identity, &committed_git_archive_pins())?;
    }
    Ok(SelectedContextOracle {
        context: context.clone(),
        production,
        policy,
        source_census,
    })
}

pub fn validate_candidate_edge_join(
    metadata: &MetadataView,
    selected: &BTreeSet<PackageIdentity>,
    edge_kinds: &str,
) -> Result<(), PolicyError> {
    let roots = selected
        .iter()
        .filter(|identity| {
            !metadata.candidate_edges.iter().any(|edge| {
                selected.contains(&edge.from)
                    && metadata
                        .package_ids
                        .get(&edge.to_package_id)
                        .is_some_and(|target| target == *identity)
            })
        })
        .collect::<Vec<_>>();
    if roots.len() != 1 {
        return Err(PolicyError::RootCount(roots.len()));
    }
    validate_candidate_edge_join_with_root(metadata, selected, edge_kinds, roots[0])
}

fn validate_candidate_edge_join_with_root(
    metadata: &MetadataView,
    selected: &BTreeSet<PackageIdentity>,
    edge_kinds: &str,
    root: &PackageIdentity,
) -> Result<(), PolicyError> {
    let allowed = edge_kinds.split(',').collect::<BTreeSet<_>>();
    for identity in selected {
        if identity == root {
            continue;
        }
        let matching = metadata.candidate_edges.iter().any(|edge| {
            selected.contains(&edge.from)
                && allowed.contains(edge.kind.as_str())
                && metadata
                    .package_ids
                    .get(&edge.to_package_id)
                    .is_some_and(|target| target == identity)
        });
        if !matching {
            return Err(PolicyError::CandidateEdgeMissing(identity.key()));
        }
    }
    Ok(())
}

pub fn build_selected_context(
    context: &SelectedContext,
    metadata_json: &str,
    production_output: &str,
    policy_output: &str,
    lock_text: &str,
) -> Result<SelectedContextOracle, PolicyError> {
    let metadata = parse_metadata(metadata_json)?;
    let lock = parse_product_lock(lock_text)?;
    selected_context_oracle(
        context,
        &metadata,
        production_output,
        policy_output,
        &lock,
        &BTreeSet::new(),
    )
}

fn validate_context(context: &SelectedContext) -> Result<(), PolicyError> {
    let expected = SelectedContext::for_system(&context.system, context.context)?;
    if expected.target != context.target
        || expected.package != context.package
        || expected.features != context.features
        || context.default_features
    {
        return Err(PolicyError::ContextMismatch);
    }
    Ok(())
}

fn selected_graph(
    context: &SelectedContext,
    metadata: &MetadataView,
    output: &str,
    edge_kinds: &str,
) -> Result<SelectedGraph, PolicyError> {
    let rows = parse_tree_rows(output)?;
    let roots = rows.iter().filter(|row| row.depth == 0).collect::<Vec<_>>();
    if roots.len() != 1 {
        return Err(PolicyError::RootCount(roots.len()));
    }
    if roots[0].package_name() != context.package {
        return Err(PolicyError::WrongRoot(roots[0].package.clone()));
    }
    let mut identities = BTreeSet::new();
    for row in &rows {
        let candidates = metadata
            .packages
            .iter()
            .filter(|identity| {
                identity.name == row.package_name()
                    && row
                        .package_version()
                        .is_none_or(|version| identity.version == version)
            })
            .cloned()
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(PolicyError::TreeIdentityMismatch(row.package.clone()));
        }
        let identity = candidates.into_iter().next().expect("length checked");
        if row.depth == 0 && identity.name != context.package {
            return Err(PolicyError::WrongRoot(identity.name));
        }
        identities.insert(identity);
    }
    Ok(SelectedGraph {
        edge_kinds: edge_kinds.to_owned(),
        rows,
        identities,
    })
}

pub fn feature_union_refusal(
    selected_rows: &[TreeRow],
    workspace_feature_union: &BTreeSet<String>,
    allowed: &BTreeSet<String>,
) -> Result<(), PolicyError> {
    let leaked = selected_rows
        .iter()
        .flat_map(|row| row.features.iter())
        .filter(|feature| workspace_feature_union.contains(*feature) && !allowed.contains(*feature))
        .cloned()
        .collect::<BTreeSet<_>>();
    if leaked.is_empty() {
        Ok(())
    } else {
        Err(PolicyError::FeatureUnionLeak(
            leaked.into_iter().collect::<Vec<_>>().join(", "),
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyInput {
    pub context: SelectedContext,
    pub variant: &'static str,
    pub edge_kinds: String,
    pub root: String,
    pub source_census_digest: String,
    pub identities: Vec<PackageIdentity>,
}

impl PolicyInput {
    pub fn as_json(&self) -> Value {
        json!({
            "system": self.context.system,
            "target": self.context.target,
            "root": self.root,
            "package": self.context.package,
            "features": self.context.features,
            "defaultFeatures": self.context.default_features,
            "variant": self.variant,
            "edgeKinds": self.edge_kinds,
            "sourceCensusSha256": self.source_census_digest,
            "identities": self.identities.iter().map(|identity| json!({
                "name": identity.name,
                "version": identity.version,
                "source": identity.source,
            })).collect::<Vec<_>>(),
        })
    }
}

pub fn package_policy_preview(root: &Path) -> Result<BTreeMap<String, String>, PolicyError> {
    let mut executor = ProcessCargoExecutor;
    package_policy_preview_with_executor(root, &mut executor)
}

pub fn package_policy_preview_with_executor(
    root: &Path,
    executor: &mut dyn CargoExecutor,
) -> Result<BTreeMap<String, String>, PolicyError> {
    let lock = fs::read_to_string(root.join(PRODUCT_LOCK))
        .map_err(|error| PolicyError::Io(PRODUCT_LOCK.to_owned(), error.to_string()))?;
    let lock_packages = parse_product_lock(&lock)?;
    let mut outputs = BTreeMap::new();
    for context in policy_contexts()? {
        let metadata_json = executor.run(root, &metadata_command(&context.target))?;
        let production_json = executor.run(root, &production_tree_command(&context))?;
        let policy_json = executor.run(root, &policy_tree_command(&context))?;
        let metadata = parse_metadata(&metadata_json)?;
        let oracle = selected_context_oracle(
            &context,
            &metadata,
            &production_json,
            &policy_json,
            &lock_packages,
            &BTreeSet::new(),
        )?;
        let selected = oracle.policy.identities.clone();
        let filtered_lock = lock_packages
            .iter()
            .filter(|package| selected.contains(&package.identity))
            .cloned()
            .collect::<Vec<_>>();
        let census = selected_source_census(&selected, &filtered_lock, &BTreeSet::new())?;
        let production_input = PolicyInput {
            context: context.clone(),
            variant: "production",
            edge_kinds: context.production_edges().to_owned(),
            root: context.package.clone(),
            source_census_digest: census.digest.clone(),
            identities: oracle.production.identities.iter().cloned().collect(),
        };
        let policy_input = PolicyInput {
            context: context.clone(),
            variant: "policy",
            edge_kinds: context.policy_edges().to_owned(),
            root: context.package.clone(),
            source_census_digest: census.digest.clone(),
            identities: oracle.policy.identities.iter().cloned().collect(),
        };
        insert_policy_output(
            &mut outputs,
            &context.preview_dir(),
            "production/closure.json",
            serde_json::to_string_pretty(&production_input.as_json())
                .map_err(|error| PolicyError::Serialization(error.to_string()))?
                + "\n",
        );
        insert_policy_output(
            &mut outputs,
            &context.preview_dir(),
            "production/Cargo.lock",
            filtered_lock_text(&lock, &oracle.production.identities),
        );
        insert_policy_output(
            &mut outputs,
            &context.preview_dir(),
            "policy/metadata.json",
            serde_json::to_string_pretty(&policy_input.as_json())
                .map_err(|error| PolicyError::Serialization(error.to_string()))?
                + "\n",
        );
        insert_policy_output(
            &mut outputs,
            &context.preview_dir(),
            "policy/Cargo.lock",
            filtered_lock_text(&lock, &oracle.policy.identities),
        );
    }
    Ok(outputs)
}

fn insert_policy_output(
    outputs: &mut BTreeMap<String, String>,
    context_dir: &str,
    suffix: &str,
    contents: String,
) {
    outputs.insert(format!("{context_dir}/{suffix}"), contents);
}

fn filtered_lock_text(lock: &str, identities: &BTreeSet<PackageIdentity>) -> String {
    let mut output = String::from(
        "# This file is a generated policy-input preview. It is not a Cargo workspace lock authority.\n\n",
    );
    for block in lock.split("[[package]]").skip(1) {
        let Some(name) = toml_value(block, "name") else {
            continue;
        };
        let Some(version) = toml_value(block, "version") else {
            continue;
        };
        let identity = PackageIdentity::new(name, version, toml_value(block, "source"));
        if identities.contains(&identity) {
            output.push_str("[[package]]");
            output.push_str(block.trim_end());
            output.push_str("\n\n");
        }
    }
    output
}

pub fn gen_package_policy_inputs(args: &[String]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err("usage: gen-package-policy-inputs [--check]".into()),
    };
    let root = repo_root()?;
    let outputs = package_policy_preview(&root)?;
    if check {
        check_policy_outputs(&root, &outputs)?;
    }
    let preview_root = root.join(POLICY_PREVIEW_ROOT);
    if !check {
        for (relative, contents) in &outputs {
            let relative_preview = Path::new(&relative)
                .strip_prefix("packages/policy-inputs")
                .unwrap_or_else(|_| Path::new(&relative));
            let path = preview_root.join(relative_preview);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, contents)?;
        }
    }
    Ok(outputs
        .keys()
        .map(|relative| PathBuf::from(POLICY_PREVIEW_ROOT).join(relative))
        .collect())
}

pub fn check_policy_outputs(
    root: &Path,
    expected: &BTreeMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    let final_root = root.join("packages/policy-inputs");
    if !final_root.exists() {
        return Ok(());
    }
    for (relative, contents) in expected {
        let relative = Path::new(relative)
            .strip_prefix("packages/policy-inputs")
            .map_err(|_| "policy output escaped its owned directory")?;
        let path = final_root.join(relative);
        let actual = match fs::read_to_string(&path) {
            Ok(actual) => actual,
            Err(_) => return Err(POLICY_DRIFT_REMEDIATION.into()),
        };
        if actual != *contents {
            return Err(POLICY_DRIFT_REMEDIATION.into());
        }
    }
    Ok(())
}

pub fn package_policy_drift_message() -> &'static str {
    POLICY_DRIFT_REMEDIATION
}

fn repo_root() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(root) = env::var_os("D2B_BAZEL_WORKTREE") {
        return fs::canonicalize(root)
            .map_err(|error| format!("cannot canonicalize D2B_BAZEL_WORKTREE: {error}").into());
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot locate repository root".into())
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyError {
    WrongSystem(String),
    ContextMismatch,
    UnpinnedTreeArgument(String),
    UnpinnedTreeFormat,
    InvalidEdgeKinds(String),
    MalformedMetadata(String),
    MetadataPackagesMissing,
    IdentityMismatch(String),
    MalformedLock(String),
    EmptyLock,
    EmptySourceCensus,
    MetadataLockMismatch(String),
    ExtraSource(String),
    ChecksumMissing(String),
    SourceMissing(String),
    GitArchivePinMissing(String),
    MalformedTree,
    EmptyTree,
    RootCount(usize),
    WrongRoot(String),
    TreeIdentityMismatch(String),
    PolicyClosureShrank,
    FeatureUnionLeak(String),
    CandidateEdgeMissing(String),
    Io(String, String),
    Serialization(String),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongSystem(system) => write!(formatter, "wrong native system: {system}"),
            Self::ContextMismatch => {
                formatter.write_str("selected Cargo context does not match its closed selector")
            }
            Self::UnpinnedTreeArgument(argument) => {
                write!(formatter, "cargo tree argument is not pinned: {argument}")
            }
            Self::UnpinnedTreeFormat => formatter
                .write_str("cargo tree format is not the repository-pinned delimited format"),
            Self::InvalidEdgeKinds(edges) => {
                write!(formatter, "cargo tree edge kinds are not pinned: {edges}")
            }
            Self::MalformedMetadata(message) => {
                write!(formatter, "malformed Cargo metadata: {message}")
            }
            Self::MetadataPackagesMissing => {
                formatter.write_str("Cargo metadata has no packages array")
            }
            Self::IdentityMismatch(message) => {
                write!(formatter, "metadata identity mismatch: {message}")
            }
            Self::MalformedLock(message) => write!(formatter, "malformed Cargo.lock: {message}"),
            Self::EmptyLock => formatter.write_str("Cargo.lock has no package records"),
            Self::EmptySourceCensus => formatter.write_str("selected source census is empty"),
            Self::MetadataLockMismatch(message) => {
                write!(formatter, "metadata and lock identities differ: {message}")
            }
            Self::ExtraSource(message) => write!(
                formatter,
                "selected source census has extra identities: {message}"
            ),
            Self::ChecksumMissing(message) => {
                write!(formatter, "lock checksum is missing: {message}")
            }
            Self::SourceMissing(message) => write!(
                formatter,
                "selected source is missing or unreadable: {message}"
            ),
            Self::GitArchivePinMissing(message) => {
                write!(formatter, "committed git archive pin is missing: {message}")
            }
            Self::MalformedTree => formatter.write_str("cargo tree output is malformed"),
            Self::EmptyTree => formatter.write_str("cargo tree closure is empty"),
            Self::RootCount(count) => write!(formatter, "selected cargo tree has {count} roots"),
            Self::WrongRoot(root) => write!(formatter, "selected cargo tree root is {root}"),
            Self::TreeIdentityMismatch(package) => write!(
                formatter,
                "cargo tree identity is not in metadata: {package}"
            ),
            Self::PolicyClosureShrank => formatter
                .write_str("dev-inclusive policy closure is smaller than production closure"),
            Self::FeatureUnionLeak(features) => write!(
                formatter,
                "workspace feature union leaked into selected context: {features}"
            ),
            Self::CandidateEdgeMissing(identity) => {
                write!(
                    formatter,
                    "selected identity has no metadata candidate edge: {identity}"
                )
            }
            Self::Io(path, message) => {
                write!(formatter, "package-policy input {path} failed: {message}")
            }
            Self::Serialization(message) => write!(
                formatter,
                "cannot serialize package-policy input: {message}"
            ),
        }
    }
}

impl Error for PolicyError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> SelectedContext {
        SelectedContext::for_system("x86_64-linux", PolicyContext::BrokerProduction)
            .expect("known context")
    }

    #[test]
    fn context_matrix_has_only_product_selected_contexts() {
        let contexts = policy_contexts().expect("matrix");
        assert_eq!(contexts.len(), 4);
        assert!(contexts.iter().all(|context| context.package != "d2b"));
        assert_eq!(contexts[0].target, "x86_64-unknown-linux-gnu");
        assert_eq!(contexts[1].target, "x86_64-unknown-linux-musl");
    }

    #[test]
    fn tree_command_has_every_parser_boundary() {
        let command = production_tree_command(&context());
        validate_tree_command(&command).expect("pinned command");
        assert!(
            command
                .windows(2)
                .any(|pair| pair == ["--edges", "normal,build"])
        );
        assert!(
            command
                .windows(2)
                .any(|pair| pair == ["--format", TREE_FORMAT])
        );
    }

    #[test]
    fn unpinned_tree_format_and_post_filtered_dev_edges_refuse() {
        let mut command = policy_tree_command(&context());
        let format_index = command.iter().position(|arg| arg == TREE_FORMAT).unwrap();
        command[format_index] = "{p}".to_owned();
        assert!(matches!(
            validate_tree_command(&command),
            Err(PolicyError::UnpinnedTreeFormat)
        ));

        let mut command = production_tree_command(&context());
        let edge_index = command
            .iter()
            .position(|arg| arg == "normal,build")
            .unwrap();
        command[edge_index] = "normal".to_owned();
        assert!(matches!(
            validate_tree_command(&command),
            Err(PolicyError::InvalidEdgeKinds(_))
        ));
    }

    #[test]
    fn source_census_requires_lock_checksums() {
        let identity = PackageIdentity::new(
            "example",
            "1.0.0",
            Some("registry+https://example.invalid/index".to_owned()),
        );
        let selected = BTreeSet::from([identity.clone()]);
        let lock = vec![LockPackage {
            identity,
            checksum: None,
            dependencies: Vec::new(),
        }];
        assert!(matches!(
            selected_source_census(&selected, &lock, &BTreeSet::new()),
            Err(PolicyError::ChecksumMissing(_))
        ));
    }

    #[test]
    fn feature_union_canary_is_not_accepted_by_selected_context() {
        let row = TreeRow {
            depth: 1,
            package: "shared".to_owned(),
            features: BTreeSet::from(["canary".to_owned()]),
        };
        let result = feature_union_refusal(
            &[row],
            &BTreeSet::from(["canary".to_owned()]),
            &BTreeSet::new(),
        );
        assert!(matches!(result, Err(PolicyError::FeatureUnionLeak(_))));
    }
}
