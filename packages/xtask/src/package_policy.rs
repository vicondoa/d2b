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
    ffi::OsStr,
    fmt, fs,
    io::{Read, Write},
    ops::Range,
    os::fd::AsFd,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const PRODUCT_LOCK: &str = "packages/Cargo.lock";
pub const POLICY_PREVIEW_ROOT: &str = ".scratch/bazel/policy-inputs";
const NATIVE_POLICY_CHECK_MANIFEST: &str =
    include_str!("../../../tests/golden/native-policy-check-manifest.json");
pub const POLICY_DRIFT_REMEDIATION: &str = "\
D2B-BZLDRIFT-PACKAGE-POLICY: package-policy output is stale.
From the repository root, run: nix develop
Then run: cd packages
Review the scratch preview, then run cargo xtask gen-package-policy-inputs --install.
Review and commit the exact repository-relative generated paths returned by the install command.
Rerun cargo xtask gen-package-policy-inputs --check, then rerun the failed command.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PackagePolicyMode {
    Preview,
    Install,
    Check,
}

fn parse_package_policy_mode(args: &[String]) -> Result<PackagePolicyMode, Box<dyn Error>> {
    match args {
        [] => Ok(PackagePolicyMode::Preview),
        [flag] if flag == "--install" => Ok(PackagePolicyMode::Install),
        [flag] if flag == "--check" => Ok(PackagePolicyMode::Check),
        _ => Err("usage: gen-package-policy-inputs [--check|--install]".into()),
    }
}

const GIT_ARCHIVE_SHA256: &str = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
const GIT_ARCHIVE_REV: &str = "072945b59fef21a2a8166460454280d543f48772";
const GIT_ARCHIVE_URL: &str = "https://github.com/vicondoa/wl-proxy.git";
const TREE_FORMAT: &str = "|{p}|{f}|";

pub trait CargoExecutor {
    fn run(&mut self, root: &Path, args: &[String]) -> Result<String, PolicyError>;
}

struct ProcessCargoExecutor;

const MAX_CHILD_DIAGNOSTIC_BYTES: usize = 768;

fn cargo_command_label(args: &[String]) -> String {
    let Some(command) = args.get(1) else {
        return "cargo <unknown> (hub=product)".to_owned();
    };
    let package = args
        .windows(2)
        .find(|pair| pair[0] == "-p" || pair[0] == "--package")
        .map(|pair| pair[1].as_str())
        .unwrap_or("<workspace>");
    let target = args
        .windows(2)
        .find(|pair| pair[0] == "--target")
        .map(|pair| pair[1].as_str())
        .unwrap_or("<native>");
    format!("cargo {command} package={package} target={target} hub=product")
}

fn bounded_redacted_diagnostic(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut rendered = String::new();
    for token in text.split_whitespace() {
        let token = if token.starts_with('/')
            || token.contains("/nix/store/")
            || token.contains("file://")
            || (token.contains('/')
                && !token.starts_with("http://")
                && !token.starts_with("https://")
                && !token.starts_with("registry+")
                && !token.starts_with("git+"))
        {
            "<path>"
        } else {
            token
        };
        if !rendered.is_empty() {
            rendered.push(' ');
        }
        rendered.push_str(token);
        if rendered.len() >= MAX_CHILD_DIAGNOSTIC_BYTES {
            rendered.truncate(MAX_CHILD_DIAGNOSTIC_BYTES);
            rendered.push_str("...[truncated]");
            break;
        }
    }

    if rendered.is_empty() {
        "<no child diagnostic>".to_owned()
    } else {
        rendered
    }
}

