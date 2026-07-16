use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{DELIVERY_SCHEMA_VERSION, DeliveryError, Result};

pub const SNAPSHOT_ARTIFACT_KIND: &str = "d2b-delivery/wave-snapshot";
pub const SEAL_ARTIFACT_KIND: &str = "d2b-delivery/wave-seal";
pub const HISTORY_PROOF_ARTIFACT_KIND: &str = "d2b-delivery/history-proof";
pub const PANEL_REQUEST_ARTIFACT_KIND: &str = "d2b-delivery/panel-request";
pub const PANEL_ATTESTATION_ARTIFACT_KIND: &str = "d2b-delivery/panel-receipt";
pub const EVIDENCE_ARTIFACT_KIND: &str = "d2b-delivery/validation-evidence";
pub const PANEL_PROVIDER_POLICY: &str = "github-copilot";
pub const PANEL_MODEL_POLICY: &str = "gemini-3.1-pro-preview";
pub const PANEL_SIGNATURE_POLICY: &str = "rsa-sha256";
pub const LEGACY_AUTHORITATIVE_MANIFEST_PATH: &str = "delivery/manifest.json";
pub const WAVE_MANIFEST_DIRECTORY: &str = "delivery/manifests";

pub const MAX_REPOSITORIES: usize = 16;
pub const MAX_STACK_NODES: usize = 128;
pub const MAX_VALIDATIONS: usize = 128;
pub const MAX_CHECKS: usize = 512;
pub const MAX_FINGERPRINTS: usize = 512;
pub const MAX_ARGUMENTS: usize = 128;
pub const MAX_STRING_BYTES: usize = 4 * 1024;
pub const MAX_RECOMMENDATIONS: usize = 128;

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

#[derive(Clone, Debug)]
pub struct SnapshotRequest {
    pub authority_repository: String,
    pub authority_ref: String,
    pub manifest_path: PathBuf,
    pub repository_roots: BTreeMap<String, PathBuf>,
    pub state_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryManifest {
    pub schema_version: u32,
    pub program: String,
    pub wave: String,
    pub authority_repository: String,
    pub panel_trust_root_sha256: String,
    pub repositories: Vec<RepositoryPolicy>,
    pub stack_nodes: Vec<StackNodePolicy>,
    pub required_validations: Vec<RequiredValidation>,
    pub required_checks: Vec<RequiredCheck>,
    #[serde(default)]
    pub generated_artifacts: Vec<FingerprintSpec>,
    #[serde(default)]
    pub dependency_fingerprints: Vec<FingerprintSpec>,
    #[serde(default)]
    pub contract_fingerprints: Vec<FingerprintSpec>,
}

impl DeliveryManifest {
    pub fn validate(&self) -> Result<()> {
        ensure_schema(self.schema_version, "delivery manifest")?;
        validate_identifier(&self.program, "program")?;
        validate_wave_identifier(&self.wave)?;
        validate_repository_id(&self.authority_repository)?;
        validate_sha256(&self.panel_trust_root_sha256, "panel trust-root digest")?;
        ensure_count(self.repositories.len(), 1, MAX_REPOSITORIES, "repositories")?;
        ensure_count(self.stack_nodes.len(), 1, MAX_STACK_NODES, "stack_nodes")?;
        ensure_count(
            self.required_validations.len(),
            1,
            MAX_VALIDATIONS,
            "required_validations",
        )?;
        ensure_count(self.required_checks.len(), 1, MAX_CHECKS, "required_checks")?;

        let mut repository_ids = BTreeSet::new();
        for repository in &self.repositories {
            repository.validate()?;
            if !repository_ids.insert(repository.id.as_str()) {
                return Err(DeliveryError::new(format!(
                    "duplicate repository {}",
                    repository.id
                )));
            }
        }
        if !repository_ids.contains(self.authority_repository.as_str()) {
            return Err(DeliveryError::new(
                "authority_repository is absent from repositories",
            ));
        }

        let mut node_ids = BTreeSet::new();
        let mut branches = BTreeSet::new();
        let mut pull_requests = BTreeSet::new();
        for node in &self.stack_nodes {
            node.validate(&repository_ids)?;
            if !node_ids.insert(node.id.as_str()) {
                return Err(DeliveryError::new(format!(
                    "duplicate stack node id {}",
                    node.id
                )));
            }
            if !branches.insert((node.repository.as_str(), node.branch.as_str())) {
                return Err(DeliveryError::new(format!(
                    "duplicate configured stack branch {} in {}",
                    node.branch, node.repository
                )));
            }
            if !pull_requests.insert((node.repository.as_str(), node.pr_number)) {
                return Err(DeliveryError::new(format!(
                    "duplicate configured PR {} in {}",
                    node.pr_number, node.repository
                )));
            }
        }
        for node in &self.stack_nodes {
            let mut dependencies = BTreeSet::new();
            for dependency in &node.external_dependencies {
                if dependency == &node.id || !node_ids.contains(dependency.as_str()) {
                    return Err(DeliveryError::new(format!(
                        "stack node {} has unknown or self dependency {}",
                        node.id, dependency
                    )));
                }
                if !dependencies.insert(dependency.as_str()) {
                    return Err(DeliveryError::new(format!(
                        "stack node {} repeats dependency {}",
                        node.id, dependency
                    )));
                }
            }
        }

        let mut validation_ids = BTreeSet::new();
        for validation in &self.required_validations {
            validation.validate(&repository_ids)?;
            if !validation_ids.insert(validation.id.as_str()) {
                return Err(DeliveryError::new(format!(
                    "duplicate required validation {}",
                    validation.id
                )));
            }
        }

        let mut check_keys = BTreeSet::new();
        let mut checked_nodes = BTreeSet::new();
        for check in &self.required_checks {
            check.validate(&node_ids)?;
            if !check_keys.insert((check.node.as_str(), check.name.as_str())) {
                return Err(DeliveryError::new(format!(
                    "duplicate required check name {} for {}",
                    check.name, check.node
                )));
            }
            checked_nodes.insert(check.node.as_str());
        }
        if checked_nodes != node_ids {
            return Err(DeliveryError::new(
                "every configured stack node must have at least one required check",
            ));
        }

        let fingerprint_count = self.generated_artifacts.len()
            + self.dependency_fingerprints.len()
            + self.contract_fingerprints.len();
        ensure_count(
            fingerprint_count,
            1,
            MAX_FINGERPRINTS,
            "authoritative fingerprint matrix",
        )?;
        ensure_count(
            self.dependency_fingerprints.len(),
            1,
            MAX_FINGERPRINTS,
            "dependency_fingerprints",
        )?;
        ensure_count(
            self.contract_fingerprints.len(),
            1,
            MAX_FINGERPRINTS,
            "contract_fingerprints",
        )?;
        validate_fingerprint_specs(
            "generated_artifacts",
            &self.generated_artifacts,
            &repository_ids,
        )?;
        validate_fingerprint_specs(
            "dependency_fingerprints",
            &self.dependency_fingerprints,
            &repository_ids,
        )?;
        validate_fingerprint_specs(
            "contract_fingerprints",
            &self.contract_fingerprints,
            &repository_ids,
        )?;
        Ok(())
    }

