use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{DELIVERY_SCHEMA_VERSION, DeliveryError, Result};

pub const PANEL_ROLES: [PanelRole; 10] = [
    PanelRole::Software,
    PanelRole::Test,
    PanelRole::Nixos,
    PanelRole::Networking,
    PanelRole::Security,
    PanelRole::Rust,
    PanelRole::Product,
    PanelRole::Docs,
    PanelRole::Observability,
    PanelRole::Kernel,
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackManifest {
    pub schema_version: u32,
    pub wave: String,
    pub root_repository: RootRepositorySpec,
    pub repository_set: Vec<RepositorySpec>,
    pub stack: Vec<StackNodeSpec>,
    pub required_validations: Vec<RequiredValidationSpec>,
    pub required_checks: Vec<RequiredCheck>,
    #[serde(default)]
    pub generated_artifacts: Vec<FingerprintSpec>,
    #[serde(default)]
    pub dependency_fingerprints: Vec<FingerprintSpec>,
    #[serde(default)]
    pub contract_fingerprints: Vec<FingerprintSpec>,
}

impl StackManifest {
    pub fn validate(&self) -> Result<()> {
        ensure_schema(self.schema_version, "stack manifest")?;
        validate_identifier(&self.wave, "wave")?;
        validate_repository_name(&self.root_repository.name)?;
        nonempty(&self.root_repository.base, "root repository base")?;
        nonempty(&self.root_repository.head, "root repository head")?;
        if self.repository_set.is_empty() {
            return Err(DeliveryError::new("repository_set must not be empty"));
        }
        if self.stack.is_empty() {
            return Err(DeliveryError::new("stack must not be empty"));
        }
        if self.required_validations.is_empty() {
            return Err(DeliveryError::new("required_validations must not be empty"));
        }
        if self.required_checks.is_empty() {
            return Err(DeliveryError::new("required_checks must not be empty"));
        }

        let mut repositories = BTreeSet::new();
        let mut roots = BTreeSet::new();
        for repository in &self.repository_set {
            validate_repository_name(&repository.name)?;
            nonempty(&repository.head, "repository head")?;
            let root_key = normalized_path_key(&repository.root)?;
            if !repositories.insert(repository.name.as_str()) {
                return Err(DeliveryError::new(format!(
                    "duplicate repository name {}",
                    repository.name
                )));
            }
            if !roots.insert(root_key) {
                return Err(DeliveryError::new(format!(
                    "duplicate repository root {}",
                    repository.root.display()
                )));
            }
        }
        if !repositories.contains(self.root_repository.name.as_str()) {
            return Err(DeliveryError::new(format!(
                "root repository {} is absent from repository_set",
                self.root_repository.name
            )));
        }

        self.validate_stack(&repositories)?;
        self.validate_validations()?;
        self.validate_checks()?;
        validate_fingerprint_specs(
            "generated_artifacts",
            &self.generated_artifacts,
            &repositories,
        )?;
        validate_fingerprint_specs(
            "dependency_fingerprints",
            &self.dependency_fingerprints,
            &repositories,
        )?;
        validate_fingerprint_specs(
            "contract_fingerprints",
            &self.contract_fingerprints,
            &repositories,
        )?;
        Ok(())
    }

    fn validate_stack(&self, repositories: &BTreeSet<&str>) -> Result<()> {
        let mut ids = BTreeSet::new();
        let mut branches = BTreeSet::new();
        let mut pull_requests = BTreeSet::new();
        for node in &self.stack {
            validate_identifier(&node.id, "stack node id")?;
            if !ids.insert(node.id.as_str()) {
                return Err(DeliveryError::new(format!(
                    "duplicate stack node id {}",
                    node.id
                )));
            }
            if !repositories.contains(node.repository.as_str()) {
                return Err(DeliveryError::new(format!(
                    "stack node {} references unknown repository {}",
                    node.id, node.repository
                )));
            }
            nonempty(&node.branch, "stack branch")?;
            nonempty(&node.head, "stack head")?;
            if !branches.insert((node.repository.as_str(), node.branch.as_str())) {
                return Err(DeliveryError::new(format!(
                    "duplicate branch {} in repository {}",
                    node.branch, node.repository
                )));
            }
            if let Some(pr) = node.pr {
                if pr == 0 {
                    return Err(DeliveryError::new(format!(
                        "stack node {} has invalid PR number 0",
                        node.id
                    )));
                }
                if !pull_requests.insert((node.repository.as_str(), pr)) {
                    return Err(DeliveryError::new(format!(
                        "duplicate PR {pr} in repository {}",
                        node.repository
                    )));
                }
            }
            let mut dependencies = BTreeSet::new();
            for dependency in &node.depends_on {
                if !dependencies.insert(dependency.as_str()) {
                    return Err(DeliveryError::new(format!(
                        "stack node {} repeats dependency {}",
                        node.id, dependency
                    )));
                }
            }
        }

        let by_id = self
            .stack
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        graph_order(&self.stack)?;
        for (index, node) in self.stack.iter().enumerate() {
            for dependency in &node.depends_on {
                let Some(dependency_index) = by_id.get(dependency.as_str()).copied() else {
                    return Err(DeliveryError::new(format!(
                        "stack node {} references unknown dependency {}",
                        node.id, dependency
                    )));
                };
                if dependency_index >= index {
                    return Err(DeliveryError::new(format!(
                        "stack is not dependency ordered: {} must precede {}",
                        dependency, node.id
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_validations(&self) -> Result<()> {
        let mut ids = BTreeSet::new();
        for validation in &self.required_validations {
            validate_identifier(&validation.id, "validation id")?;
            nonempty(&validation.command, "validation command")?;
            if !ids.insert(validation.id.as_str()) {
                return Err(DeliveryError::new(format!(
                    "duplicate validation id {}",
                    validation.id
                )));
            }
        }
        Ok(())
    }

    fn validate_checks(&self) -> Result<()> {
        let node_ids = self
            .stack
            .iter()
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut checks = BTreeSet::new();
        let mut nodes_with_checks = BTreeSet::new();
        for check in &self.required_checks {
            if !node_ids.contains(check.node.as_str()) {
                return Err(DeliveryError::new(format!(
                    "required check {} references unknown stack node {}",
                    check.name, check.node
                )));
            }
            nonempty(&check.name, "required check name")?;
            if !checks.insert((check.node.as_str(), check.name.as_str())) {
                return Err(DeliveryError::new(format!(
                    "duplicate required check {} for stack node {}",
                    check.name, check.node
                )));
            }
            nodes_with_checks.insert(check.node.as_str());
        }
        for node in &self.stack {
            if !nodes_with_checks.contains(node.id.as_str()) {
                return Err(DeliveryError::new(format!(
                    "stack node {} has no required checks",
                    node.id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RootRepositorySpec {
    pub name: String,
    pub root: PathBuf,
    pub base: String,
    pub head: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositorySpec {
    pub name: String,
    pub root: PathBuf,
    pub head: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackNodeSpec {
    pub id: String,
    pub repository: String,
    pub branch: String,
    pub pr: Option<u64>,
    pub head: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredValidationSpec {
    pub id: String,
    pub command: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredCheck {
    pub node: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FingerprintSpec {
    pub name: String,
    pub repository: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WaveSnapshot {
    pub schema_version: u32,
    pub wave: String,
    pub root_repository: RootRepository,
    pub repository_set: Vec<RepositoryRecord>,
    pub stack: Vec<StackNode>,
    pub required_validations: Vec<RequiredValidation>,
    pub required_checks: Vec<RequiredCheck>,
    pub generated_artifacts: Vec<Fingerprint>,
    pub dependency_fingerprints: Vec<Fingerprint>,
    pub contract_fingerprints: Vec<Fingerprint>,
}

impl WaveSnapshot {
    pub fn validate(&self) -> Result<()> {
        ensure_schema(self.schema_version, "wave snapshot")?;
        validate_identifier(&self.wave, "wave")?;
        validate_repository_name(&self.root_repository.name)?;
        validate_hash(&self.root_repository.base_commit, "base commit")?;
        validate_hash(&self.root_repository.head_commit, "head commit")?;
        validate_hash(&self.root_repository.tree_hash, "integrated tree")?;
        if !Path::new(&self.root_repository.root).is_absolute() {
            return Err(DeliveryError::new(
                "snapshot root repository path must be absolute",
            ));
        }
        if self.repository_set.is_empty() {
            return Err(DeliveryError::new(
                "snapshot repository_set must not be empty",
            ));
        }
        let mut repository_names = BTreeSet::new();
        let mut repository_roots = BTreeSet::new();
        for repository in &self.repository_set {
            validate_repository_name(&repository.name)?;
            validate_hash(&repository.head_commit, "repository head")?;
            validate_hash(&repository.tree_hash, "repository tree")?;
            if !Path::new(&repository.root).is_absolute() {
                return Err(DeliveryError::new(format!(
                    "snapshot repository root must be absolute: {}",
                    repository.root
                )));
            }
            if !repository_names.insert(repository.name.as_str()) {
                return Err(DeliveryError::new(format!(
                    "duplicate snapshot repository {}",
                    repository.name
                )));
            }
            if !repository_roots.insert(repository.root.as_str()) {
                return Err(DeliveryError::new(format!(
                    "duplicate snapshot repository root {}",
                    repository.root
                )));
            }
        }
        if !self
            .repository_set
            .windows(2)
            .all(|pair| pair[0].name < pair[1].name)
        {
            return Err(DeliveryError::new(
                "snapshot repository_set must be sorted by repository name",
            ));
        }
        let root_record = self
            .repository_set
            .iter()
            .find(|repository| repository.name == self.root_repository.name)
            .ok_or_else(|| {
                DeliveryError::new("root repository is absent from snapshot repository_set")
            })?;
        if root_record.root != self.root_repository.root
            || root_record.head_commit != self.root_repository.head_commit
            || root_record.tree_hash != self.root_repository.tree_hash
        {
            return Err(DeliveryError::new(
                "root repository does not match its repository_set record",
            ));
        }

        validate_snapshot_stack(&self.stack, &repository_names)?;
        validate_required_validations(&self.required_validations)?;
        validate_snapshot_checks(&self.required_checks, &self.stack)?;
        validate_fingerprints(
            "generated_artifacts",
            &self.generated_artifacts,
            &repository_names,
        )?;
        validate_fingerprints(
            "dependency_fingerprints",
            &self.dependency_fingerprints,
            &repository_names,
        )?;
        validate_fingerprints(
            "contract_fingerprints",
            &self.contract_fingerprints,
            &repository_names,
        )?;
        Ok(())
    }

    pub fn repository_bindings(&self) -> Vec<RepositoryTreeBinding> {
        self.repository_set
            .iter()
            .map(|repository| RepositoryTreeBinding {
                name: repository.name.clone(),
                tree_hash: repository.tree_hash.clone(),
            })
            .collect()
    }

    pub fn content_identity(&self) -> ContentIdentity {
        ContentIdentity {
            integrated_tree: self.root_repository.tree_hash.clone(),
            repository_set: self.repository_bindings(),
            generated_artifacts: self.generated_artifacts.clone(),
            dependency_fingerprints: self.dependency_fingerprints.clone(),
            contract_fingerprints: self.contract_fingerprints.clone(),
            required_validations: self.required_validations.clone(),
            required_checks: self.required_checks.clone(),
        }
    }

    pub fn stack_order_is_unambiguous(&self) -> Result<bool> {
        Ok(graph_order(&self.stack)?.unique)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RootRepository {
    pub name: String,
    pub root: String,
    pub base_commit: String,
    pub head_commit: String,
    pub tree_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryRecord {
    pub name: String,
    pub root: String,
    pub head_commit: String,
    pub tree_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackNode {
    pub id: String,
    pub repository: String,
    pub branch: String,
    pub pr: Option<u64>,
    pub head_commit: String,
    pub depends_on: Vec<String>,
}

impl GraphNode for StackNode {
    fn id(&self) -> &str {
        &self.id
    }

    fn dependencies(&self) -> &[String] {
        &self.depends_on
    }
}

impl GraphNode for StackNodeSpec {
    fn id(&self) -> &str {
        &self.id
    }

    fn dependencies(&self) -> &[String] {
        &self.depends_on
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredValidation {
    pub id: String,
    pub command_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub name: String,
    pub repository: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryTreeBinding {
    pub name: String,
    pub tree_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContentIdentity {
    pub integrated_tree: String,
    pub repository_set: Vec<RepositoryTreeBinding>,
    pub generated_artifacts: Vec<Fingerprint>,
    pub dependency_fingerprints: Vec<Fingerprint>,
    pub contract_fingerprints: Vec<Fingerprint>,
    pub required_validations: Vec<RequiredValidation>,
    pub required_checks: Vec<RequiredCheck>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelRole {
    Software,
    Test,
    Nixos,
    Networking,
    Security,
    Rust,
    Product,
    Docs,
    Observability,
    Kernel,
}

impl PanelRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Software => "software",
            Self::Test => "test",
            Self::Nixos => "nixos",
            Self::Networking => "networking",
            Self::Security => "security",
            Self::Rust => "rust",
            Self::Product => "product",
            Self::Docs => "docs",
            Self::Observability => "observability",
            Self::Kernel => "kernel",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceResultClass {
    Passed,
    Failed,
    Pending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GraphOrder {
    unique: bool,
}

trait GraphNode {
    fn id(&self) -> &str;
    fn dependencies(&self) -> &[String];
}

fn graph_order<T: GraphNode>(nodes: &[T]) -> Result<GraphOrder> {
    let mut indegree = nodes
        .iter()
        .map(|node| (node.id(), node.dependencies().len()))
        .collect::<BTreeMap<_, _>>();
    let mut dependants: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in nodes {
        for dependency in node.dependencies() {
            if !indegree.contains_key(dependency.as_str()) {
                return Err(DeliveryError::new(format!(
                    "stack node {} references unknown dependency {}",
                    node.id(),
                    dependency
                )));
            }
            dependants
                .entry(dependency.as_str())
                .or_default()
                .push(node.id());
        }
    }

    let mut ready = indegree
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(*id))
        .collect::<VecDeque<_>>();
    let mut visited = 0;
    let mut unique = true;
    while !ready.is_empty() {
        if ready.len() != 1 {
            unique = false;
        }
        let id = ready.pop_front().expect("ready is not empty");
        visited += 1;
        for dependant in dependants.get(id).into_iter().flatten() {
            let count = indegree
                .get_mut(dependant)
                .expect("dependant was collected from known nodes");
            *count -= 1;
            if *count == 0 {
                ready.push_back(dependant);
            }
        }
    }
    if visited != nodes.len() {
        return Err(DeliveryError::new(
            "stack dependency graph contains a cycle",
        ));
    }
    Ok(GraphOrder { unique })
}

fn validate_snapshot_stack(stack: &[StackNode], repositories: &BTreeSet<&str>) -> Result<()> {
    if stack.is_empty() {
        return Err(DeliveryError::new("snapshot stack must not be empty"));
    }
    let mut ids = BTreeSet::new();
    let mut branches = BTreeSet::new();
    let mut pull_requests = BTreeSet::new();
    for node in stack {
        validate_identifier(&node.id, "stack node id")?;
        if !ids.insert(node.id.as_str()) {
            return Err(DeliveryError::new(format!(
                "duplicate snapshot stack node {}",
                node.id
            )));
        }
        if !repositories.contains(node.repository.as_str()) {
            return Err(DeliveryError::new(format!(
                "snapshot stack node {} references unknown repository {}",
                node.id, node.repository
            )));
        }
        nonempty(&node.branch, "stack branch")?;
        validate_hash(&node.head_commit, "stack head")?;
        if !branches.insert((node.repository.as_str(), node.branch.as_str())) {
            return Err(DeliveryError::new(format!(
                "duplicate snapshot branch {} in {}",
                node.branch, node.repository
            )));
        }
        if let Some(pr) = node.pr
            && (!pull_requests.insert((node.repository.as_str(), pr)) || pr == 0)
        {
            return Err(DeliveryError::new(format!(
                "invalid or duplicate snapshot PR {pr} in {}",
                node.repository
            )));
        }
        let mut dependencies = BTreeSet::new();
        for dependency in &node.depends_on {
            if !dependencies.insert(dependency.as_str()) {
                return Err(DeliveryError::new(format!(
                    "snapshot stack node {} repeats dependency {}",
                    node.id, dependency
                )));
            }
        }
    }
    let indices = stack
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    graph_order(stack)?;
    for (index, node) in stack.iter().enumerate() {
        for dependency in &node.depends_on {
            let Some(dependency_index) = indices.get(dependency.as_str()) else {
                return Err(DeliveryError::new(format!(
                    "snapshot stack node {} references unknown dependency {}",
                    node.id, dependency
                )));
            };
            if *dependency_index >= index {
                return Err(DeliveryError::new(format!(
                    "snapshot stack is not dependency ordered: {} must precede {}",
                    dependency, node.id
                )));
            }
        }
    }
    Ok(())
}

fn validate_required_validations(validations: &[RequiredValidation]) -> Result<()> {
    if validations.is_empty() {
        return Err(DeliveryError::new(
            "snapshot required_validations must not be empty",
        ));
    }
    let mut ids = BTreeSet::new();
    for validation in validations {
        validate_identifier(&validation.id, "validation id")?;
        validate_sha256(&validation.command_sha256, "validation command digest")?;
        if !ids.insert(validation.id.as_str()) {
            return Err(DeliveryError::new(format!(
                "duplicate snapshot validation {}",
                validation.id
            )));
        }
    }
    if !validations.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(DeliveryError::new(
            "snapshot required_validations must be sorted",
        ));
    }
    Ok(())
}

fn validate_snapshot_checks(checks: &[RequiredCheck], stack: &[StackNode]) -> Result<()> {
    if checks.is_empty() {
        return Err(DeliveryError::new(
            "snapshot required_checks must not be empty",
        ));
    }
    let nodes = stack
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut unique = BTreeSet::new();
    let mut covered = BTreeSet::new();
    for check in checks {
        if !nodes.contains(check.node.as_str()) {
            return Err(DeliveryError::new(format!(
                "snapshot check {} references unknown node {}",
                check.name, check.node
            )));
        }
        nonempty(&check.name, "required check name")?;
        if !unique.insert((check.node.as_str(), check.name.as_str())) {
            return Err(DeliveryError::new(format!(
                "duplicate snapshot check {} for {}",
                check.name, check.node
            )));
        }
        covered.insert(check.node.as_str());
    }
    if covered.len() != nodes.len() {
        return Err(DeliveryError::new(
            "every snapshot stack node must have a required check",
        ));
    }
    if !checks.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(DeliveryError::new(
            "snapshot required_checks must be sorted",
        ));
    }
    Ok(())
}

fn validate_fingerprint_specs(
    label: &str,
    fingerprints: &[FingerprintSpec],
    repositories: &BTreeSet<&str>,
) -> Result<()> {
    let mut names = BTreeSet::new();
    for fingerprint in fingerprints {
        validate_identifier(&fingerprint.name, "fingerprint name")?;
        if !repositories.contains(fingerprint.repository.as_str()) {
            return Err(DeliveryError::new(format!(
                "{label} entry {} references unknown repository {}",
                fingerprint.name, fingerprint.repository
            )));
        }
        validate_repo_relative_path(&fingerprint.path)?;
        if !names.insert(fingerprint.name.as_str()) {
            return Err(DeliveryError::new(format!(
                "duplicate {label} name {}",
                fingerprint.name
            )));
        }
    }
    Ok(())
}

fn validate_fingerprints(
    label: &str,
    fingerprints: &[Fingerprint],
    repositories: &BTreeSet<&str>,
) -> Result<()> {
    let mut names = BTreeSet::new();
    for fingerprint in fingerprints {
        validate_identifier(&fingerprint.name, "fingerprint name")?;
        if !repositories.contains(fingerprint.repository.as_str()) {
            return Err(DeliveryError::new(format!(
                "{label} entry {} references unknown repository {}",
                fingerprint.name, fingerprint.repository
            )));
        }
        validate_repo_relative_path(Path::new(&fingerprint.path))?;
        validate_sha256(&fingerprint.sha256, "fingerprint digest")?;
        if !names.insert(fingerprint.name.as_str()) {
            return Err(DeliveryError::new(format!(
                "duplicate {label} name {}",
                fingerprint.name
            )));
        }
    }
    if !fingerprints.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(DeliveryError::new(format!(
            "snapshot {label} must be sorted"
        )));
    }
    Ok(())
}

pub fn validate_repo_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(DeliveryError::new(format!(
            "path must be repository-relative without traversal: {}",
            path.display()
        )));
    }
    Ok(())
}

pub fn validate_hash(value: &str, label: &str) -> Result<()> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DeliveryError::new(format!(
            "{label} must be a full lowercase Git object hash"
        )));
    }
    Ok(())
}

pub fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DeliveryError::new(format!(
            "{label} must be a lowercase SHA-256 digest"
        )));
    }
    Ok(())
}

pub fn validate_identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
        || !value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(DeliveryError::new(format!(
            "{label} must use lowercase ASCII letters, digits, '.', '_' or '-'"
        )));
    }
    Ok(())
}

pub fn ensure_schema(version: u32, label: &str) -> Result<()> {
    if version != DELIVERY_SCHEMA_VERSION {
        return Err(DeliveryError::new(format!(
            "unsupported {label} schema version {version}"
        )));
    }
    Ok(())
}

fn validate_repository_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 200
        || name.starts_with('/')
        || name.ends_with('/')
        || name.contains(char::is_whitespace)
        || name.contains("..")
    {
        return Err(DeliveryError::new(format!(
            "invalid repository identity {name:?}"
        )));
    }
    Ok(())
}

fn nonempty(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(DeliveryError::new(format!("{label} must not be empty")))
    } else {
        Ok(())
    }
}

fn normalized_path_key(path: &Path) -> Result<String> {
    if path.as_os_str().is_empty() {
        return Err(DeliveryError::new("repository root must not be empty"));
    }
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> StackManifest {
        StackManifest {
            schema_version: DELIVERY_SCHEMA_VERSION,
            wave: "w1".to_owned(),
            root_repository: RootRepositorySpec {
                name: "example/d2b".to_owned(),
                root: PathBuf::from("/workspace/d2b"),
                base: "main".to_owned(),
                head: "feature".to_owned(),
            },
            repository_set: vec![RepositorySpec {
                name: "example/d2b".to_owned(),
                root: PathBuf::from("/workspace/d2b"),
                head: "feature".to_owned(),
            }],
            stack: vec![
                StackNodeSpec {
                    id: "root".to_owned(),
                    repository: "example/d2b".to_owned(),
                    branch: "feature-root".to_owned(),
                    pr: Some(1),
                    head: "feature-root".to_owned(),
                    depends_on: vec![],
                },
                StackNodeSpec {
                    id: "leaf".to_owned(),
                    repository: "example/d2b".to_owned(),
                    branch: "feature-leaf".to_owned(),
                    pr: Some(2),
                    head: "feature-leaf".to_owned(),
                    depends_on: vec!["root".to_owned()],
                },
            ],
            required_validations: vec![RequiredValidationSpec {
                id: "unit".to_owned(),
                command: "cargo test -p xtask".to_owned(),
            }],
            required_checks: vec![
                RequiredCheck {
                    node: "root".to_owned(),
                    name: "unit".to_owned(),
                },
                RequiredCheck {
                    node: "leaf".to_owned(),
                    name: "unit".to_owned(),
                },
            ],
            generated_artifacts: vec![],
            dependency_fingerprints: vec![],
            contract_fingerprints: vec![],
        }
    }

    #[test]
    fn accepts_ordered_linear_stack() {
        manifest().validate().expect("valid manifest");
    }

    #[test]
    fn rejects_unknown_dependency() {
        let mut manifest = manifest();
        manifest.stack[1].depends_on = vec!["missing".to_owned()];
        let error = manifest.validate().expect_err("unknown dependency");
        assert!(error.to_string().contains("unknown dependency"));
    }

    #[test]
    fn rejects_cycle() {
        let nodes = vec![
            StackNode {
                id: "one".to_owned(),
                repository: "example/d2b".to_owned(),
                branch: "one".to_owned(),
                pr: Some(1),
                head_commit: "a".repeat(40),
                depends_on: vec!["two".to_owned()],
            },
            StackNode {
                id: "two".to_owned(),
                repository: "example/d2b".to_owned(),
                branch: "two".to_owned(),
                pr: Some(2),
                head_commit: "b".repeat(40),
                depends_on: vec!["one".to_owned()],
            },
        ];
        let error = graph_order(&nodes).expect_err("cycle");
        assert!(error.to_string().contains("cycle"));
    }

    #[test]
    fn manifest_cycle_is_rejected_as_a_cycle() {
        let mut manifest = manifest();
        manifest.stack[0].depends_on = vec!["leaf".to_owned()];
        let error = manifest.validate().expect_err("cycle");
        assert!(error.to_string().contains("cycle"));
    }

    #[test]
    fn detects_ambiguous_stack_order() {
        let nodes = vec![
            StackNode {
                id: "one".to_owned(),
                repository: "example/d2b".to_owned(),
                branch: "one".to_owned(),
                pr: Some(1),
                head_commit: "a".repeat(40),
                depends_on: vec![],
            },
            StackNode {
                id: "two".to_owned(),
                repository: "example/d2b".to_owned(),
                branch: "two".to_owned(),
                pr: Some(2),
                head_commit: "b".repeat(40),
                depends_on: vec![],
            },
        ];
        assert!(!graph_order(&nodes).expect("DAG").unique);
    }

    #[test]
    fn serde_rejects_unknown_manifest_fields() {
        let mut value = serde_json::to_value(manifest()).expect("serialize");
        value
            .as_object_mut()
            .expect("object")
            .insert("model".to_owned(), serde_json::json!("not-source-metadata"));
        let error = serde_json::from_value::<StackManifest>(value).expect_err("unknown field");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn rejects_duplicate_branch_and_pr() {
        let mut manifest = manifest();
        manifest.stack[1].branch = manifest.stack[0].branch.clone();
        manifest.stack[1].pr = manifest.stack[0].pr;
        let error = manifest.validate().expect_err("duplicates");
        assert!(error.to_string().contains("duplicate branch"));
    }

    #[test]
    fn rejects_repository_path_traversal() {
        let error =
            validate_repo_relative_path(Path::new("../secret")).expect_err("parent traversal");
        assert!(error.to_string().contains("repository-relative"));
    }
}