fn redacted_policy_text(value: &str) -> String {
    bounded_redacted_diagnostic(value.as_bytes())
}

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
        let command_label = cargo_command_label(args);
        let output = command.output().map_err(|_| {
            PolicyError::Io(
                "cargo".to_owned(),
                "locked offline Cargo command could not start".to_owned(),
            )
        })?;
        if !output.status.success() {
            return Err(PolicyError::CommandFailed {
                command: command_label,
                status: output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_owned()),
                diagnostic: bounded_redacted_diagnostic(&output.stderr),
            });
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

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "broker-production" => Some(Self::BrokerProduction),
            "guest-real-libshpool" => Some(Self::GuestProduction),
            _ => None,
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
    let manifest: Value = serde_json::from_str(NATIVE_POLICY_CHECK_MANIFEST)
        .map_err(|_| PolicyError::ContextManifestInvalid)?;
    if manifest.get("schemaVersion") != Some(&json!(1)) {
        return Err(PolicyError::ContextManifestInvalid);
    }
    let contexts = manifest
        .get("contexts")
        .and_then(Value::as_array)
        .ok_or(PolicyError::ContextManifestInvalid)?;
    let mut selected = Vec::with_capacity(contexts.len());
    let mut ids = BTreeSet::new();
    for record in contexts {
        let system = record
            .get("system")
            .and_then(Value::as_str)
            .ok_or(PolicyError::ContextManifestInvalid)?;
        let target = record
            .get("target")
            .and_then(Value::as_str)
            .ok_or(PolicyError::ContextManifestInvalid)?;
        let name = record
            .get("context")
            .and_then(Value::as_str)
            .ok_or(PolicyError::ContextManifestInvalid)?;
        let context = PolicyContext::from_name(name).ok_or(PolicyError::ContextManifestInvalid)?;
        let id = record
            .get("id")
            .and_then(Value::as_str)
            .ok_or(PolicyError::ContextManifestInvalid)?;
        let expected_policy_input = format!("packages/policy-inputs/{system}/{target}/{name}");
        if !ids.insert(id.to_owned())
            || id != format!("{system}/{target}/{name}")
            || record.get("package").and_then(Value::as_str) != Some(context.package())
            || record.get("policyInput").and_then(Value::as_str)
                != Some(expected_policy_input.as_str())
            || record.get("defaultFeatures") != Some(&Value::Bool(false))
            || record.get("productionEdgeKinds").and_then(Value::as_str) != Some("normal,build")
            || record.get("policyEdgeKinds").and_then(Value::as_str) != Some("normal,build,dev")
        {
            return Err(PolicyError::ContextManifestInvalid);
        }
        let features = record
            .get("features")
            .and_then(Value::as_array)
            .ok_or(PolicyError::ContextManifestInvalid)?
            .iter()
            .map(|feature| {
                feature
                    .as_str()
                    .map(str::to_owned)
                    .ok_or(PolicyError::ContextManifestInvalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let expected = SelectedContext::for_system(system, context)?;
        if expected.target != target || expected.features != features {
            return Err(PolicyError::ContextManifestInvalid);
        }
        selected.push(expected);
    }
    if selected.len() != 4 {
        return Err(PolicyError::ContextManifestInvalid);
    }
    let checks = manifest
        .get("nativeChecks")
        .and_then(Value::as_array)
        .ok_or(PolicyError::ContextManifestInvalid)?;
    let expected_checks = [
        "broker-production-dependency-policy",
        "guest-shell-runner-static-dependency-policy",
        "broker-production-package-policy",
        "guest-real-libshpool-package-policy",
        "broker-host-artifact-contract",
        "guest-static-elf",
    ];
    let actual_checks = checks
        .iter()
        .map(|check| check.as_str().ok_or(PolicyError::ContextManifestInvalid))
        .collect::<Result<Vec<_>, _>>()?;
    if actual_checks != expected_checks {
        return Err(PolicyError::ContextManifestInvalid);
    }
    Ok(selected)
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

#[cfg(test)]
pub fn validate_tree_command(args: &[String]) -> Result<(), PolicyError> {
    let edges = value_after(args, "--edges")
        .ok_or_else(|| PolicyError::UnpinnedTreeArgument("--edges".to_owned()))?;
    if !matches!(edges, "normal,build" | "normal,build,dev") {
        return Err(PolicyError::InvalidEdgeKinds(edges.to_owned()));
    }
    validate_tree_command_for(args, edges)
}

#[cfg(test)]
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

#[cfg(test)]
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

/// Return Cargo metadata containing only the identities selected for one
/// policy context.
///
/// Cargo metadata is resolved for the whole workspace even when a target is
/// filtered.  The package-selected `cargo tree` traversal is therefore the
/// authority for the identity set; this function only projects the already
/// captured metadata onto that set.  It does not synthesize a manifest or
/// resolve another graph.
pub fn filter_selected_metadata(
    document: &str,
    selected: &BTreeSet<PackageIdentity>,
    context: &SelectedContext,
    root: &Path,
) -> Result<Value, PolicyError> {
    let mut metadata: Value = serde_json::from_str(document)
        .map_err(|error| PolicyError::MalformedMetadata(error.to_string()))?;
    let workspace_root = root.join("packages");
    normalize_metadata_paths(&mut metadata, &workspace_root)?;

    let metadata_object = metadata.as_object_mut().ok_or_else(|| {
        PolicyError::MalformedMetadata("Cargo metadata is not an object".to_owned())
    })?;
    let package_values = metadata_object
        .get("packages")
        .and_then(Value::as_array)
        .cloned()
        .ok_or(PolicyError::MetadataPackagesMissing)?;

    let mut package_ids = BTreeMap::<PackageIdentity, String>::new();
    let mut all_ids = BTreeSet::new();
    for package in &package_values {
        let identity = package_identity(package)?;
        let id = package_id(package)?;
        if !all_ids.insert(id.clone()) {
            return Err(PolicyError::IdentityMismatch(format!(
                "duplicate package id {id}"
            )));
        }
        if package_ids.insert(identity.clone(), id.clone()).is_some() {
            return Err(PolicyError::IdentityMismatch(format!(
                "duplicate package identity {}",
                identity.key()
            )));
        }
    }

    let missing = selected
        .iter()
        .filter(|identity| !package_ids.contains_key(*identity))
        .map(PackageIdentity::key)
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(PolicyError::SelectedPackageMissing(missing.join(", ")));
    }

    let root_identities = selected
        .iter()
        .filter(|identity| identity.name == context.package)
        .collect::<Vec<_>>();
    if root_identities.len() != 1 {
        return Err(PolicyError::WrongRoot(context.package.clone()));
    }
    let root_identity = root_identities[0];
    let root_id = package_ids
        .get(root_identity)
        .cloned()
        .ok_or_else(|| PolicyError::SelectedPackageMissing(root_identity.key()))?;
    let selected_ids = selected
        .iter()
        .map(|identity| {
            package_ids
                .get(identity)
                .cloned()
                .ok_or_else(|| PolicyError::SelectedPackageMissing(identity.key()))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;

    let mut selected_packages = package_values
        .into_iter()
        .filter(|package| {
            package_identity(package)
                .map(|identity| selected.contains(&identity))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    selected_packages.sort_by(|left, right| {
        package_id(left)
            .expect("normalized Cargo package id")
            .cmp(&package_id(right).expect("normalized Cargo package id"))
    });
    metadata_object.insert("packages".to_owned(), Value::Array(selected_packages));

    filter_workspace_ids(
        metadata_object,
        "workspace_members",
        &selected_ids,
        &package_ids,
        true,
    )?;
    filter_workspace_ids(
        metadata_object,
        "workspace_default_members",
        &selected_ids,
        &package_ids,
        false,
    )?;

    let resolve = metadata_object
        .get_mut("resolve")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            PolicyError::MalformedMetadata("Cargo metadata has no resolve object".to_owned())
        })?;
    let raw_root = resolve.get("root").cloned().ok_or_else(|| {
        PolicyError::MalformedMetadata("Cargo resolve has no root field".to_owned())
    })?;
    if !raw_root.is_null() {
        let raw_root = raw_root.as_str().ok_or_else(|| {
            PolicyError::MalformedMetadata("Cargo resolve root is not a package id".to_owned())
        })?;
        if raw_root != root_id {
            return Err(PolicyError::MetadataRootMismatch(raw_root.to_owned()));
        }
    }

    let raw_nodes = resolve
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            PolicyError::MalformedMetadata("Cargo resolve has no nodes array".to_owned())
        })?;
    let mut selected_nodes = Vec::new();
    let mut node_ids = BTreeSet::new();
    for mut node in raw_nodes {
        let node_object = node.as_object_mut().ok_or_else(|| {
            PolicyError::MalformedMetadata("Cargo resolve node is not an object".to_owned())
        })?;
        let node_id = node_object
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                PolicyError::MalformedMetadata("Cargo resolve node has no id".to_owned())
            })?
            .to_owned();
        if !all_ids.contains(&node_id) {
            return Err(PolicyError::DanglingResolveEdge(node_id));
        }
        if !selected_ids.contains(&node_id) {
            continue;
        }
        if !node_ids.insert(node_id.clone()) {
            return Err(PolicyError::IdentityMismatch(format!(
                "duplicate resolve node {node_id}"
            )));
        }

        let dependencies = node_object
            .get("dependencies")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                PolicyError::MalformedMetadata(format!(
                    "resolve node {node_id} has no dependencies array"
                ))
            })?;
        let mut filtered_dependencies = Vec::new();
        for dependency in dependencies {
            let dependency_id = dependency
                .as_str()
                .ok_or_else(|| {
                    PolicyError::MalformedMetadata("resolve dependency is not an id".to_owned())
                })?
                .to_owned();
            if !all_ids.contains(&dependency_id) {
                return Err(PolicyError::DanglingResolveEdge(dependency_id));
            }
            if selected_ids.contains(&dependency_id) {
                filtered_dependencies.push(Value::String(dependency_id));
            }
        }
        filtered_dependencies.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
        node_object.insert(
            "dependencies".to_owned(),
            Value::Array(filtered_dependencies),
        );

        let deps = node_object
            .get("deps")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                PolicyError::MalformedMetadata(format!("resolve node {node_id} has no deps array"))
            })?;
        let mut filtered_deps = Vec::new();
        for dependency in deps {
            let dependency_object = dependency.as_object().ok_or_else(|| {
                PolicyError::MalformedMetadata(
                    "resolve dependency detail is not an object".to_owned(),
                )
            })?;
            let dependency_id = dependency_object
                .get("pkg")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    PolicyError::DanglingResolveEdge(format!(
                        "{node_id} has a dependency without a package id"
                    ))
                })?
                .to_owned();
            if !all_ids.contains(&dependency_id) {
                return Err(PolicyError::DanglingResolveEdge(dependency_id));
            }
            if selected_ids.contains(&dependency_id) {
                filtered_deps.push(dependency);
            }
        }
        filtered_deps.sort_by_key(dependency_sort_key);
        node_object.insert("deps".to_owned(), Value::Array(filtered_deps));
        selected_nodes.push(node);
    }

    let missing_nodes = selected_ids
        .difference(&node_ids)
        .cloned()
        .collect::<Vec<_>>();
    if !missing_nodes.is_empty() {
        return Err(PolicyError::SelectedPackageMissing(
            missing_nodes.join(", "),
        ));
    }
    selected_nodes.sort_by(|left, right| {
        left.get("id")
            .and_then(Value::as_str)
            .cmp(&right.get("id").and_then(Value::as_str))
    });
    resolve.insert("root".to_owned(), Value::String(root_id));
    resolve.insert("nodes".to_owned(), Value::Array(selected_nodes));
    Ok(metadata)
}