    pub fn repository(&self, id: &str) -> Option<&RepositoryPolicy> {
        self.repositories
            .iter()
            .find(|repository| repository.id == id)
    }
}

pub fn validate_wave_identifier(wave: &str) -> Result<()> {
    let Some(number) = wave.strip_prefix('w') else {
        return Err(DeliveryError::new(
            "delivery wave must use the canonical w<N> identifier",
        ));
    };
    if number.is_empty()
        || number.starts_with('0')
        || !number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(DeliveryError::new(
            "delivery wave must use the canonical w<N> identifier",
        ));
    }
    validate_identifier(wave, "wave")
}

pub fn is_authoritative_manifest_path(path: &Path) -> bool {
    if path == Path::new(LEGACY_AUTHORITATIVE_MANIFEST_PATH) {
        return true;
    }
    let Some(file_name) = path
        .strip_prefix(WAVE_MANIFEST_DIRECTORY)
        .ok()
        .and_then(|relative| {
            let mut components = relative.components();
            let file = components.next()?;
            components.next().is_none().then_some(file)
        })
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
    else {
        return false;
    };
    let Some(wave) = file_name.strip_suffix(".json") else {
        return false;
    };
    validate_wave_identifier(wave).is_ok()
}

pub fn expected_wave_manifest_path(wave: &str) -> Result<PathBuf> {
    validate_wave_identifier(wave)?;
    Ok(Path::new(WAVE_MANIFEST_DIRECTORY).join(format!("{wave}.json")))
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryPolicy {
    pub id: String,
    pub object_format: GitObjectFormat,
    pub trunk_ref: String,
    pub integration_ref: String,
}

impl RepositoryPolicy {
    fn validate(&self) -> Result<()> {
        validate_repository_id(&self.id)?;
        validate_git_ref(&self.trunk_ref, "trunk ref")?;
        validate_git_ref(&self.integration_ref, "integration ref")?;
        if self.trunk_ref == self.integration_ref {
            return Err(DeliveryError::new(format!(
                "repository {} has identical trunk and integration refs",
                self.id
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackNodePolicy {
    pub id: String,
    pub repository: String,
    pub branch: String,
    pub pr_number: u64,
    #[serde(default)]
    pub external_dependencies: Vec<String>,
}

impl StackNodePolicy {
    fn validate(&self, repositories: &BTreeSet<&str>) -> Result<()> {
        validate_identifier(&self.id, "stack node id")?;
        if !repositories.contains(self.repository.as_str()) {
            return Err(DeliveryError::new(format!(
                "stack node {} references unknown repository {}",
                self.id, self.repository
            )));
        }
        validate_git_ref(&self.branch, "stack branch")?;
        if self.pr_number == 0 {
            return Err(DeliveryError::new(format!(
                "stack node {} has invalid PR number 0",
                self.id
            )));
        }
        ensure_count(
            self.external_dependencies.len(),
            0,
            MAX_STACK_NODES,
            "external_dependencies",
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalPath {
    pub repository: String,
    pub path: String,
}

impl LogicalPath {
    fn validate(&self, repositories: &BTreeSet<&str>) -> Result<()> {
        if !repositories.contains(self.repository.as_str()) {
            return Err(DeliveryError::new(format!(
                "logical path references unknown repository {}",
                self.repository
            )));
        }
        validate_repo_relative_path(Path::new(&self.path))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationAuthority {
    LocalRunner,
    GithubAttestation,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredValidation {
    pub id: String,
    pub argv: Vec<String>,
    pub cwd: LogicalPath,
    pub authority: ValidationAuthority,
    pub ci_publisher: Option<CheckPublisher>,
    pub ci_signer_workflow: Option<String>,
    pub timeout_seconds: u64,
}

impl RequiredValidation {
    fn validate(&self, repositories: &BTreeSet<&str>) -> Result<()> {
        validate_identifier(&self.id, "validation id")?;
        ensure_count(self.argv.len(), 1, MAX_ARGUMENTS, "validation argv")?;
        for argument in &self.argv {
            validate_bounded_string(argument, "validation argument")?;
            if argument.contains('\0') {
                return Err(DeliveryError::new(
                    "validation argument contains a NUL byte",
                ));
            }
        }
        self.cwd.validate(repositories)?;
        match (self.authority, &self.ci_publisher, &self.ci_signer_workflow) {
            (ValidationAuthority::LocalRunner, None, None) => {}
            (ValidationAuthority::GithubAttestation, Some(publisher), Some(signer_workflow))
                if publisher.kind == CheckPublisherKind::CheckRun && publisher.workflow_id != 0 =>
            {
                publisher.validate()?;
                validate_bounded_string(signer_workflow, "CI signer workflow")?;
                if !signer_workflow.starts_with("github.com/")
                    || !(signer_workflow.ends_with(".yml") || signer_workflow.ends_with(".yaml"))
                {
                    return Err(DeliveryError::new(format!(
                        "validation {} CI signer workflow must be a full github.com workflow path",
                        self.id
                    )));
                }
            }
            (ValidationAuthority::GithubAttestation, Some(_), Some(_)) => {
                return Err(DeliveryError::new(format!(
                    "validation {} CI publisher must be a check run",
                    self.id
                )));
            }
            _ => {
                return Err(DeliveryError::new(format!(
                    "validation {} must declare CI publisher exactly when GitHub attested",
                    self.id
                )));
            }
        }
        if self.timeout_seconds == 0 || self.timeout_seconds > 24 * 60 * 60 {
            return Err(DeliveryError::new(format!(
                "validation {} timeout must be between 1 and 86400 seconds",
                self.id
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckPublisher {
    pub kind: CheckPublisherKind,
    pub app_slug: String,
    pub app_id: u64,
    pub workflow: String,
    pub workflow_id: u64,
}

impl CheckPublisher {
    pub(crate) fn validate(&self) -> Result<()> {
        validate_bounded_string(&self.app_slug, "check publisher app_slug")?;
        validate_bounded_string(&self.workflow, "check publisher workflow")?;
        match self.kind {
            CheckPublisherKind::CheckRun if self.app_id == 0 => {
                return Err(DeliveryError::new(
                    "check-run publisher app_id must be non-zero",
                ));
            }
            CheckPublisherKind::CheckRun
                if (self.workflow == "none") != (self.workflow_id == 0) =>
            {
                return Err(DeliveryError::new(
                    "check-run workflow_id must be zero exactly when workflow is 'none'",
                ));
            }
            CheckPublisherKind::StatusContext if self.app_id != 0 || self.workflow_id != 0 => {
                return Err(DeliveryError::new(
                    "status-context publisher app_id and workflow_id must be zero",
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckPublisherKind {
    CheckRun,
    StatusContext,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredCheck {
    pub node: String,
    pub name: String,
    pub publisher: CheckPublisher,
}

impl RequiredCheck {
    fn validate(&self, nodes: &BTreeSet<&str>) -> Result<()> {
        if !nodes.contains(self.node.as_str()) {
            return Err(DeliveryError::new(format!(
                "required check {} references unknown stack node {}",
                self.name, self.node
            )));
        }
        validate_bounded_string(&self.name, "required check name")?;
        self.publisher.validate()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FingerprintSpec {
    pub name: String,
    pub repository: String,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackGraph {
    pub trunk: String,
    pub current_branch: String,
    pub branches: Vec<StackBranch>,
}

impl StackGraph {
    pub fn validate(&self) -> Result<()> {
        validate_git_ref(&self.trunk, "Git Town trunk")?;
        validate_bounded_string(&self.current_branch, "Git Town current branch")?;
        ensure_count(
            self.branches.len(),
            1,
            MAX_STACK_NODES,
            "Git Town stack branches",
        )?;
        let mut names = BTreeSet::new();
        let mut prs = BTreeSet::new();
        let mut current = 0;
        let mut saw_active = false;
        for branch in &self.branches {
            branch.validate()?;
            if !names.insert(branch.name.as_str()) {
                return Err(DeliveryError::new(format!(
                    "Git Town stack repeats branch {}",
                    branch.name
                )));
            }
            if branch.is_current {
                if branch.is_merged {
                    return Err(DeliveryError::new("Git Town current branch must be active"));
                }
                current += 1;
                if branch.name != self.current_branch {
                    return Err(DeliveryError::new(
                        "Git Town current branch identity is inconsistent",
                    ));
                }
            }
            if branch.is_merged {
                if saw_active {
                    return Err(DeliveryError::new(
                        "Git Town stack has a merged node after an active node",
                    ));
                }
            } else {
                saw_active = true;
            }
            if let Some(pr) = &branch.pr
                && !prs.insert(pr.number)
            {
                return Err(DeliveryError::new(format!(
                    "Git Town stack repeats PR {}",
                    pr.number
                )));
            }
        }
        if current != 1 {
            return Err(DeliveryError::new(
                "Git Town stack must identify exactly one current branch",
            ));
        }
        if !self.branches.iter().any(|branch| !branch.is_merged) {
            return Err(DeliveryError::new(
                "Git Town stack must contain at least one active branch",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackBranch {
    pub name: String,
    pub parent: String,
    #[serde(rename = "baseRef")]
    pub base_ref: String,
    #[serde(rename = "observedBase")]
    pub observed_base: String,
    pub head: String,
    pub base: String,
    #[serde(rename = "isCurrent")]
    pub is_current: bool,
    #[serde(rename = "isMerged")]
    pub is_merged: bool,
    #[serde(rename = "isQueued")]
    pub is_queued: bool,
    #[serde(rename = "needsRebase")]
    pub needs_rebase: bool,
    pub pr: Option<StackPr>,
    #[serde(rename = "mergeCommitOid")]
    pub merge_commit_oid: Option<String>,
    #[serde(rename = "mergeCommitTreeOid")]
    pub merge_commit_tree_oid: Option<String>,
}

impl StackBranch {
    fn validate(&self) -> Result<()> {
        validate_git_ref(&self.name, "Git Town branch")?;
        validate_git_ref(&self.parent, "Git Town parent")?;
        validate_git_ref(&self.base_ref, "GitHub PR base ref")?;
        validate_hash(&self.observed_base, "GitHub PR observed base")?;
        validate_hash(&self.head, "Git Town branch head")?;
        validate_hash(&self.base, "Git Town branch base")?;
        if self.is_queued || self.needs_rebase {
            return Err(DeliveryError::new(format!(
                "Git Town branch {} is queued or needs rebase",
                self.name
            )));
        }
        let pr = self.pr.as_ref().ok_or_else(|| {
            DeliveryError::new(format!("Git Town branch {} has no PR", self.name))
        })?;
        pr.validate()?;
        let expected = if self.is_merged { "MERGED" } else { "OPEN" };
        if pr.state != expected {
            return Err(DeliveryError::new(format!(
                "Git Town branch {} state disagrees with PR state",
                self.name
            )));
        }
        match (
            self.is_merged,
            &self.merge_commit_oid,
            &self.merge_commit_tree_oid,
        ) {
            (true, Some(commit), Some(tree)) => {
                validate_hash(commit, "GitHub merge commit OID")?;
                validate_hash(tree, "GitHub merge commit tree OID")?;
            }
            (false, None, None) => {}
            _ => {
                return Err(DeliveryError::new(format!(
                    "Git Town branch {} has inconsistent merge authority",
                    self.name
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackPr {
    pub number: u64,
    #[serde(default)]
    pub url: String,
    pub state: String,
}

impl StackPr {
    fn validate(&self) -> Result<()> {
        if self.number == 0 {
            return Err(DeliveryError::new("Git Town stack PR number must not be 0"));
        }
        if !matches!(self.state.as_str(), "OPEN" | "MERGED") {
            return Err(DeliveryError::new(format!(
                "unsupported Git Town stack PR state {}",
                self.state
            )));
        }
        validate_optional_bounded_string(&self.url, "Git Town stack PR URL")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WaveSnapshot {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub program: String,
    pub wave: String,
    pub candidate_id: String,
    pub content_id: String,
    pub panel_trust_root_sha256: String,
    pub authority: AuthorityBinding,
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
        if self.artifact_kind != SNAPSHOT_ARTIFACT_KIND {
            return Err(DeliveryError::new("invalid wave snapshot artifact_kind"));
        }
        ensure_schema(self.schema_version, "wave snapshot")?;
        validate_identifier(&self.program, "program")?;
        validate_identifier(&self.wave, "wave")?;
        validate_sha256(&self.candidate_id, "candidate ID")?;
        validate_sha256(&self.content_id, "content ID")?;
        validate_sha256(
            &self.panel_trust_root_sha256,
            "snapshot panel trust-root digest",
        )?;
        self.authority.validate()?;
        ensure_sorted_unique_by(
            &self.repository_set,
            |repository| repository.id.as_str(),
            "snapshot repository_set",
        )?;
        ensure_count(
            self.repository_set.len(),
            1,
            MAX_REPOSITORIES,
            "snapshot repository_set",
        )?;
        let repository_ids = self
            .repository_set
            .iter()
            .map(|repository| {
                repository.validate()?;
                Ok(repository.id.as_str())
            })
            .collect::<Result<BTreeSet<_>>>()?;
        if !repository_ids.contains(self.authority.repository.as_str()) {
            return Err(DeliveryError::new(
                "snapshot authority repository is absent from repository_set",
            ));
        }
        let authority_repository = self
            .repository_set
            .iter()
            .find(|repository| repository.id == self.authority.repository)
            .expect("authority repository membership was checked");
        if authority_repository.integration_ref != self.authority.ref_name
            || authority_repository.integration_oid != self.authority.commit_oid
            || authority_repository.integration_tree_oid != self.authority.tree_oid
        {
            return Err(DeliveryError::new(
                "snapshot authority must be the exact integration head of its repository",
            ));
        }

        ensure_count(self.stack.len(), 1, MAX_STACK_NODES, "snapshot stack")?;
        let mut node_ids = BTreeSet::new();
        let mut branch_keys = BTreeSet::new();
        let mut pr_keys = BTreeSet::new();
        for node in &self.stack {
            node.validate(&repository_ids)?;
            if !node_ids.insert(node.id.as_str())
                || !branch_keys.insert((node.repository.as_str(), node.head_ref.as_str()))
                || !pr_keys.insert((node.repository.as_str(), node.pr_number))
            {
                return Err(DeliveryError::new(
                    "snapshot stack repeats an id, branch, or PR",
                ));
            }
        }
        graph_order(&self.stack)?;
        for repository in &self.repository_set {
            let terminal = self
                .stack
                .iter()
                .rfind(|node| {
                    node.repository == repository.id
                        && node.snapshot_state == PullRequestState::Open
                })
                .ok_or_else(|| {
                    DeliveryError::new(format!(
                        "repository {} has no active snapshot stack nodes",
                        repository.id
                    ))
                })?;
            if terminal.head_ref != repository.integration_ref
                || terminal.head_oid != repository.integration_oid
                || terminal.head_tree_oid != repository.integration_tree_oid
            {
                return Err(DeliveryError::new(format!(
                    "repository {} terminal stack node is not its integration head",
                    repository.id
                )));
            }
        }

        ensure_sorted_unique_by(
            &self.required_validations,
            |validation| validation.id.as_str(),
            "snapshot required_validations",
        )?;
        ensure_count(
            self.required_validations.len(),
            1,
            MAX_VALIDATIONS,
            "snapshot required_validations",
        )?;
        for validation in &self.required_validations {
            validation.validate(&repository_ids)?;
        }
        if self
            .required_checks
            .windows(2)
            .any(|pair| (&pair[0].node, &pair[0].name) >= (&pair[1].node, &pair[1].name))
        {
            return Err(DeliveryError::new(
                "snapshot required_checks must be sorted and unique",
            ));
        }
        ensure_count(
            self.required_checks.len(),
            1,
            MAX_CHECKS,
            "snapshot required_checks",
        )?;
        let mut checked_nodes = BTreeSet::new();
        let mut check_keys = BTreeSet::new();
        for check in &self.required_checks {
            check.validate(&node_ids)?;
            if !check_keys.insert((check.node.as_str(), check.name.as_str())) {
                return Err(DeliveryError::new(
                    "snapshot repeats a required check name for a node",
                ));
            }
            checked_nodes.insert(check.node.as_str());
        }
        if checked_nodes != node_ids {
            return Err(DeliveryError::new(
                "snapshot required checks do not cover every stack node",
            ));
        }

        validate_fingerprints(
            "generated_artifacts",
            &self.generated_artifacts,
            &repository_ids,
        )?;
        validate_fingerprints(
            "dependency_fingerprints",
            &self.dependency_fingerprints,
            &repository_ids,
        )?;
        validate_fingerprints(
            "contract_fingerprints",
            &self.contract_fingerprints,
            &repository_ids,
        )?;
        let fingerprint_count = self.generated_artifacts.len()
            + self.dependency_fingerprints.len()
            + self.contract_fingerprints.len();
        ensure_count(
            fingerprint_count,
            1,
            MAX_FINGERPRINTS,
            "snapshot fingerprint matrix",
        )?;
        ensure_count(
            self.dependency_fingerprints.len(),
            1,
            MAX_FINGERPRINTS,
            "snapshot dependency_fingerprints",
        )?;
        ensure_count(
            self.contract_fingerprints.len(),
            1,
            MAX_FINGERPRINTS,
            "snapshot contract_fingerprints",
        )?;

        if self.recompute_candidate_id()? != self.candidate_id {
            return Err(DeliveryError::new(
                "snapshot candidate ID does not match canonical content",
            ));
        }
        if self.recompute_content_id()? != self.content_id {
            return Err(DeliveryError::new(
                "snapshot content ID does not match canonical content",
            ));
        }
        Ok(())
    }

    pub fn repository_bindings(&self) -> Vec<RepositoryBinding> {
        self.repository_set
            .iter()
            .map(RepositoryRecord::binding)
            .collect()
    }

    pub fn recompute_candidate_id(&self) -> Result<String> {
        #[derive(Serialize)]
        struct CandidateMaterial<'a> {
            program: &'a str,
            wave: &'a str,
            panel_trust_root_sha256: &'a str,
            authority: &'a AuthorityBinding,
            repository_set: &'a [RepositoryRecord],
            stack: &'a [StackNode],
            required_validations: &'a [RequiredValidation],
            required_checks: &'a [RequiredCheck],
            generated_artifacts: &'a [Fingerprint],
            dependency_fingerprints: &'a [Fingerprint],
            contract_fingerprints: &'a [Fingerprint],
        }
        canonical_digest(
            b"d2b-delivery-candidate-v1\0",
            &CandidateMaterial {
                program: &self.program,
                wave: &self.wave,
                panel_trust_root_sha256: &self.panel_trust_root_sha256,
                authority: &self.authority,
                repository_set: &self.repository_set,
                stack: &self.stack,
                required_validations: &self.required_validations,
                required_checks: &self.required_checks,
                generated_artifacts: &self.generated_artifacts,
                dependency_fingerprints: &self.dependency_fingerprints,
                contract_fingerprints: &self.contract_fingerprints,
            },
        )
    }

    pub fn recompute_content_id(&self) -> Result<String> {
        #[derive(Serialize)]
        struct ContentRepository<'a> {
            id: &'a str,
            object_format: GitObjectFormat,
            integration_tree_oid: &'a str,
        }
        #[derive(Serialize)]
        struct ContentMaterial<'a> {
            repositories: Vec<ContentRepository<'a>>,
            panel_trust_root_sha256: &'a str,
            required_validations: &'a [RequiredValidation],
            required_checks: &'a [RequiredCheck],
            generated_artifacts: &'a [Fingerprint],
            dependency_fingerprints: &'a [Fingerprint],
            contract_fingerprints: &'a [Fingerprint],
        }
        let repositories = self
            .repository_set
            .iter()
            .map(|repository| ContentRepository {
                id: &repository.id,
                object_format: repository.object_format,
                integration_tree_oid: &repository.integration_tree_oid,
            })
            .collect();
        canonical_digest(
            b"d2b-delivery-content-v1\0",
            &ContentMaterial {
                repositories,
                panel_trust_root_sha256: &self.panel_trust_root_sha256,
                required_validations: &self.required_validations,
                required_checks: &self.required_checks,
                generated_artifacts: &self.generated_artifacts,
                dependency_fingerprints: &self.dependency_fingerprints,
                contract_fingerprints: &self.contract_fingerprints,
            },
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityBinding {
    pub repository: String,
    pub ref_name: String,
    pub commit_oid: String,
    pub tree_oid: String,
    pub manifest_path: String,
    pub manifest_blob_oid: String,
    pub manifest_sha256: String,
}

impl AuthorityBinding {
    fn validate(&self) -> Result<()> {
        validate_repository_id(&self.repository)?;
        validate_git_ref(&self.ref_name, "authority ref")?;
        validate_hash(&self.commit_oid, "authority commit")?;
        validate_hash(&self.tree_oid, "authority tree")?;
        validate_repo_relative_path(Path::new(&self.manifest_path))?;
        validate_hash(&self.manifest_blob_oid, "authority manifest blob")?;
        validate_sha256(&self.manifest_sha256, "authority manifest digest")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryRecord {
    pub id: String,
    pub object_format: GitObjectFormat,
    pub trunk_ref: String,
    pub trunk_oid: String,
    pub trunk_tree_oid: String,
    pub integration_ref: String,
    pub integration_oid: String,
    pub integration_tree_oid: String,
    pub base_to_head_diff_sha256: String,
    pub generated_diff_sha256: String,
    pub dependency_diff_sha256: String,
    pub contract_diff_sha256: String,
    pub stack_graph_sha256: String,
}

impl RepositoryRecord {
    fn validate(&self) -> Result<()> {
        validate_repository_id(&self.id)?;
        validate_git_ref(&self.trunk_ref, "trunk ref")?;
        validate_git_ref(&self.integration_ref, "integration ref")?;
        validate_hash_for_format(&self.trunk_oid, self.object_format, "trunk commit")?;
        validate_hash_for_format(&self.trunk_tree_oid, self.object_format, "trunk tree")?;
        validate_hash_for_format(
            &self.integration_oid,
            self.object_format,
            "integration commit",
        )?;
        validate_hash_for_format(
            &self.integration_tree_oid,
            self.object_format,
            "integration tree",
        )?;
        validate_sha256(&self.base_to_head_diff_sha256, "base-to-head diff digest")?;
        validate_sha256(
            &self.generated_diff_sha256,
            "generated-artifact diff digest",
        )?;
        validate_sha256(&self.dependency_diff_sha256, "dependency diff digest")?;
        validate_sha256(&self.contract_diff_sha256, "contract diff digest")?;
        validate_sha256(&self.stack_graph_sha256, "stack graph digest")
    }

    pub fn binding(&self) -> RepositoryBinding {
        RepositoryBinding {
            id: self.id.clone(),
            object_format: self.object_format,
            commit_oid: self.integration_oid.clone(),
            tree_oid: self.integration_tree_oid.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryBinding {
    pub id: String,
    pub object_format: GitObjectFormat,
    pub commit_oid: String,
    pub tree_oid: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GitObjectFormat {
    Sha1,
    Sha256,
}

impl GitObjectFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
        }
    }

    pub fn hash_len(self) -> usize {
        match self {
            Self::Sha1 => 40,
            Self::Sha256 => 64,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StackNode {
    pub id: String,
    pub repository: String,
    pub pr_number: u64,
    pub expected_base_ref: String,
    pub expected_base_oid: String,
    pub observed_base_oid: String,
    pub head_ref: String,
    pub head_oid: String,
    pub head_tree_oid: String,
    pub merge_commit_oid: Option<String>,
    pub merge_commit_tree_oid: Option<String>,
    pub prospective_merge_tree_oid: String,
    pub prospective_content_id: String,
    pub snapshot_state: PullRequestState,
    pub depends_on: Vec<String>,
}

impl StackNode {
    fn validate(&self, repositories: &BTreeSet<&str>) -> Result<()> {
        validate_identifier(&self.id, "stack node id")?;
        if !repositories.contains(self.repository.as_str()) {
            return Err(DeliveryError::new(format!(
                "stack node {} references unknown repository {}",
                self.id, self.repository
            )));
        }
        if self.pr_number == 0 {
            return Err(DeliveryError::new(format!(
                "stack node {} has PR number 0",
                self.id
            )));
        }
        validate_git_ref(&self.expected_base_ref, "expected base ref")?;
        validate_git_ref(&self.head_ref, "head ref")?;
        validate_hash(&self.expected_base_oid, "expected base OID")?;
        validate_hash(&self.observed_base_oid, "observed PR base OID")?;
        validate_hash(&self.head_oid, "head OID")?;
        validate_hash(&self.head_tree_oid, "head tree OID")?;
        match (
            self.snapshot_state,
            &self.merge_commit_oid,
            &self.merge_commit_tree_oid,
        ) {
            (PullRequestState::Merged, Some(commit), Some(tree)) => {
                validate_hash(commit, "merged PR commit OID")?;
                validate_hash(tree, "merged PR commit tree OID")?;
            }
            (PullRequestState::Open, None, None) => {}
            (PullRequestState::Closed, _, _) => {
                return Err(DeliveryError::new(format!(
                    "stack node {} cannot snapshot a closed PR",
                    self.id
                )));
            }
            _ => {
                return Err(DeliveryError::new(format!(
                    "stack node {} merge-commit authority disagrees with PR state",
                    self.id
                )));
            }
        }
        validate_hash(
            &self.prospective_merge_tree_oid,
            "prospective merge tree OID",
        )?;
        validate_sha256(&self.prospective_content_id, "prospective content identity")?;
        let mut dependencies = BTreeSet::new();
        for dependency in &self.depends_on {
            validate_identifier(dependency, "stack dependency")?;
            if dependency == &self.id || !dependencies.insert(dependency.as_str()) {
                return Err(DeliveryError::new(format!(
                    "stack node {} has a self or duplicate dependency",
                    self.id
                )));
            }
        }
        Ok(())
    }
}

impl GraphNode for StackNode {
    fn id(&self) -> &str {
        &self.id
    }

    fn dependencies(&self) -> &[String] {
        &self.depends_on
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestState {
    Open,
    Merged,
    Closed,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub name: String,
    pub repository: String,
    pub path: String,
    pub git_blob_oid: String,
    pub sha256: String,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceResult {
    Passed,
    Failed,
}

pub fn prospective_content_id(
    repository: &str,
    object_format: GitObjectFormat,
    base_oid: &str,
    head_oid: &str,
    head_tree_oid: &str,
    merge_tree_oid: &str,
) -> Result<String> {
    #[derive(Serialize)]
    struct Material<'a> {
        repository: &'a str,
        object_format: GitObjectFormat,
        base_oid: &'a str,
        head_oid: &'a str,
        head_tree_oid: &'a str,
        merge_tree_oid: &'a str,
    }
    canonical_digest(
        b"d2b-delivery-prospective-content-v1\0",
        &Material {
            repository,
            object_format,
            base_oid,
            head_oid,
            head_tree_oid,
            merge_tree_oid,
        },
    )
}

pub fn canonical_digest(domain: &[u8], value: &impl Serialize) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    Ok(render_digest(hasher.finalize()))
}

fn render_digest(digest: impl IntoIterator<Item = u8>) -> String {
    let mut rendered = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut rendered, "{byte:02x}").expect("writing to String cannot fail");
    }
    rendered
}

trait GraphNode {
    fn id(&self) -> &str;
    fn dependencies(&self) -> &[String];
}

fn graph_order<T: GraphNode>(nodes: &[T]) -> Result<()> {
    let mut indegree = nodes
        .iter()
        .map(|node| (node.id(), node.dependencies().len()))
        .collect::<BTreeMap<_, _>>();
    if indegree.len() != nodes.len() {
        return Err(DeliveryError::new("stack dependency graph repeats a node"));
    }
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
    while let Some(id) = ready.pop_front() {
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
    Ok(())
}

fn validate_fingerprint_specs(
    label: &str,
    fingerprints: &[FingerprintSpec],
    repositories: &BTreeSet<&str>,
) -> Result<()> {
    ensure_count(fingerprints.len(), 0, MAX_FINGERPRINTS, "fingerprints")?;
    if fingerprints
        .windows(2)
        .any(|pair| fingerprint_spec_key(&pair[0]) >= fingerprint_spec_key(&pair[1]))
    {
        return Err(DeliveryError::new(format!(
            "{label} must be strictly sorted by canonical key and duplicate-free"
        )));
    }
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
        if !names.insert(fingerprint.name.as_str()) {
            return Err(DeliveryError::new(format!(
                "duplicate {label} entry {}",
                fingerprint.name
            )));
        }
    }
    Ok(())
}

fn fingerprint_spec_key(fingerprint: &FingerprintSpec) -> (&str, &str, &str) {
    (
        fingerprint.name.as_str(),
        fingerprint.repository.as_str(),
        fingerprint.path.as_str(),
    )
}

fn validate_fingerprints(
    label: &str,
    fingerprints: &[Fingerprint],
    repositories: &BTreeSet<&str>,
) -> Result<()> {
    if !fingerprints.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(DeliveryError::new(format!(
            "snapshot {label} must be sorted and unique"
        )));
    }
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
        validate_hash(&fingerprint.git_blob_oid, "fingerprint Git blob")?;
        validate_sha256(&fingerprint.sha256, "fingerprint digest")?;
        if !names.insert(fingerprint.name.as_str()) {
            return Err(DeliveryError::new(format!(
                "snapshot {label} repeats fingerprint name {}",
                fingerprint.name
            )));
        }
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
    let rendered = path
        .to_str()
        .ok_or_else(|| DeliveryError::new("repository-relative path is not UTF-8"))?;
    validate_bounded_string(rendered, "repository-relative path")
}

pub fn validate_hash(value: &str, label: &str) -> Result<()> {
    if !matches!(value.len(), 40 | 64) || !is_lower_hex(value) {
        return Err(DeliveryError::new(format!(
            "{label} must be a full lowercase Git object hash"
        )));
    }
    Ok(())
}

pub fn validate_hash_for_format(value: &str, format: GitObjectFormat, label: &str) -> Result<()> {
    if value.len() != format.hash_len() || !is_lower_hex(value) {
        return Err(DeliveryError::new(format!(
            "{label} does not match Git object format {}",
            format.as_str()
        )));
    }
    Ok(())
}

pub fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64 || !is_lower_hex(value) {
        return Err(DeliveryError::new(format!(
            "{label} must be a lowercase SHA-256 digest"
        )));
    }
    Ok(())
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

pub fn validate_repository_id(id: &str) -> Result<()> {
    validate_bounded_string(id, "repository identity")?;
    let parts = id.split('/').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != "github.com"
        || parts[1].is_empty()
        || parts[2].is_empty()
        || !parts[1]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || !parts[2]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(DeliveryError::new(format!(
            "logical repository identity must be github.com/owner/repository: {id:?}"
        )));
    }
    Ok(())
}

pub fn validate_git_ref(value: &str, label: &str) -> Result<()> {
    validate_bounded_string(value, label)?;
    if value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("..")
        || value.contains("@{")
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(DeliveryError::new(format!("invalid {label}")));
    }
    Ok(())
}

pub fn validate_bounded_string(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > MAX_STRING_BYTES {
        return Err(DeliveryError::new(format!(
            "{label} must be non-empty and at most {MAX_STRING_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_optional_bounded_string(value: &str, label: &str) -> Result<()> {
    if value.len() > MAX_STRING_BYTES {
        return Err(DeliveryError::new(format!(
            "{label} must be at most {MAX_STRING_BYTES} bytes"
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

fn ensure_count(count: usize, minimum: usize, maximum: usize, label: &str) -> Result<()> {
    if count < minimum || count > maximum {
        return Err(DeliveryError::new(format!(
            "{label} count must be between {minimum} and {maximum}, found {count}"
        )));
    }
    Ok(())
}

fn ensure_sorted_unique_by<'a, T>(
    values: &'a [T],
    key: impl Fn(&'a T) -> &'a str,
    label: &str,
) -> Result<()> {
    if values.windows(2).any(|pair| key(&pair[0]) >= key(&pair[1])) {
        return Err(DeliveryError::new(format!(
            "{label} must be sorted and unique"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery_manifest() -> DeliveryManifest {
        serde_json::from_str(include_str!("../../../../delivery/manifest.json"))
            .expect("checked-in delivery manifest")
    }

    fn fingerprint_category_mut<'a>(
        manifest: &'a mut DeliveryManifest,
        category: &str,
    ) -> &'a mut Vec<FingerprintSpec> {
        match category {
            "generated_artifacts" => &mut manifest.generated_artifacts,
            "dependency_fingerprints" => &mut manifest.dependency_fingerprints,
            "contract_fingerprints" => &mut manifest.contract_fingerprints,
            _ => panic!("unknown fingerprint category"),
        }
    }

    fn ensure_two_fingerprints(fingerprints: &mut Vec<FingerprintSpec>) {
        if fingerprints.len() == 1 {
            fingerprints.push(FingerprintSpec {
                name: "zz-test-artifact".to_owned(),
                repository: fingerprints[0].repository.clone(),
                path: "tests/layer1-jobs.json".to_owned(),
            });
        }
        fingerprints.sort();
    }

    #[test]
    fn canonical_ids_are_domain_separated() {
        let value = serde_json::json!({"a": 1});
        assert_ne!(
            canonical_digest(b"candidate\0", &value).expect("candidate"),
            canonical_digest(b"content\0", &value).expect("content")
        );
    }

    #[test]
    fn checked_in_delivery_manifest_has_canonical_fingerprint_order() {
        delivery_manifest()
            .validate()
            .expect("valid delivery manifest");
    }

    #[test]
    fn delivery_authority_paths_are_closed_and_wave_bound() {
        assert!(is_authoritative_manifest_path(Path::new(
            "delivery/manifest.json"
        )));
        assert!(is_authoritative_manifest_path(Path::new(
            "delivery/manifests/w5.json"
        )));
        for rejected in [
            "delivery/manifests/w0.json",
            "delivery/manifests/w05.json",
            "delivery/manifests/w5.yaml",
            "delivery/manifests/nested/w5.json",
            "delivery/other/w5.json",
        ] {
            assert!(
                !is_authoritative_manifest_path(Path::new(rejected)),
                "{rejected}"
            );
        }
        assert_eq!(
            expected_wave_manifest_path("w7").expect("wave path"),
            PathBuf::from("delivery/manifests/w7.json")
        );
        assert!(expected_wave_manifest_path("wave7").is_err());
    }

    #[test]
    fn delivery_manifest_rejects_reordered_fingerprint_declarations() {
        for category in [
            "generated_artifacts",
            "dependency_fingerprints",
            "contract_fingerprints",
        ] {
            let mut manifest = delivery_manifest();
            let fingerprints = fingerprint_category_mut(&mut manifest, category);
            ensure_two_fingerprints(fingerprints);
            fingerprints.swap(0, 1);

            let error = manifest.validate().expect_err("reordered fingerprints");
            assert!(error.to_string().contains(category));
            assert!(error.to_string().contains("strictly sorted"));
        }
    }

    #[test]
    fn delivery_manifest_rejects_equal_canonical_fingerprint_keys() {
        for category in [
            "generated_artifacts",
            "dependency_fingerprints",
            "contract_fingerprints",
        ] {
            let mut manifest = delivery_manifest();
            let fingerprints = fingerprint_category_mut(&mut manifest, category);
            let duplicate = fingerprints[0].clone();
            fingerprints.insert(1, duplicate);

            let error = manifest.validate().expect_err("equal canonical keys");
            assert!(error.to_string().contains(category));
            assert!(error.to_string().contains("duplicate-free"));
        }
    }

    #[test]
    fn delivery_manifest_rejects_duplicate_names_with_distinct_canonical_keys() {
        let mut manifest = delivery_manifest();
        let fingerprints = &mut manifest.contract_fingerprints;
        let mut duplicate = fingerprints[0].clone();
        duplicate.path.push_str(".duplicate");
        fingerprints.insert(1, duplicate);

        let error = manifest.validate().expect_err("duplicate fingerprint name");
        assert!(
            error
                .to_string()
                .contains("duplicate contract_fingerprints entry")
        );
    }

    #[test]
    fn git_town_graph_rejects_duplicate_or_ambiguous_current_branch() {
        let graph = StackGraph {
            trunk: "main".to_owned(),
            current_branch: "one".to_owned(),
            branches: vec![
                StackBranch {
                    name: "one".to_owned(),
                    parent: "main".to_owned(),
                    base_ref: "main".to_owned(),
                    observed_base: "b".repeat(40),
                    head: "a".repeat(40),
                    base: "b".repeat(40),
                    is_current: true,
                    is_merged: false,
                    is_queued: false,
                    needs_rebase: false,
                    pr: Some(StackPr {
                        number: 1,
                        url: String::new(),
                        state: "OPEN".to_owned(),
                    }),
                    merge_commit_oid: None,
                    merge_commit_tree_oid: None,
                },
                StackBranch {
                    name: "one".to_owned(),
                    parent: "one".to_owned(),
                    base_ref: "one".to_owned(),
                    observed_base: "a".repeat(40),
                    head: "c".repeat(40),
                    base: "a".repeat(40),
                    is_current: false,
                    is_merged: false,
                    is_queued: false,
                    needs_rebase: false,
                    pr: Some(StackPr {
                        number: 2,
                        url: String::new(),
                        state: "OPEN".to_owned(),
                    }),
                    merge_commit_oid: None,
                    merge_commit_tree_oid: None,
                },
            ],
        };
        assert!(graph.validate().is_err());
    }

    #[test]
    fn git_town_graph_rejects_duplicate_pull_requests() {
        let branch = |name: &str, current: bool| StackBranch {
            name: name.to_owned(),
            parent: "main".to_owned(),
            base_ref: "main".to_owned(),
            observed_base: "0".repeat(40),
            head: if current { "b" } else { "a" }.repeat(40),
            base: "0".repeat(40),
            is_current: current,
            is_merged: false,
            is_queued: false,
            needs_rebase: false,
            pr: Some(StackPr {
                number: 7,
                url: String::new(),
                state: "OPEN".to_owned(),
            }),
            merge_commit_oid: None,
            merge_commit_tree_oid: None,
        };
        let graph = StackGraph {
            trunk: "main".to_owned(),
            current_branch: "two".to_owned(),
            branches: vec![branch("one", false), branch("two", true)],
        };
        let error = graph.validate().expect_err("duplicate PR");
        assert!(error.to_string().contains("repeats PR"));
    }

    #[test]
    fn rejects_traversal_and_oversized_identifiers() {
        assert!(validate_repo_relative_path(Path::new("../secret")).is_err());
        assert!(validate_identifier(&"a".repeat(129), "id").is_err());
    }
}