fn package_id(package: &Value) -> Result<String, PolicyError> {
    package
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| PolicyError::MalformedMetadata("package has no id".to_owned()))
}

fn filter_workspace_ids(
    metadata: &mut serde_json::Map<String, Value>,
    field: &str,
    selected_ids: &BTreeSet<String>,
    package_ids: &BTreeMap<PackageIdentity, String>,
    require_selected_workspace_members: bool,
) -> Result<(), PolicyError> {
    let values = metadata
        .get(field)
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            PolicyError::MalformedMetadata(format!("Cargo metadata has no {field} array"))
        })?;
    let mut members = BTreeSet::new();
    for value in values {
        let id = value
            .as_str()
            .ok_or_else(|| PolicyError::MalformedMetadata(format!("{field} contains a non-id")))?;
        if selected_ids.contains(id) {
            members.insert(id.to_owned());
        }
    }
    if require_selected_workspace_members {
        let missing = package_ids
            .iter()
            .filter(|(identity, id)| identity.source.is_none() && selected_ids.contains(*id))
            .map(|(_, id)| id)
            .filter(|id| !members.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(PolicyError::SelectedPackageMissing(missing.join(", ")));
        }
    }
    metadata.insert(
        field.to_owned(),
        Value::Array(members.into_iter().map(Value::String).collect()),
    );
    Ok(())
}

fn dependency_sort_key(value: &Value) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}",
        value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        value.get("pkg").and_then(Value::as_str).unwrap_or_default(),
        value
            .get("dep_kinds")
            .map(Value::to_string)
            .unwrap_or_default()
    )
}

fn normalize_metadata_paths(value: &mut Value, workspace_root: &Path) -> Result<(), PolicyError> {
    match value {
        Value::String(string) => {
            *string = normalize_metadata_string(string, workspace_root)?;
        }
        Value::Array(values) => {
            for value in values {
                normalize_metadata_paths(value, workspace_root)?;
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                normalize_metadata_paths(value, workspace_root)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn normalize_metadata_string(value: &str, workspace_root: &Path) -> Result<String, PolicyError> {
    for (prefix, uri_prefix) in [("path+file://", "path+file://"), ("file://", "file://")] {
        if let Some(path_value) = value.strip_prefix(prefix) {
            let (path, suffix) = path_value.split_once('#').unwrap_or((path_value, ""));
            let normalized = normalize_absolute_path(path, workspace_root)?;
            return Ok(if suffix.is_empty() {
                format!("{uri_prefix}{normalized}")
            } else {
                format!("{uri_prefix}{normalized}#{suffix}")
            });
        }
    }
    if value.starts_with('/') {
        return normalize_absolute_path(value, workspace_root);
    }
    Ok(value.to_owned())
}

fn normalize_absolute_path(value: &str, workspace_root: &Path) -> Result<String, PolicyError> {
    let path = Path::new(value);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(PolicyError::UnrecognizedAbsolutePath);
    }
    if let Ok(relative) = path.strip_prefix(workspace_root) {
        return Ok(canonical_workspace_path(relative));
    }
    if path.starts_with("/workspace/packages") {
        return Ok(value.to_owned());
    }
    for cargo_root in ["registry", "git"] {
        if let Some(relative) = cargo_path_after_marker(path, cargo_root) {
            return Ok(if relative.is_empty() {
                format!("/cargo/{cargo_root}")
            } else {
                format!("/cargo/{cargo_root}/{relative}")
            });
        }
    }
    if value.starts_with("/cargo/registry") || value.starts_with("/cargo/git") {
        return Ok(value.to_owned());
    }
    Err(PolicyError::UnrecognizedAbsolutePath)
}

fn canonical_workspace_path(relative: &Path) -> String {
    let relative = relative.to_string_lossy().replace('\\', "/");
    if relative.is_empty() {
        "/workspace/packages".to_owned()
    } else {
        format!("/workspace/packages/{relative}")
    }
}

fn cargo_path_after_marker(path: &Path, cargo_root: &str) -> Option<String> {
    let components = path.components().collect::<Vec<_>>();
    components.windows(2).enumerate().find_map(|(index, pair)| {
        if pair[0] == Component::Normal(OsStr::new(".cargo"))
            && pair[1] == Component::Normal(OsStr::new(cargo_root))
        {
            let suffix = components[index + 2..]
                .iter()
                .map(|component| component.as_os_str().to_str())
                .collect::<Option<Vec<_>>>()?
                .join("/");
            Some(suffix)
        } else {
            None
        }
    })
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
    let parsed = parse_lock_records(lock)?;
    let mut packages = parsed
        .records
        .into_iter()
        .map(|record| record.package)
        .collect::<Vec<_>>();
    packages.sort_by(|left, right| left.identity.cmp(&right.identity));
    Ok(packages)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LockRecord {
    package: LockPackage,
    block: String,
    dependency_field: Option<DependencyField>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DependencyField {
    range: Range<usize>,
    tokens: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedLock {
    version: Option<u64>,
    records: Vec<LockRecord>,
}

fn parse_lock_records(lock: &str) -> Result<ParsedLock, PolicyError> {
    let mut sections = lock.split("[[package]]");
    let preamble = sections.next().unwrap_or_default();
    let version = parse_lock_version(preamble)?;
    let mut records = Vec::new();
    let mut identities = BTreeSet::new();
    for block in sections {
        let name = toml_value(block, "name")
            .ok_or_else(|| PolicyError::MalformedLock("package has no name".to_owned()))?;
        let version = toml_value(block, "version")
            .ok_or_else(|| PolicyError::MalformedLock(format!("{name} has no version")))?;
        let source = toml_value(block, "source");
        let checksum = toml_value(block, "checksum");
        let dependency_field = parse_dependency_field(block)?;
        let dependencies = dependency_field
            .as_ref()
            .map(|field| field.tokens.clone())
            .unwrap_or_default();
        let package = LockPackage {
            identity: PackageIdentity::new(name, version, source),
            checksum,
            dependencies,
        };
        if !identities.insert(package.identity.clone()) {
            return Err(PolicyError::DuplicateLockPackage(package.identity.key()));
        }
        records.push(LockRecord {
            package,
            block: block.to_owned(),
            dependency_field,
        });
    }
    if records.is_empty() {
        return Err(PolicyError::EmptyLock);
    }
    Ok(ParsedLock { version, records })
}

fn parse_lock_version(preamble: &str) -> Result<Option<u64>, PolicyError> {
    let mut version = None;
    for line in preamble.lines() {
        let line = line.trim();
        let Some(value) = line.strip_prefix("version") else {
            continue;
        };
        if !value.starts_with(char::is_whitespace) && !value.starts_with('=') {
            continue;
        }
        let value = value
            .trim_start()
            .strip_prefix('=')
            .ok_or_else(|| PolicyError::MalformedLock("lock version has no value".to_owned()))?
            .trim();
        let parsed = value
            .parse::<u64>()
            .map_err(|_| PolicyError::MalformedLock(format!("invalid lock version {value}")))?;
        if version.replace(parsed).is_some() {
            return Err(PolicyError::MalformedLock(
                "lock version appears more than once".to_owned(),
            ));
        }
    }
    Ok(version)
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

fn parse_dependency_field(block: &str) -> Result<Option<DependencyField>, PolicyError> {
    let mut field = None;
    let mut offset = 0;
    for line in block.split_inclusive('\n') {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        let line_without_newline = line_without_newline
            .strip_suffix('\r')
            .unwrap_or(line_without_newline);
        let trimmed = line_without_newline.trim_start();
        let Some(after_key) = trimmed.strip_prefix("dependencies") else {
            offset += line.len();
            continue;
        };
        if !after_key.is_empty()
            && !after_key.starts_with(char::is_whitespace)
            && !after_key.starts_with('=')
        {
            offset += line.len();
            continue;
        }
        if field.is_some() {
            return Err(PolicyError::MalformedLock(
                "dependencies field appears more than once".to_owned(),
            ));
        }
        let key_offset = line_without_newline.len() - trimmed.len();
        let after_key_offset = key_offset + "dependencies".len();
        let after_key = &line_without_newline[after_key_offset..];
        let equals_offset = after_key.find('=').ok_or_else(|| {
            PolicyError::MalformedLock("dependencies field has no assignment".to_owned())
        })?;
        if !after_key[..equals_offset].trim().is_empty() {
            return Err(PolicyError::MalformedLock(
                "dependencies field has invalid assignment".to_owned(),
            ));
        }
        let value_offset = after_key_offset + equals_offset + 1;
        let value = &line_without_newline[value_offset..];
        let leading = value.len() - value.trim_start().len();
        let array_start = offset + value_offset + leading;
        if !block[array_start..].starts_with('[') {
            return Err(PolicyError::MalformedLock(
                "dependencies field is not an array".to_owned(),
            ));
        }
        let array_end = find_array_end(block, array_start)?;
        let tokens = parse_lock_string_array(&block[array_start + 1..array_end])?;
        field = Some(DependencyField {
            range: key_offset + offset..array_end + 1,
            tokens,
        });
        offset += line.len();
    }
    Ok(field)
}

fn find_array_end(block: &str, array_start: usize) -> Result<usize, PolicyError> {
    let bytes = block.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut in_comment = false;
    for (index, byte) in bytes.iter().enumerate().skip(array_start + 1) {
        if in_comment {
            if *byte == b'\n' {
                in_comment = false;
            }
        } else if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
        } else if *byte == b'"' {
            in_string = true;
        } else if *byte == b'#' {
            in_comment = true;
        } else if *byte == b']' {
            return Ok(index);
        }
    }
    Err(PolicyError::MalformedLock(
        "dependencies array has no closing bracket".to_owned(),
    ))
}

fn parse_lock_string_array(contents: &str) -> Result<Vec<String>, PolicyError> {
    let bytes = contents.as_bytes();
    let mut index = 0;
    let mut values = Vec::new();
    while index < bytes.len() {
        while index < bytes.len() {
            match bytes[index] {
                byte if byte.is_ascii_whitespace() || byte == b',' => index += 1,
                b'#' => {
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                }
                _ => break,
            }
        }
        if index == bytes.len() {
            break;
        }
        if bytes[index] != b'"' {
            return Err(PolicyError::MalformedLock(
                "dependencies array contains a non-string value".to_owned(),
            ));
        }
        index += 1;
        let mut value = String::new();
        let mut closed = false;
        while index < bytes.len() {
            match bytes[index] {
                b'"' => {
                    index += 1;
                    closed = true;
                    break;
                }
                b'\\' => {
                    index += 1;
                    let escaped = *bytes.get(index).ok_or_else(|| {
                        PolicyError::MalformedLock(
                            "dependencies array has an incomplete string escape".to_owned(),
                        )
                    })?;
                    let character = match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        _ => {
                            return Err(PolicyError::MalformedLock(
                                "dependencies array has an unsupported string escape".to_owned(),
                            ));
                        }
                    };
                    value.push(character);
                    index += 1;
                }
                _ => {
                    let character = contents[index..].chars().next().ok_or_else(|| {
                        PolicyError::MalformedLock(
                            "dependencies array contains invalid UTF-8".to_owned(),
                        )
                    })?;
                    value.push(character);
                    index += character.len_utf8();
                }
            }
        }
        if !closed {
            return Err(PolicyError::MalformedLock(
                "dependencies array has an unterminated string".to_owned(),
            ));
        }
        values.push(value);
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index < bytes.len() {
            if bytes[index] == b',' {
                index += 1;
            } else if bytes[index] == b'#' {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                    index += 1;
                }
                if index < bytes.len() && bytes[index] != b',' {
                    return Err(PolicyError::MalformedLock(
                        "dependencies array is missing a comma".to_owned(),
                    ));
                }
                if index < bytes.len() {
                    index += 1;
                }
            } else {
                return Err(PolicyError::MalformedLock(
                    "dependencies array is missing a comma".to_owned(),
                ));
            }
        }
    }
    Ok(values)
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

pub fn validate_selected_identity_set(
    selected: &BTreeSet<PackageIdentity>,
    actual: &BTreeSet<PackageIdentity>,
) -> Result<(), PolicyError> {
    let missing = selected
        .difference(actual)
        .map(PackageIdentity::key)
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(PolicyError::SelectedPackageMissing(missing.join(", ")));
    }
    let extra = actual
        .difference(selected)
        .map(PackageIdentity::key)
        .collect::<Vec<_>>();
    if !extra.is_empty() {
        return Err(PolicyError::SelectedPackageExtra(extra.join(", ")));
    }
    Ok(())
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

#[cfg(test)]
#[allow(dead_code)]
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

#[cfg(test)]
#[allow(dead_code)]
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

#[cfg(test)]
#[allow(dead_code)]
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

    pub fn as_selected_metadata_json(
        &self,
        metadata_json: &str,
        root: &Path,
    ) -> Result<String, PolicyError> {
        let selected = self.identities.iter().cloned().collect::<BTreeSet<_>>();
        let metadata = filter_selected_metadata(metadata_json, &selected, &self.context, root)?;
        let mut metadata = metadata.as_object().cloned().ok_or_else(|| {
            PolicyError::MalformedMetadata("Cargo metadata is not an object".to_owned())
        })?;
        let context = self.as_json().as_object().cloned().ok_or_else(|| {
            PolicyError::Serialization("policy context is not a JSON object".to_owned())
        })?;
        metadata.extend(context);
        serde_json::to_string_pretty(&Value::Object(metadata))
            .map(|json| json + "\n")
            .map_err(|error| PolicyError::Serialization(error.to_string()))
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
        let filtered_lock_identities = filtered_lock
            .iter()
            .map(|package| package.identity.clone())
            .collect::<BTreeSet<_>>();
        validate_selected_identity_set(&selected, &filtered_lock_identities)?;
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
            filtered_lock_text(&lock, &oracle.production.identities)?,
        );
        insert_policy_output(
            &mut outputs,
            &context.preview_dir(),
            "policy/metadata.json",
            policy_input.as_selected_metadata_json(&metadata_json, root)?,
        );
        insert_policy_output(
            &mut outputs,
            &context.preview_dir(),
            "policy/Cargo.lock",
            filtered_lock_text(&lock, &oracle.policy.identities)?,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct LockDependencyToken {
    name: String,
    version: Option<String>,
    source: Option<String>,
}

pub fn filtered_lock_text(
    lock: &str,
    identities: &BTreeSet<PackageIdentity>,
) -> Result<String, PolicyError> {
    if identities.is_empty() {
        return Err(PolicyError::EmptySelectedLock);
    }
    let parsed = parse_lock_records(lock)?;
    let available = parsed
        .records
        .iter()
        .map(|record| record.package.identity.clone())
        .collect::<BTreeSet<_>>();
    let missing = identities
        .difference(&available)
        .map(PackageIdentity::key)
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(PolicyError::MetadataLockMismatch(missing.join(", ")));
    }

    let mut packages_by_name = BTreeMap::<String, Vec<PackageIdentity>>::new();
    for identity in &available {
        packages_by_name
            .entry(identity.name.clone())
            .or_default()
            .push(identity.clone());
    }

    let mut output = String::from(
        "# This file is a generated policy-input preview. It is not a Cargo workspace lock authority.\n\n",
    );
    if let Some(version) = parsed.version {
        output.push_str(&format!("version = {version}\n\n"));
    }

    let mut rendered_identities = BTreeSet::new();
    for record in parsed.records {
        if !identities.contains(&record.package.identity) {
            continue;
        }
        let mut retained_dependencies = Vec::new();
        for token in &record.package.dependencies {
            let target = resolve_lock_dependency(token, &packages_by_name)?;
            if identities.contains(&target) {
                retained_dependencies.push(token.clone());
            }
        }
        let block = if let Some(field) = &record.dependency_field {
            rewrite_dependency_field(&record.block, field, &retained_dependencies)
        } else {
            record.block
        };
        output.push_str("[[package]]");
        output.push_str(block.trim_end());
        output.push_str("\n\n");
        rendered_identities.insert(record.package.identity);
    }
    validate_selected_identity_set(identities, &rendered_identities)?;
    Ok(output)
}

fn resolve_lock_dependency(
    token: &str,
    packages_by_name: &BTreeMap<String, Vec<PackageIdentity>>,
) -> Result<PackageIdentity, PolicyError> {
    let parsed = parse_lock_dependency_token(token)?;
    let candidates = packages_by_name
        .get(&parsed.name)
        .ok_or_else(|| PolicyError::UnresolvableLockDependency(token.to_owned()))?;
    let matching = candidates
        .iter()
        .filter(|identity| {
            parsed
                .version
                .as_deref()
                .is_none_or(|version| identity.version == version)
                && parsed
                    .source
                    .as_deref()
                    .is_none_or(|source| identity.source.as_deref() == Some(source))
        })
        .cloned()
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [identity] => Ok(identity.clone()),
        [] => Err(PolicyError::UnresolvableLockDependency(token.to_owned())),
        _ => Err(PolicyError::AmbiguousLockDependency(token.to_owned())),
    }
}

fn parse_lock_dependency_token(token: &str) -> Result<LockDependencyToken, PolicyError> {
    if token.is_empty() || token.trim() != token {
        return Err(PolicyError::MalformedLockDependency(token.to_owned()));
    }
    let (head, source) = if let Some(source_start) = token.rfind(" (") {
        if !token.ends_with(')')
            || token[..source_start].contains(['(', ')'])
            || token[source_start + 2..token.len() - 1].is_empty()
            || token[source_start + 2..token.len() - 1].contains(['(', ')'])
        {
            return Err(PolicyError::MalformedLockDependency(token.to_owned()));
        }
        (
            &token[..source_start],
            Some(token[source_start + 2..token.len() - 1].to_owned()),
        )
    } else {
        if token.contains(['(', ')']) {
            return Err(PolicyError::MalformedLockDependency(token.to_owned()));
        }
        (token, None)
    };
    let parts = head.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 2 || (source.is_some() && parts.len() != 2) {
        return Err(PolicyError::MalformedLockDependency(token.to_owned()));
    }
    if head != parts.join(" ") {
        return Err(PolicyError::MalformedLockDependency(token.to_owned()));
    }
    Ok(LockDependencyToken {
        name: parts[0].to_owned(),
        version: parts.get(1).map(|version| (*version).to_owned()),
        source,
    })
}

fn rewrite_dependency_field(
    block: &str,
    field: &DependencyField,
    retained_dependencies: &[String],
) -> String {
    let mut replacement = String::from("dependencies = ");
    if retained_dependencies.is_empty() {
        replacement.push_str("[]");
    } else {
        replacement.push_str("[\n");
        for dependency in retained_dependencies {
            replacement.push_str(" \"");
            replacement.push_str(&escape_toml_string(dependency));
            replacement.push_str("\",\n");
        }
        replacement.push(']');
    }
    format!(
        "{}{}{}",
        &block[..field.range.start],
        replacement,
        &block[field.range.end..]
    )
}

fn escape_toml_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

pub fn gen_package_policy_inputs(args: &[String]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mode = parse_package_policy_mode(args)?;
    let root = repo_root()?;
    let outputs = package_policy_preview(&root)?;
    match mode {
        PackagePolicyMode::Preview => write_policy_preview(&root, &outputs),
        PackagePolicyMode::Install => {
            install_policy_outputs(&root, &outputs)?;
            Ok(outputs.keys().map(PathBuf::from).collect())
        }
        PackagePolicyMode::Check => {
            check_policy_outputs(&root, &outputs)?;
            Ok(outputs.keys().map(PathBuf::from).collect())
        }
    }
}

pub fn check_policy_outputs(
    root: &Path,
    expected: &BTreeMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    if expected.is_empty() {
        return Err(POLICY_DRIFT_REMEDIATION.into());
    }
    let final_root = root.join("packages/policy-inputs");
    anchor_policy_directory(&final_root)?;

    let actual = policy_file_census(&final_root)?;
    let expected_paths = expected
        .keys()
        .map(|relative| {
            Path::new(relative)
                .strip_prefix("packages/policy-inputs")
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .map_err(|_| POLICY_DRIFT_REMEDIATION.into())
        })
        .collect::<Result<BTreeSet<_>, Box<dyn Error>>>()?;
    let expected_directories = policy_directory_set(&expected_paths);
    let actual_directories = policy_directory_census(&final_root)?;
    if actual_directories != expected_directories {
        return Err(policy_directory_drift(
            &expected_directories,
            &actual_directories,
        ));
    }
    if actual != expected_paths {
        return Err(policy_census_drift(&expected_paths, &actual));
    }

    for (relative, contents) in expected {
        let relative = Path::new(relative)
            .strip_prefix("packages/policy-inputs")
            .map_err(|_| POLICY_DRIFT_REMEDIATION)?;
        let actual = read_policy_leaf(&final_root, relative)?;
        if actual != *contents {
            return Err(POLICY_DRIFT_REMEDIATION.into());
        }
    }
    Ok(())
}

fn write_policy_preview(
    root: &Path,
    outputs: &BTreeMap<String, String>,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let preview_root = root.join(POLICY_PREVIEW_ROOT);
    if let Ok(metadata) = fs::symlink_metadata(&preview_root) {
        if !metadata.file_type().is_dir() {
            return Err(POLICY_DRIFT_REMEDIATION.into());
        }
        anchor_policy_directory(&preview_root)?;
        remove_policy_extras(&preview_root, &BTreeSet::new())?;
    }
    ensure_policy_directory(&preview_root)?;

    for (relative, contents) in outputs {
        let relative = policy_preview_relative(relative)?;
        let path = preview_root.join(relative);
        atomic_policy_write(&path, contents)?;
    }

    let actual = policy_file_census(&preview_root)?;
    let expected = outputs
        .keys()
        .map(|relative| {
            policy_preview_relative(relative).map(|path| path.to_string_lossy().replace('\\', "/"))
        })
        .collect::<Result<BTreeSet<_>, Box<dyn Error>>>()?;
    if actual != expected {
        return Err(policy_census_drift(&expected, &actual));
    }
    outputs
        .keys()
        .map(|relative| {
            policy_preview_relative(relative)
                .map(|path| PathBuf::from(POLICY_PREVIEW_ROOT).join(path))
        })
        .collect()
}

fn install_policy_outputs(
    root: &Path,
    outputs: &BTreeMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    let final_root = root.join("packages/policy-inputs");
    if let Ok(metadata) = fs::symlink_metadata(&final_root) {
        if !metadata.file_type().is_dir() {
            return Err(POLICY_DRIFT_REMEDIATION.into());
        }
        policy_file_census(&final_root)?;
    } else {
        ensure_policy_directory(&final_root)?;
    }

    for (relative, contents) in outputs {
        let relative = policy_preview_relative(relative)?;
        let path = final_root.join(relative);
        atomic_policy_write(&path, contents)?;
    }

    let expected = outputs
        .keys()
        .map(|relative| {
            Path::new(relative)
                .strip_prefix("packages/policy-inputs")
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .map_err(|_| POLICY_DRIFT_REMEDIATION.into())
        })
        .collect::<Result<BTreeSet<_>, Box<dyn Error>>>()?;
    remove_policy_extras(&final_root, &expected)?;
    check_policy_outputs(root, outputs)
}

fn policy_preview_relative(relative: &str) -> Result<PathBuf, Box<dyn Error>> {
    Path::new(relative)
        .strip_prefix("packages/policy-inputs")
        .map(Path::to_path_buf)
        .map_err(|_| POLICY_DRIFT_REMEDIATION.into())
}

fn anchor_policy_directory(path: &Path) -> Result<std::fs::File, Box<dyn Error>> {
    reject_policy_symlink_components(path)?;
    let descriptor = rustix::fs::openat2(
        rustix::fs::CWD,
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| POLICY_DRIFT_REMEDIATION)?;
    let file = std::fs::File::from(descriptor);
    let named = fs::symlink_metadata(path).map_err(|_| POLICY_DRIFT_REMEDIATION)?;
    let pinned = rustix::fs::fstat(file.as_fd()).map_err(|_| POLICY_DRIFT_REMEDIATION)?;
    if named.dev() != pinned.st_dev || named.ino() != pinned.st_ino {
        return Err(POLICY_DRIFT_REMEDIATION.into());
    }
    Ok(file)
}

fn ensure_policy_directory(path: &Path) -> Result<(), Box<dyn Error>> {
    let mut current = std::fs::File::from(
        rustix::fs::open(
            "/",
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| POLICY_DRIFT_REMEDIATION)?,
    );
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            if matches!(
                component,
                std::path::Component::RootDir | std::path::Component::Prefix(_)
            ) {
                continue;
            }
            return Err(POLICY_DRIFT_REMEDIATION.into());
        };
        let name = name.to_str().ok_or(POLICY_DRIFT_REMEDIATION)?;
        let child = match rustix::fs::openat(
            current.as_fd(),
            name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        ) {
            Ok(child) => child,
            Err(rustix::io::Errno::NOENT) => {
                rustix::fs::mkdirat(
                    current.as_fd(),
                    name,
                    rustix::fs::Mode::from_bits_truncate(0o755),
                )
                .map_err(|_| POLICY_DRIFT_REMEDIATION)?;
                rustix::fs::fsync(current.as_fd()).map_err(|_| POLICY_DRIFT_REMEDIATION)?;
                rustix::fs::openat(
                    current.as_fd(),
                    name,
                    rustix::fs::OFlags::RDONLY
                        | rustix::fs::OFlags::DIRECTORY
                        | rustix::fs::OFlags::NOFOLLOW
                        | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .map_err(|_| POLICY_DRIFT_REMEDIATION)?
            }
            Err(_) => return Err(POLICY_DRIFT_REMEDIATION.into()),
        };
        let named =
            rustix::fs::statat(current.as_fd(), name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| POLICY_DRIFT_REMEDIATION)?;
        let pinned = rustix::fs::fstat(&child).map_err(|_| POLICY_DRIFT_REMEDIATION)?;
        if named.st_dev != pinned.st_dev || named.st_ino != pinned.st_ino {
            return Err(POLICY_DRIFT_REMEDIATION.into());
        }
        current = std::fs::File::from(child);
    }
    Ok(())
}

fn read_policy_leaf(root: &Path, relative: &Path) -> Result<String, Box<dyn Error>> {
    let parent = root.join(relative.parent().unwrap_or_else(|| Path::new("")));
    let directory = anchor_policy_directory(&parent)?;
    let name = relative
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or(POLICY_DRIFT_REMEDIATION)?;
    let descriptor = rustix::fs::openat(
        directory.as_fd(),
        name,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| POLICY_DRIFT_REMEDIATION)?;
    let mut file = std::fs::File::from(descriptor);
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|_| POLICY_DRIFT_REMEDIATION)?;
    Ok(contents)
}

fn policy_file_census(root: &Path) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let _ = anchor_policy_directory(root)?;
    let mut files = BTreeSet::new();
    collect_policy_entries(root, root, &mut files)?;
    Ok(files)
}

fn policy_directory_census(root: &Path) -> Result<BTreeSet<String>, Box<dyn Error>> {
    let _ = anchor_policy_directory(root)?;
    let mut directories = BTreeSet::new();
    collect_policy_directories(root, root, &mut directories)?;
    Ok(directories)
}

fn collect_policy_directories(
    root: &Path,
    current: &Path,
    directories: &mut BTreeSet<String>,
) -> Result<(), Box<dyn Error>> {
    let _ = anchor_policy_directory(current)?;
    for entry in fs::read_dir(current).map_err(|_| POLICY_DRIFT_REMEDIATION)? {
        let entry = entry.map_err(|_| POLICY_DRIFT_REMEDIATION)?;
        let file_type = entry.file_type().map_err(|_| POLICY_DRIFT_REMEDIATION)?;
        if file_type.is_dir() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| POLICY_DRIFT_REMEDIATION)?
                .to_string_lossy()
                .replace('\\', "/");
            directories.insert(relative);
            collect_policy_directories(root, &entry.path(), directories)?;
        } else if !file_type.is_file() {
            return Err(POLICY_DRIFT_REMEDIATION.into());
        }
    }
    Ok(())
}

fn policy_directory_set(files: &BTreeSet<String>) -> BTreeSet<String> {
    let mut directories = BTreeSet::new();
    for file in files {
        let mut current = Path::new(file);
        while let Some(parent) = current.parent() {
            if parent.as_os_str().is_empty() || parent == Path::new(".") {
                break;
            }
            directories.insert(parent.to_string_lossy().replace('\\', "/"));
            current = parent;
        }
    }
    directories
}

fn collect_policy_entries(
    root: &Path,
    current: &Path,
    files: &mut BTreeSet<String>,
) -> Result<(), Box<dyn Error>> {
    let _ = anchor_policy_directory(current)?;
    for entry in fs::read_dir(current).map_err(|_| POLICY_DRIFT_REMEDIATION)? {
        let entry = entry.map_err(|_| POLICY_DRIFT_REMEDIATION)?;
        let file_type = entry.file_type().map_err(|_| POLICY_DRIFT_REMEDIATION)?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_policy_entries(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| POLICY_DRIFT_REMEDIATION)?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative);
        } else {
            return Err(POLICY_DRIFT_REMEDIATION.into());
        }
    }
    Ok(())
}

fn policy_census_drift(expected: &BTreeSet<String>, actual: &BTreeSet<String>) -> Box<dyn Error> {
    let missing = expected.difference(actual).cloned().collect::<Vec<_>>();
    let extra = actual.difference(expected).cloned().collect::<Vec<_>>();
    let mut message = POLICY_DRIFT_REMEDIATION.to_owned();
    if !missing.is_empty() {
        message.push_str("\nMissing paths: ");
        message.push_str(&missing.join(", "));
    }
    if !extra.is_empty() {
        message.push_str("\nExtra paths: ");
        message.push_str(&extra.join(", "));
    }
    message.into()
}

fn policy_directory_drift(
    expected: &BTreeSet<String>,
    actual: &BTreeSet<String>,
) -> Box<dyn Error> {
    let missing = expected.difference(actual).cloned().collect::<Vec<_>>();
    let extra = actual.difference(expected).cloned().collect::<Vec<_>>();
    let mut message = POLICY_DRIFT_REMEDIATION.to_owned();
    if !missing.is_empty() {
        message.push_str("\nMissing directories: ");
        message.push_str(&missing.join(", "));
    }
    if !extra.is_empty() {
        message.push_str("\nExtra directories: ");
        message.push_str(&extra.join(", "));
    }
    message.into()
}

fn remove_policy_extras(root: &Path, expected: &BTreeSet<String>) -> Result<(), Box<dyn Error>> {
    let actual = policy_file_census(root)?;
    for relative in actual.difference(expected) {
        let relative = Path::new(relative);
        let parent = root.join(relative.parent().unwrap_or_else(|| Path::new("")));
        let directory = anchor_policy_directory(&parent)?;
        let name = relative
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or(POLICY_DRIFT_REMEDIATION)?;
        rustix::fs::unlinkat(directory.as_fd(), name, rustix::fs::AtFlags::empty())
            .map_err(|_| POLICY_DRIFT_REMEDIATION)?;
        rustix::fs::fsync(directory.as_fd()).map_err(|_| POLICY_DRIFT_REMEDIATION)?;
    }
    let expected_directories = policy_directory_set(expected);
    remove_policy_extra_directories(root, root, &expected_directories)?;
    Ok(())
}

fn remove_policy_extra_directories(
    root: &Path,
    current: &Path,
    expected: &BTreeSet<String>,
) -> Result<(), Box<dyn Error>> {
    let _ = anchor_policy_directory(current)?;
    let entries = fs::read_dir(current)
        .map_err(|_| POLICY_DRIFT_REMEDIATION)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| POLICY_DRIFT_REMEDIATION)?;
    for entry in entries {
        let file_type = entry.file_type().map_err(|_| POLICY_DRIFT_REMEDIATION)?;
        if !file_type.is_dir() {
            continue;
        }
        remove_policy_extra_directories(root, &entry.path(), expected)?;
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| POLICY_DRIFT_REMEDIATION)?
            .to_string_lossy()
            .replace('\\', "/");
        if !expected.contains(&relative)
            && fs::read_dir(entry.path())
                .map_err(|_| POLICY_DRIFT_REMEDIATION)?
                .next()
                .is_none()
        {
            let entry_path = entry.path();
            let parent = entry_path.parent().ok_or(POLICY_DRIFT_REMEDIATION)?;
            let directory = anchor_policy_directory(parent)?;
            let name = entry_path
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or(POLICY_DRIFT_REMEDIATION)?
                .to_owned();
            rustix::fs::unlinkat(
                directory.as_fd(),
                name.as_str(),
                rustix::fs::AtFlags::REMOVEDIR,
            )
            .map_err(|_| POLICY_DRIFT_REMEDIATION)?;
            rustix::fs::fsync(directory.as_fd()).map_err(|_| POLICY_DRIFT_REMEDIATION)?;
        }
    }
    Ok(())
}

fn atomic_policy_write(path: &Path, contents: &str) -> Result<(), Box<dyn Error>> {
    let parent = path.parent().ok_or(POLICY_DRIFT_REMEDIATION)?;
    ensure_policy_directory(parent)?;
    let parent_fd = rustix::fs::openat2(
        rustix::fs::CWD,
        parent,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| POLICY_DRIFT_REMEDIATION)?;
    let named = fs::symlink_metadata(parent).map_err(|_| POLICY_DRIFT_REMEDIATION)?;
    let pinned = rustix::fs::fstat(parent_fd.as_fd()).map_err(|_| POLICY_DRIFT_REMEDIATION)?;
    if named.dev() != pinned.st_dev || named.ino() != pinned.st_ino {
        return Err(POLICY_DRIFT_REMEDIATION.into());
    }
    if let Ok(metadata) = fs::symlink_metadata(path)
        && !metadata.file_type().is_file()
    {
        return Err(POLICY_DRIFT_REMEDIATION.into());
    }
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or(POLICY_DRIFT_REMEDIATION)?;
    for attempt in 0..100_u32 {
        let temporary = format!(".{file_name}.d2b-install-{}-{attempt}", std::process::id());
        let descriptor = match rustix::fs::openat(
            parent_fd.as_fd(),
            temporary.as_str(),
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_bits_truncate(0o600),
        ) {
            Ok(descriptor) => descriptor,
            Err(rustix::io::Errno::EXIST) => continue,
            Err(_) => return Err(POLICY_DRIFT_REMEDIATION.into()),
        };
        let mut file = fs::File::from(descriptor);
        if file.write_all(contents.as_bytes()).is_err() || file.sync_all().is_err() {
            let _ = rustix::fs::unlinkat(
                parent_fd.as_fd(),
                temporary.as_str(),
                rustix::fs::AtFlags::empty(),
            );
            return Err(POLICY_DRIFT_REMEDIATION.into());
        }
        drop(file);
        if rustix::fs::renameat(
            parent_fd.as_fd(),
            temporary.as_str(),
            parent_fd.as_fd(),
            file_name,
        )
        .is_err()
        {
            let _ = rustix::fs::unlinkat(
                parent_fd.as_fd(),
                temporary.as_str(),
                rustix::fs::AtFlags::empty(),
            );
            return Err(POLICY_DRIFT_REMEDIATION.into());
        }
        rustix::fs::fsync(parent_fd.as_fd()).map_err(|_| POLICY_DRIFT_REMEDIATION)?;
        return Ok(());
    }
    Err(POLICY_DRIFT_REMEDIATION.into())
}

fn reject_policy_symlink_components(path: &Path) -> Result<(), Box<dyn Error>> {
    let mut current = if path.is_absolute() {
        PathBuf::from("/")
    } else {
        env::current_dir()?
    };
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(POLICY_DRIFT_REMEDIATION.into());
            }
            Ok(metadata) if !metadata.file_type().is_dir() => {
                return Err(POLICY_DRIFT_REMEDIATION.into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err(POLICY_DRIFT_REMEDIATION.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(dead_code)]
pub fn package_policy_drift_message() -> &'static str {
    POLICY_DRIFT_REMEDIATION
}

fn repo_root() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(root) = env::var_os("D2B_BAZEL_WORKTREE") {
        return fs::canonicalize(root).map_err(|_| {
            "D2B-BZL-WORKTREE: D2B_BAZEL_WORKTREE is not a repository directory.".into()
        });
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

#[derive(Clone, Eq, PartialEq)]
#[allow(dead_code)]
pub enum PolicyError {
    WrongSystem(String),
    ContextManifestInvalid,
    ContextMismatch,
    UnpinnedTreeArgument(String),
    UnpinnedTreeFormat,
    InvalidEdgeKinds(String),
    MalformedMetadata(String),
    MetadataPackagesMissing,
    IdentityMismatch(String),
    DuplicateLockPackage(String),
    SelectedPackageMissing(String),
    SelectedPackageExtra(String),
    DanglingResolveEdge(String),
    MetadataRootMismatch(String),
    UnrecognizedAbsolutePath,
    MalformedLock(String),
    EmptyLock,
    EmptySelectedLock,
    MalformedLockDependency(String),
    UnresolvableLockDependency(String),
    AmbiguousLockDependency(String),
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
    CommandFailed {
        command: String,
        status: String,
        diagnostic: String,
    },
    Serialization(String),
}

impl fmt::Debug for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl PolicyError {
    fn code(&self) -> &'static str {
        match self {
            Self::WrongSystem(_) => "D2B-POLICY-WRONG-SYSTEM",
            Self::ContextManifestInvalid => "D2B-POLICY-CONTEXT-MANIFEST",
            Self::ContextMismatch => "D2B-POLICY-CONTEXT-MISMATCH",
            Self::UnpinnedTreeArgument(_) => "D2B-POLICY-UNPINNED-TREE-ARGUMENT",
            Self::UnpinnedTreeFormat => "D2B-POLICY-UNPINNED-TREE-FORMAT",
            Self::InvalidEdgeKinds(_) => "D2B-POLICY-INVALID-EDGE-KINDS",
            Self::MalformedMetadata(_) => "D2B-POLICY-MALFORMED-METADATA",
            Self::MetadataPackagesMissing => "D2B-POLICY-METADATA-PACKAGES",
            Self::IdentityMismatch(_) => "D2B-POLICY-IDENTITY-MISMATCH",
            Self::DuplicateLockPackage(_) => "D2B-POLICY-DUPLICATE-LOCK-PACKAGE",
            Self::SelectedPackageMissing(_) => "D2B-POLICY-SELECTED-PACKAGE-MISSING",
            Self::SelectedPackageExtra(_) => "D2B-POLICY-SELECTED-PACKAGE-EXTRA",
            Self::DanglingResolveEdge(_) => "D2B-POLICY-DANGLING-EDGE",
            Self::MetadataRootMismatch(_) => "D2B-POLICY-METADATA-ROOT",
            Self::UnrecognizedAbsolutePath => "D2B-POLICY-ABSOLUTE-PATH",
            Self::MalformedLock(_) => "D2B-POLICY-MALFORMED-LOCK",
            Self::EmptyLock => "D2B-POLICY-EMPTY-LOCK",
            Self::EmptySelectedLock => "D2B-POLICY-EMPTY-SELECTED-LOCK",
            Self::MalformedLockDependency(_) => "D2B-POLICY-MALFORMED-LOCK-DEPENDENCY",
            Self::UnresolvableLockDependency(_) => "D2B-POLICY-UNRESOLVABLE-LOCK-DEPENDENCY",
            Self::AmbiguousLockDependency(_) => "D2B-POLICY-AMBIGUOUS-LOCK-DEPENDENCY",
            Self::EmptySourceCensus => "D2B-POLICY-EMPTY-SOURCE-CENSUS",
            Self::MetadataLockMismatch(_) => "D2B-POLICY-METADATA-LOCK-MISMATCH",
            Self::ExtraSource(_) => "D2B-POLICY-EXTRA-SOURCE",
            Self::ChecksumMissing(_) => "D2B-POLICY-CHECKSUM-MISSING",
            Self::SourceMissing(_) => "D2B-POLICY-SOURCE-MISSING",
            Self::GitArchivePinMissing(_) => "D2B-POLICY-GIT-ARCHIVE-PIN",
            Self::MalformedTree => "D2B-POLICY-MALFORMED-TREE",
            Self::EmptyTree => "D2B-POLICY-EMPTY-TREE",
            Self::RootCount(_) => "D2B-POLICY-ROOT-COUNT",
            Self::WrongRoot(_) => "D2B-POLICY-WRONG-ROOT",
            Self::TreeIdentityMismatch(_) => "D2B-POLICY-TREE-IDENTITY",
            Self::PolicyClosureShrank => "D2B-POLICY-CLOSURE-SHRANK",
            Self::FeatureUnionLeak(_) => "D2B-POLICY-FEATURE-UNION",
            Self::CandidateEdgeMissing(_) => "D2B-POLICY-CANDIDATE-EDGE",
            Self::Io(_, _) => "D2B-POLICY-IO",
            Self::CommandFailed { .. } => "D2B-POLICY-COMMAND",
            Self::Serialization(_) => "D2B-POLICY-SERIALIZATION",
        }
    }
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongSystem(system) => write!(
                formatter,
                "wrong native system: {}",
                redacted_policy_text(system)
            ),
            Self::ContextManifestInvalid => {
                formatter.write_str("native policy/check manifest is invalid")
            }
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
                write!(
                    formatter,
                    "malformed Cargo metadata: {}",
                    redacted_policy_text(message)
                )
            }
            Self::MetadataPackagesMissing => {
                formatter.write_str("Cargo metadata has no packages array")
            }
            Self::IdentityMismatch(message) => {
                write!(
                    formatter,
                    "metadata identity mismatch: {}",
                    redacted_policy_text(message)
                )
            }
            Self::DuplicateLockPackage(identity) => {
                write!(formatter, "Cargo.lock repeats package identity: {identity}")
            }
            Self::SelectedPackageMissing(message) => {
                write!(
                    formatter,
                    "selected package is missing from Cargo metadata: {message}"
                )
            }
            Self::SelectedPackageExtra(message) => {
                write!(
                    formatter,
                    "selected package set has an extra identity: {message}"
                )
            }
            Self::DanglingResolveEdge(message) => {
                write!(
                    formatter,
                    "Cargo resolve graph has a dangling edge: {message}"
                )
            }
            Self::MetadataRootMismatch(root) => {
                write!(
                    formatter,
                    "Cargo metadata resolve root is not selected: {}",
                    redacted_policy_text(root)
                )
            }
            Self::UnrecognizedAbsolutePath => formatter.write_str(
                "Cargo metadata contains an unrecognized absolute path (path class rejected)",
            ),
            Self::MalformedLock(message) => write!(formatter, "malformed Cargo.lock: {message}"),
            Self::EmptyLock => formatter.write_str("Cargo.lock has no package records"),
            Self::EmptySelectedLock => {
                formatter.write_str("selected package lock would contain no package records")
            }
            Self::MalformedLockDependency(token) => {
                write!(formatter, "malformed Cargo.lock dependency token: {token}")
            }
            Self::UnresolvableLockDependency(token) => {
                write!(
                    formatter,
                    "Cargo.lock dependency has no package target: {token}"
                )
            }
            Self::AmbiguousLockDependency(token) => {
                write!(
                    formatter,
                    "Cargo.lock dependency has multiple package targets: {token}"
                )
            }
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
                "selected source is missing or unreadable: {}",
                redacted_policy_text(message)
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
                let _ = message;
                write!(
                    formatter,
                    "package-policy input {} failed",
                    redacted_policy_text(path)
                )
            }
            Self::CommandFailed {
                command,
                status,
                diagnostic,
            } => {
                write!(
                    formatter,
                    "package-policy command {} failed with status {}: {}",
                    redacted_policy_text(command),
                    redacted_policy_text(status),
                    redacted_policy_text(diagnostic)
                )
            }
            Self::Serialization(message) => write!(
                formatter,
                "cannot serialize package-policy input: {}",
                redacted_policy_text(message)
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

    #[test]
    fn policy_error_renderings_redact_rejected_paths_and_child_output() {
        let error = PolicyError::UnrecognizedAbsolutePath;
        let display = error.to_string();
        let debug = format!("{error:?}");
        assert!(!display.contains("/"));
        assert!(!debug.contains("/"));

        let command = PolicyError::CommandFailed {
            command: "cargo metadata package=d2b target=x86_64 hub=product".to_owned(),
            status: "36".to_owned(),
            diagnostic: bounded_redacted_diagnostic(
                b"failure at /home/operator/private and /nix/store/secret",
            ),
        };
        let rendered = command.to_string();
        assert!(!rendered.contains("/home/operator"));
        assert!(!rendered.contains("/nix/store"));
        assert!(rendered.contains("<path>"));
        assert_eq!(format!("{command:?}"), "D2B-POLICY-COMMAND");
    }
}
