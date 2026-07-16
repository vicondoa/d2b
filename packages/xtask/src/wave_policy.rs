use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::ExitCode,
};

use serde::Deserialize;

use crate::delivery::{
    command::{
        CommandLimits, CommandOutput, CommandOutputAdapter, GhStatusSource, GitProbe,
        ProcessCommandOutput, PullRequestStatusSource, RepositoryProbe, authority_git_environment,
    },
    model::{
        DeliveryManifest, PullRequestState, expected_wave_manifest_path,
        is_authoritative_manifest_path, validate_git_ref, validate_hash, validate_identifier,
        validate_repository_id, validate_wave_identifier,
    },
};

const POLICY_PATH: &str = "delivery/shared-contracts.json";
const USAGE: &str = "usage: cargo xtask wave-policy check --candidate-root <wave-worktree-path>";
const REQUIRED_PROTECTED_PATHS: &[&str] = &[
    "AGENTS.md",
    "Makefile",
    "delivery/README.md",
    POLICY_PATH,
    "docs/adr/0045-provider-and-transport-framework.md",
    "docs/reference/delivery-tooling.md",
    "packages/xtask/Cargo.toml",
    "packages/xtask/src/lib.rs",
    "packages/xtask/src/main.rs",
    "packages/xtask/src/wave_policy.rs",
    "packages/xtask/tests/policy_workspace.rs",
    "tests/AGENTS.md",
];
const REQUIRED_PROTECTED_PREFIXES: &[&str] =
    &["packages/d2b-contracts/", "packages/xtask/src/delivery/"];
const REQUIRED_DOCUMENTATION_PATHS: &[&str] = &["CHANGELOG.md", "README.md"];
const REQUIRED_DOCUMENTATION_PREFIXES: &[&str] = &[
    "docs/completions/",
    "docs/explanation/",
    "docs/how-to/",
    "docs/manpages/",
    "docs/reference/",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SharedContractPolicy {
    pub schema_version: u32,
    pub authority_repository: String,
    pub shared_root_branch: String,
    pub waves: Vec<WaveOwnership>,
    pub protected_paths: Vec<String>,
    pub protected_prefixes: Vec<String>,
    pub frozen_prefixes: Vec<String>,
    pub documentation_paths: Vec<String>,
    pub documentation_prefixes: Vec<String>,
    pub frozen_service_packages: Vec<String>,
    pub broker_typed_methods: Vec<TypedBrokerMethod>,
    pub service_dependency_edges: Vec<ServiceDependencyEdge>,
    pub workspace_dependencies: Vec<WorkspaceDependency>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WaveOwnership {
    pub wave: String,
    pub branch_stem: String,
    pub manifest_path: String,
    pub responsibility: String,
    pub allowed_parent_waves: Vec<String>,
    pub allowed_prefixes: Vec<String>,
    pub foreign_prefixes: Vec<String>,
    #[serde(default)]
    pub additional_protected_paths: Vec<String>,
    #[serde(default)]
    pub allowed_protected_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(deny_unknown_fields)]
pub struct TypedBrokerMethod {
    pub method: String,
    pub request: String,
    pub response: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceDependency {
    pub name: String,
    pub requirement: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(deny_unknown_fields)]
pub struct ServiceDependencyEdge {
    pub consumer: String,
    pub dependency: String,
    pub default_features: bool,
    pub features: Vec<String>,
}

impl SharedContractPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 4 {
            return Err("unsupported shared-contract policy schema".to_owned());
        }
        if self.authority_repository != "github.com/vicondoa/d2b" {
            return Err("shared-contract policy names an unexpected repository".to_owned());
        }
        validate_repository_id(&self.authority_repository).map_err(|error| error.to_string())?;
        if self.shared_root_branch != "adr0045-post-w4-contracts" {
            return Err("shared-contract policy names an unexpected root branch".to_owned());
        }
        let waves = self
            .waves
            .iter()
            .map(|wave| wave.wave.as_str())
            .collect::<BTreeSet<_>>();
        if waves != BTreeSet::from(["w5", "w6", "w7"]) || waves.len() != self.waves.len() {
            return Err("shared-contract policy must define exactly w5, w6, and w7".to_owned());
        }
        let expected_branch_stems = BTreeMap::from([
            ("w5", "adr0045-w5"),
            ("w6", "adr0045-w6"),
            ("w7", "adr0045-w7"),
        ]);
        let mut prefix_owners = BTreeMap::new();
        for wave in &self.waves {
            validate_wave_identifier(&wave.wave).map_err(|error| error.to_string())?;
            validate_git_ref(&wave.branch_stem, "wave branch stem")
                .map_err(|error| error.to_string())?;
            if expected_branch_stems.get(wave.wave.as_str()).copied()
                != Some(wave.branch_stem.as_str())
            {
                return Err(format!("{} branch stem is not canonical", wave.wave));
            }
            validate_sorted_strings_allow_empty(
                &wave.allowed_parent_waves,
                "allowed parent waves",
            )?;
            let expected_parent_waves: &[&str] = match wave.wave.as_str() {
                "w5" => &[],
                "w6" => &["w5"],
                "w7" => &["w6"],
                _ => unreachable!("wave set was validated"),
            };
            if !wave
                .allowed_parent_waves
                .iter()
                .map(String::as_str)
                .eq(expected_parent_waves.iter().copied())
            {
                return Err(format!(
                    "{} parent-wave authority does not match the delivery graph",
                    wave.wave
                ));
            }
            let expected =
                expected_wave_manifest_path(&wave.wave).map_err(|error| error.to_string())?;
            if Path::new(&wave.manifest_path) != expected {
                return Err(format!(
                    "{} manifest authority must be {}",
                    wave.wave,
                    expected.display()
                ));
            }
            if wave.responsibility.trim().is_empty() {
                return Err(format!("{} responsibility is empty", wave.wave));
            }
            validate_sorted_directory_prefixes(
                &wave.allowed_prefixes,
                "allowed implementation prefixes",
            )?;
            validate_sorted_directory_prefixes(
                &wave.foreign_prefixes,
                "foreign implementation prefixes",
            )?;
            for prefix in &wave.allowed_prefixes {
                if let Some(owner) = prefix_owners.insert(prefix.as_str(), wave.wave.as_str()) {
                    return Err(format!(
                        "implementation prefix {prefix} is owned by both {owner} and {}",
                        wave.wave
                    ));
                }
            }
            if !wave.additional_protected_paths.is_empty() {
                validate_sorted_paths(&wave.additional_protected_paths)?;
            }
            if !wave.allowed_protected_paths.is_empty() {
                validate_sorted_paths(&wave.allowed_protected_paths)?;
            }
        }
        validate_sorted_paths(&self.protected_paths)?;
        validate_sorted_prefixes(&self.protected_prefixes)?;
        validate_sorted_directory_prefixes(
            &self.frozen_prefixes,
            "frozen implementation prefixes",
        )?;
        validate_sorted_relative_paths(&self.documentation_paths, "documentation paths")?;
        validate_sorted_directory_prefixes(&self.documentation_prefixes, "documentation prefixes")?;
        validate_sorted_strings(&self.frozen_service_packages, "frozen service packages")?;
        validate_sorted_values(&self.broker_typed_methods, "typed broker methods")?;
        validate_sorted_values(&self.service_dependency_edges, "service dependency edges")?;
        for edge in &self.service_dependency_edges {
            validate_identifier(&edge.consumer, "service dependency consumer")
                .map_err(|error| error.to_string())?;
            validate_identifier(&edge.dependency, "service dependency")
                .map_err(|error| error.to_string())?;
            validate_sorted_strings_allow_empty(&edge.features, "service dependency features")?;
            if edge.default_features {
                return Err(format!(
                    "{} -> {} must disable default features",
                    edge.consumer, edge.dependency
                ));
            }
        }
        validate_sorted_values(&self.workspace_dependencies, "workspace dependencies")?;
        for required in REQUIRED_PROTECTED_PATHS {
            if self
                .protected_paths
                .binary_search_by(|path| path.as_str().cmp(required))
                .is_err()
            {
                return Err(format!(
                    "shared-contract policy does not protect required path {required}"
                ));
            }
        }
        for required in REQUIRED_PROTECTED_PREFIXES {
            if self
                .protected_prefixes
                .binary_search_by(|prefix| prefix.as_str().cmp(required))
                .is_err()
            {
                return Err(format!(
                    "shared-contract policy does not protect required prefix {required}"
                ));
            }
        }
        for required in REQUIRED_DOCUMENTATION_PATHS {
            if self
                .documentation_paths
                .binary_search_by(|path| path.as_str().cmp(required))
                .is_err()
            {
                return Err(format!(
                    "shared-contract policy does not allow required documentation path {required}"
                ));
            }
        }
        for required in REQUIRED_DOCUMENTATION_PREFIXES {
            if self
                .documentation_prefixes
                .binary_search_by(|prefix| prefix.as_str().cmp(required))
                .is_err()
            {
                return Err(format!(
                    "shared-contract policy does not allow required documentation prefix {required}"
                ));
            }
        }
        for wave in &self.waves {
            let expected_foreign = self
                .waves
                .iter()
                .filter(|other| other.wave != wave.wave)
                .flat_map(|other| other.allowed_prefixes.iter().cloned())
                .collect::<BTreeSet<_>>();
            let actual_foreign = wave
                .foreign_prefixes
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            if actual_foreign != expected_foreign {
                return Err(format!(
                    "{} foreign prefixes must be exactly the other waves' implementation prefixes",
                    wave.wave
                ));
            }
        }
        let owned_prefixes = self
            .waves
            .iter()
            .flat_map(|wave| {
                wave.allowed_prefixes
                    .iter()
                    .map(move |prefix| (wave.wave.as_str(), prefix.as_str()))
            })
            .collect::<Vec<_>>();
        for (index, (left_wave, left)) in owned_prefixes.iter().enumerate() {
            for (right_wave, right) in &owned_prefixes[index + 1..] {
                if left.starts_with(right) || right.starts_with(left) {
                    return Err(format!(
                        "implementation prefixes {left} ({left_wave}) and {right} ({right_wave}) overlap"
                    ));
                }
            }
        }
        for frozen in &self.frozen_prefixes {
            for (wave, owned) in &owned_prefixes {
                if frozen.starts_with(owned) || owned.starts_with(frozen) {
                    return Err(format!(
                        "frozen implementation prefix {frozen} overlaps {owned} owned by {wave}"
                    ));
                }
            }
        }
        for (index, left) in self.frozen_prefixes.iter().enumerate() {
            for right in &self.frozen_prefixes[index + 1..] {
                if left.starts_with(right) || right.starts_with(left) {
                    return Err(format!(
                        "frozen implementation prefixes {left} and {right} overlap"
                    ));
                }
            }
        }
        for wave in &self.waves {
            for path in &wave.allowed_protected_paths {
                let globally_protected = self.protected_paths.binary_search(path).is_ok()
                    || self
                        .protected_prefixes
                        .iter()
                        .any(|prefix| path_matches_prefix(path, prefix));
                if !globally_protected {
                    return Err(format!(
                        "{} exception {path} is not a protected shared-root path",
                        wave.wave
                    ));
                }
                if wave
                    .foreign_prefixes
                    .iter()
                    .any(|prefix| path_matches_prefix(path, prefix))
                {
                    return Err(format!(
                        "{} exception {path} grants a foreign-wave implementation path",
                        wave.wave
                    ));
                }
            }
        }
        Ok(())
    }

    fn implementation_path(&self, path: &str) -> Option<ImplementationPath<'_>> {
        for wave in &self.waves {
            for prefix in &wave.allowed_prefixes {
                if path_matches_prefix(path, prefix) {
                    return Some(ImplementationPath {
                        owner: ImplementationOwner::Wave(&wave.wave),
                        at_prefix_root: path == prefix.trim_end_matches('/'),
                    });
                }
            }
        }
        self.frozen_prefixes.iter().find_map(|prefix| {
            path_matches_prefix(path, prefix).then_some(ImplementationPath {
                owner: ImplementationOwner::Frozen,
                at_prefix_root: path == prefix.trim_end_matches('/'),
            })
        })
    }

    fn is_documentation_path(&self, path: &str) -> bool {
        self.documentation_paths
            .binary_search_by(|candidate| candidate.as_str().cmp(path))
            .is_ok()
            || self
                .documentation_prefixes
                .iter()
                .any(|prefix| path.starts_with(prefix))
    }

    fn wave(&self, wave: &str) -> Result<&WaveOwnership, String> {
        self.waves
            .iter()
            .find(|entry| entry.wave == wave)
            .ok_or_else(|| format!("wave {wave} is not governed by the shared-contract policy"))
    }

    fn wave_for_branch(&self, branch: &str) -> Result<&WaveOwnership, String> {
        let matches = self
            .waves
            .iter()
            .filter(|wave| branch_matches_stem(branch, &wave.branch_stem))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [wave] => Ok(*wave),
            [] => Err(format!(
                "branch {branch} does not identify a governed w5, w6, or w7 wave"
            )),
            _ => Err(format!("branch {branch} ambiguously identifies a wave")),
        }
    }

    fn parent_is_allowed(&self, ownership: &WaveOwnership, parent: &str) -> bool {
        if parent == self.shared_root_branch {
            return true;
        }
        self.wave_for_branch(parent).is_ok_and(|parent_wave| {
            ownership
                .allowed_parent_waves
                .binary_search(&parent_wave.wave)
                .is_ok()
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImplementationOwner<'a> {
    Wave(&'a str),
    Frozen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ImplementationPath<'a> {
    owner: ImplementationOwner<'a>,
    at_prefix_root: bool,
}

pub fn run_cli(args: &[String]) -> ExitCode {
    match run(args) {
        Ok(verified) => {
            println!(
                "wave ownership policy: ok ({} {} against {})",
                verified.wave, verified.branch, verified.base_oid
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("wave ownership policy failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<VerifiedOwnership, String> {
    let candidate_root = parse_candidate_root(args)?;
    let authority_root = repository_root()?;
    verify_ownership(
        &ProcessOwnershipProbe::default(),
        &authority_root,
        &candidate_root,
    )
}

fn parse_candidate_root(args: &[String]) -> Result<PathBuf, String> {
    let [action, candidate_flag, candidate_root] = args else {
        return Err(USAGE.to_owned());
    };
    if action != "check" || candidate_flag != "--candidate-root" {
        return Err(USAGE.to_owned());
    }
    let candidate_root = PathBuf::from(candidate_root);
    if !candidate_root.is_absolute() {
        return Err("candidate root must be an absolute path".to_owned());
    }
    Ok(candidate_root)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VerifiedOwnership {
    wave: String,
    branch: String,
    base_oid: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OrdinaryPullRequest {
    repository: String,
    state: PullRequestState,
    base_ref: String,
    base_oid: String,
    head_repository: String,
    head_ref: String,
    head_oid: String,
    is_in_merge_queue: bool,
}

trait OwnershipProbe {
    fn canonical_root(&self, root: &Path) -> Result<PathBuf, String>;
    fn reject_history_rewrites(&self, root: &Path) -> Result<(), String>;
    fn repository_identity(&self, root: &Path) -> Result<String, String>;
    fn is_dirty(&self, root: &Path) -> Result<bool, String>;
    fn current_branch(&self, root: &Path) -> Result<String, String>;
    fn git_town_parent(&self, root: &Path, branch: &str) -> Result<String, String>;
    fn resolve_commit(&self, root: &Path, revision: &str) -> Result<String, String>;
    fn is_ancestor(&self, root: &Path, base: &str, head: &str) -> Result<bool, String>;
    fn open_pull_request(
        &self,
        repository: &str,
        branch: &str,
    ) -> Result<OrdinaryPullRequest, String>;
    fn tracked_blob(&self, root: &Path, commit: &str, path: &str) -> Result<Vec<u8>, String>;
    fn changed_paths(&self, root: &Path, base: &str, head: &str) -> Result<Vec<String>, String>;
}

#[derive(Clone, Copy, Debug, Default)]
struct ProcessOwnershipProbe {
    command: ProcessCommandOutput,
}

impl ProcessOwnershipProbe {
    fn git_probe(&self) -> GitProbe<ProcessCommandOutput> {
        GitProbe::new(self.command)
    }

    fn output(
        &self,
        program: &str,
        arguments: &[String],
        cwd: Option<&Path>,
        failure: &str,
    ) -> Result<CommandOutput, String> {
        let output = self
            .command
            .output(program, arguments, cwd)
            .map_err(|error| error.to_string())?;
        if output.success {
            Ok(output)
        } else {
            Err(format!("{failure}: {}", output.safe_failure_summary()))
        }
    }

    fn git_output(
        &self,
        root: &Path,
        arguments: &[String],
        failure: &str,
    ) -> Result<CommandOutput, String> {
        let mut args = vec![
            "--no-replace-objects".to_owned(),
            "-c".to_owned(),
            "diff.ignoreSubmodules=none".to_owned(),
            "-C".to_owned(),
            root.to_str()
                .ok_or_else(|| "repository path is not UTF-8".to_owned())?
                .to_owned(),
        ];
        args.extend_from_slice(arguments);
        let output = self
            .command
            .output_with_environment(
                "git",
                &args,
                None,
                &authority_git_environment(),
                CommandLimits::default(),
            )
            .map_err(|error| error.to_string())?;
        if output.success {
            Ok(output)
        } else {
            Err(format!("{failure}: {}", output.safe_failure_summary()))
        }
    }

    fn command_text(
        &self,
        program: &str,
        arguments: &[String],
        cwd: Option<&Path>,
        failure: &str,
    ) -> Result<String, String> {
        let output = self.output(program, arguments, cwd, failure)?;
        let value = String::from_utf8(output.stdout)
            .map_err(|_| format!("{failure}: output is not UTF-8"))?
            .trim()
            .to_owned();
        if value.is_empty() || value.contains('\n') || value.contains('\0') {
            return Err(format!("{failure}: output is missing or ambiguous"));
        }
        Ok(value)
    }
}

impl OwnershipProbe for ProcessOwnershipProbe {
    fn canonical_root(&self, root: &Path) -> Result<PathBuf, String> {
        self.git_probe()
            .canonical_root(root)
            .map_err(|error| error.to_string())
    }

    fn reject_history_rewrites(&self, root: &Path) -> Result<(), String> {
        let replace_refs = self.git_output(
            root,
            &[
                "for-each-ref".to_owned(),
                "--format=%(refname)".to_owned(),
                "refs/replace".to_owned(),
            ],
            "cannot inspect Git replacement refs",
        )?;
        if !replace_refs.stdout.is_empty() {
            return Err("repository contains forbidden refs/replace metadata".to_owned());
        }

        let common_dir = self
            .git_probe()
            .git_common_dir(root)
            .map_err(|error| error.to_string())?;
        for (relative, label) in [
            (Path::new("info/grafts"), "graft"),
            (Path::new("shallow"), "shallow"),
        ] {
            let path = common_dir.join(relative);
            match fs::symlink_metadata(&path) {
                Ok(_) => {
                    return Err(format!(
                        "repository contains forbidden Git {label} metadata"
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!("cannot inspect Git {label} metadata: {error}"));
                }
            }
        }
        Ok(())
    }

    fn repository_identity(&self, root: &Path) -> Result<String, String> {
        self.git_probe()
            .repository_identity(root)
            .map_err(|error| error.to_string())
    }

    fn is_dirty(&self, root: &Path) -> Result<bool, String> {
        self.git_probe()
            .is_dirty(root)
            .map_err(|error| error.to_string())
    }

    fn current_branch(&self, root: &Path) -> Result<String, String> {
        let output = self.git_output(
            root,
            &[
                "symbolic-ref".to_owned(),
                "--quiet".to_owned(),
                "--short".to_owned(),
                "HEAD".to_owned(),
            ],
            "cannot resolve candidate branch",
        )?;
        let value = String::from_utf8(output.stdout)
            .map_err(|_| "cannot resolve candidate branch: output is not UTF-8".to_owned())?
            .trim()
            .to_owned();
        if value.is_empty() || value.contains('\n') || value.contains('\0') {
            return Err(
                "cannot resolve candidate branch: output is missing or ambiguous".to_owned(),
            );
        }
        Ok(value)
    }

    fn git_town_parent(&self, root: &Path, branch: &str) -> Result<String, String> {
        self.command_text(
            "git-town",
            &[
                "config".to_owned(),
                "get-parent".to_owned(),
                branch.to_owned(),
            ],
            Some(root),
            "Git Town parent configuration is missing or unreadable",
        )
    }

    fn resolve_commit(&self, root: &Path, revision: &str) -> Result<String, String> {
        self.git_probe()
            .resolve_commit(root, revision)
            .map_err(|error| error.to_string())
    }

    fn is_ancestor(&self, root: &Path, base: &str, head: &str) -> Result<bool, String> {
        self.git_probe()
            .is_ancestor(root, base, head)
            .map_err(|error| error.to_string())
    }

    fn open_pull_request(
        &self,
        repository: &str,
        branch: &str,
    ) -> Result<OrdinaryPullRequest, String> {
        let slug = repository
            .strip_prefix("github.com/")
            .ok_or_else(|| "authority repository is not hosted by GitHub".to_owned())?;
        let output = self.output(
            "gh",
            &[
                "pr".to_owned(),
                "list".to_owned(),
                "--repo".to_owned(),
                slug.to_owned(),
                "--state".to_owned(),
                "open".to_owned(),
                "--head".to_owned(),
                branch.to_owned(),
                "--limit".to_owned(),
                "2".to_owned(),
                "--json".to_owned(),
                "number".to_owned(),
            ],
            None,
            "cannot discover the ordinary GitHub PR for the candidate branch",
        )?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct PullRequestNumber {
            number: u64,
        }
        let rows: Vec<PullRequestNumber> = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("invalid GitHub PR discovery JSON: {error}"))?;
        let [row] = rows.as_slice() else {
            return Err(
                "candidate branch must have exactly one open ordinary GitHub PR".to_owned(),
            );
        };
        let status = GhStatusSource::new(&self.command)
            .status(repository, row.number)
            .map_err(|error| error.to_string())?;
        Ok(OrdinaryPullRequest {
            repository: status.repository,
            state: status.state,
            base_ref: status.base_ref,
            base_oid: status.base_oid,
            head_repository: status.head_repository,
            head_ref: status.head_ref,
            head_oid: status.head_oid,
            is_in_merge_queue: status.is_in_merge_queue,
        })
    }

    fn tracked_blob(&self, root: &Path, commit: &str, path: &str) -> Result<Vec<u8>, String> {
        validate_hash(commit, "tracked blob commit").map_err(|error| error.to_string())?;
        validate_relative_path(Path::new(path))?;
        Ok(self
            .git_output(
                root,
                &[
                    "cat-file".to_owned(),
                    "blob".to_owned(),
                    format!("{commit}:{path}"),
                ],
                "cannot read tracked authority blob",
            )?
            .stdout)
    }

    fn changed_paths(&self, root: &Path, base: &str, head: &str) -> Result<Vec<String>, String> {
        validate_hash(base, "wave ownership base").map_err(|error| error.to_string())?;
        validate_hash(head, "wave ownership head").map_err(|error| error.to_string())?;
        let output = self.git_output(
            root,
            &[
                "diff".to_owned(),
                "--no-renames".to_owned(),
                "--ignore-submodules=none".to_owned(),
                "--name-only".to_owned(),
                "-z".to_owned(),
                base.to_owned(),
                head.to_owned(),
                "--".to_owned(),
            ],
            "cannot inspect wave diff",
        )?;
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                std::str::from_utf8(entry)
                    .map(str::to_owned)
                    .map_err(|_| "wave diff path is not UTF-8".to_owned())
            })
            .collect()
    }
}

fn verify_ownership<P: OwnershipProbe>(
    probe: &P,
    authority_root: &Path,
    candidate_root: &Path,
) -> Result<VerifiedOwnership, String> {
    let authority_root = probe.canonical_root(authority_root)?;
    let candidate_root = probe.canonical_root(candidate_root)?;
    probe.reject_history_rewrites(&authority_root)?;
    probe.reject_history_rewrites(&candidate_root)?;
    if probe.is_dirty(&authority_root)? {
        return Err("trusted authority worktree must be clean".to_owned());
    }
    if probe.is_dirty(&candidate_root)? {
        return Err("wave ownership checks require a clean candidate worktree".to_owned());
    }

    let authority_oid = probe.resolve_commit(&authority_root, "HEAD")?;
    validate_hash(&authority_oid, "trusted authority HEAD").map_err(|error| error.to_string())?;
    let policy_bytes = probe.tracked_blob(&authority_root, &authority_oid, POLICY_PATH)?;
    let policy = parse_policy(&policy_bytes)?;

    let authority_repository = probe.repository_identity(&authority_root)?;
    let candidate_repository = probe.repository_identity(&candidate_root)?;
    validate_repository_id(&authority_repository).map_err(|error| error.to_string())?;
    validate_repository_id(&candidate_repository).map_err(|error| error.to_string())?;
    if authority_repository != policy.authority_repository
        || candidate_repository != policy.authority_repository
    {
        return Err("authority or candidate repository identity differs from policy".to_owned());
    }

    let branch = probe.current_branch(&candidate_root)?;
    validate_git_ref(&branch, "candidate branch").map_err(|error| error.to_string())?;
    let ownership = policy.wave_for_branch(&branch)?;
    let head_oid = probe.resolve_commit(&candidate_root, "HEAD")?;
    let branch_oid = probe.resolve_commit(&candidate_root, &branch)?;
    validate_hash(&head_oid, "candidate HEAD").map_err(|error| error.to_string())?;
    validate_hash(&branch_oid, "candidate branch OID").map_err(|error| error.to_string())?;
    if head_oid != branch_oid {
        return Err("candidate HEAD does not match its current branch ref".to_owned());
    }

    let parent = probe.git_town_parent(&candidate_root, &branch)?;
    validate_git_ref(&parent, "Git Town parent").map_err(|error| error.to_string())?;
    if parent == branch {
        return Err("Git Town parent configuration contains a self-cycle".to_owned());
    }
    let base_oid = probe.resolve_commit(&candidate_root, &parent)?;
    validate_hash(&base_oid, "Git Town parent OID").map_err(|error| error.to_string())?;

    let pull_request = probe.open_pull_request(&candidate_repository, &branch)?;
    verify_pr_authority(
        &pull_request,
        &candidate_repository,
        &branch,
        &head_oid,
        &parent,
        &base_oid,
        "candidate",
    )?;
    if base_oid == head_oid {
        return Err("candidate HEAD cannot be its own ownership base".to_owned());
    }
    if !probe.is_ancestor(&candidate_root, &base_oid, &head_oid)? {
        return Err("verified ownership base is not an ancestor of candidate HEAD".to_owned());
    }

    if authority_oid != base_oid {
        return Err(
            "ownership checker is not executing from the exact verified parent commit".to_owned(),
        );
    }
    if !policy.parent_is_allowed(ownership, &parent) {
        return Err(format!(
            "{} cannot use Git Town parent {parent} as ownership authority",
            ownership.wave
        ));
    }
    verify_parent_graph(
        probe,
        &candidate_root,
        &candidate_repository,
        &policy,
        ownership,
        &parent,
        &base_oid,
    )?;

    let manifest = probe.tracked_blob(&candidate_root, &head_oid, &ownership.manifest_path)?;
    verify_checked_in_manifest(&manifest, ownership)?;
    let paths = probe.changed_paths(&candidate_root, &base_oid, &head_oid)?;
    check_changed_paths(&policy, &ownership.wave, &paths)?;
    Ok(VerifiedOwnership {
        wave: ownership.wave.clone(),
        branch,
        base_oid,
    })
}

fn verify_pr_authority(
    pull_request: &OrdinaryPullRequest,
    repository: &str,
    branch: &str,
    head_oid: &str,
    parent: &str,
    base_oid: &str,
    label: &str,
) -> Result<(), String> {
    validate_repository_id(&pull_request.repository).map_err(|error| error.to_string())?;
    validate_repository_id(&pull_request.head_repository).map_err(|error| error.to_string())?;
    validate_git_ref(&pull_request.base_ref, "GitHub PR base")
        .map_err(|error| error.to_string())?;
    validate_git_ref(&pull_request.head_ref, "GitHub PR head")
        .map_err(|error| error.to_string())?;
    validate_hash(&pull_request.base_oid, "GitHub PR base OID")
        .map_err(|error| error.to_string())?;
    validate_hash(&pull_request.head_oid, "GitHub PR head OID")
        .map_err(|error| error.to_string())?;
    if pull_request.state != PullRequestState::Open
        || pull_request.repository != repository
        || pull_request.head_repository != repository
        || pull_request.head_ref != branch
        || pull_request.head_oid != head_oid
    {
        return Err(format!(
            "ordinary GitHub PR head authority does not match {label}"
        ));
    }
    if pull_request.is_in_merge_queue {
        return Err(format!(
            "queued pull requests cannot provide {label} ownership authority"
        ));
    }
    if pull_request.base_ref != parent || pull_request.base_oid != base_oid {
        return Err(format!(
            "Git Town parent and ordinary GitHub PR base authority do not match {label}"
        ));
    }
    Ok(())
}

fn verify_parent_graph<P: OwnershipProbe>(
    probe: &P,
    candidate_root: &Path,
    repository: &str,
    policy: &SharedContractPolicy,
    ownership: &WaveOwnership,
    immediate_parent: &str,
    immediate_base_oid: &str,
) -> Result<(), String> {
    let mut branch = immediate_parent.to_owned();
    let mut head_oid = immediate_base_oid.to_owned();
    let mut waves = Vec::new();
    let mut seen = BTreeSet::new();
    while branch != policy.shared_root_branch {
        if !seen.insert(branch.clone()) {
            return Err("Git Town ownership parent graph contains a cycle".to_owned());
        }
        let parent_ownership = policy.wave_for_branch(&branch)?;
        waves.push(parent_ownership.wave.clone());
        let local_head = probe.resolve_commit(candidate_root, &branch)?;
        validate_hash(&local_head, "parent graph head OID").map_err(|error| error.to_string())?;
        if local_head != head_oid {
            return Err(format!(
                "Git Town parent graph head for {branch} changed during verification"
            ));
        }
        let parent = probe.git_town_parent(candidate_root, &branch)?;
        validate_git_ref(&parent, "Git Town ancestor parent").map_err(|error| error.to_string())?;
        if !policy.parent_is_allowed(parent_ownership, &parent) {
            return Err(format!(
                "{} cannot use Git Town parent {parent} as ownership authority",
                parent_ownership.wave
            ));
        }
        let base_oid = probe.resolve_commit(candidate_root, &parent)?;
        validate_hash(&base_oid, "parent graph base OID").map_err(|error| error.to_string())?;
        if base_oid == head_oid {
            return Err(format!(
                "Git Town parent graph branch {branch} cannot use its HEAD as base"
            ));
        }
        let pull_request = probe.open_pull_request(repository, &branch)?;
        verify_pr_authority(
            &pull_request,
            repository,
            &branch,
            &head_oid,
            &parent,
            &base_oid,
            &format!("parent graph branch {branch}"),
        )?;
        if !probe.is_ancestor(candidate_root, &base_oid, &head_oid)? {
            return Err(format!(
                "verified ownership base is not an ancestor of parent graph branch {branch}"
            ));
        }
        branch = parent;
        head_oid = base_oid;
    }

    let allowed = match ownership.wave.as_str() {
        "w5" => waves.is_empty(),
        "w6" => waves.is_empty() || waves == ["w5"],
        "w7" => waves.is_empty() || waves == ["w6", "w5"],
        _ => false,
    };
    if !allowed {
        return Err(format!(
            "{} Git Town parent graph does not match a sibling or fully linearized authority chain",
            ownership.wave
        ));
    }
    Ok(())
}

pub fn read_policy(root: &Path) -> Result<SharedContractPolicy, String> {
    let bytes =
        fs::read(root.join(POLICY_PATH)).map_err(|error| format!("cannot read policy: {error}"))?;
    parse_policy(&bytes)
}

fn parse_policy(bytes: &[u8]) -> Result<SharedContractPolicy, String> {
    let policy: SharedContractPolicy =
        serde_json::from_slice(bytes).map_err(|error| format!("invalid policy JSON: {error}"))?;
    policy.validate()?;
    Ok(policy)
}

pub fn check_changed_paths(
    policy: &SharedContractPolicy,
    wave: &str,
    paths: &[String],
) -> Result<(), String> {
    let ownership = policy.wave(wave)?;
    let mut violations = Vec::new();
    for path in paths {
        validate_relative_path(Path::new(path))?;
        if path == &ownership.manifest_path {
            continue;
        }
        if ownership
            .allowed_protected_paths
            .binary_search(path)
            .is_ok()
        {
            continue;
        }
        let candidate = Path::new(path);
        if is_authoritative_manifest_path(candidate) {
            violations.push(format!("{path} (foreign delivery authority)"));
            continue;
        }
        if policy.protected_paths.binary_search(path).is_ok()
            || policy
                .protected_prefixes
                .iter()
                .any(|prefix| path_matches_prefix(path, prefix))
            || ownership
                .additional_protected_paths
                .binary_search(path)
                .is_ok()
        {
            violations.push(format!("{path} (shared-root authority)"));
            continue;
        }
        if let Some(implementation) = policy.implementation_path(path) {
            if implementation.at_prefix_root {
                violations.push(format!(
                    "{path} (implementation prefix root cannot become a symlink, gitlink, or file)"
                ));
                continue;
            }
            match implementation.owner {
                ImplementationOwner::Wave(owner) if owner == wave => continue,
                ImplementationOwner::Wave(owner) => {
                    violations.push(format!("{path} (owned by {owner})"));
                }
                ImplementationOwner::Frozen => {
                    violations.push(format!("{path} (frozen pre-wave implementation)"));
                }
            }
            continue;
        }
        if policy.is_documentation_path(path) {
            continue;
        }
        violations.push(format!("{path} (unowned by any implementation wave)"));
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{wave} changed paths outside its positive ownership partition; return these changes to the owning branch (shared root {}):\n{}",
            policy.shared_root_branch,
            violations.join("\n")
        ))
    }
}

fn verify_checked_in_manifest(bytes: &[u8], ownership: &WaveOwnership) -> Result<(), String> {
    let manifest: DeliveryManifest = serde_json::from_slice(bytes)
        .map_err(|error| format!("{} authority is invalid JSON: {error}", ownership.wave))?;
    manifest.validate().map_err(|error| error.to_string())?;
    if manifest.wave != ownership.wave
        || !manifest
            .contract_fingerprints
            .iter()
            .any(|fingerprint| fingerprint.path == ownership.manifest_path)
    {
        return Err(format!(
            "{} authority does not declare and fingerprint its selected path",
            ownership.wave
        ));
    }
    Ok(())
}

fn repository_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot locate repository root".to_owned())
}

fn branch_matches_stem(branch: &str, stem: &str) -> bool {
    branch == stem
        || branch
            .strip_prefix(stem)
            .is_some_and(|suffix| suffix.starts_with('-') && suffix.len() > 1)
}

fn validate_sorted_paths(paths: &[String]) -> Result<(), String> {
    validate_sorted_relative_paths(paths, "protected paths")
}

fn validate_sorted_relative_paths(paths: &[String], label: &str) -> Result<(), String> {
    validate_sorted_strings(paths, label)?;
    for path in paths {
        validate_relative_path(Path::new(path))?;
    }
    Ok(())
}

fn validate_sorted_directory_prefixes(prefixes: &[String], label: &str) -> Result<(), String> {
    validate_sorted_strings(prefixes, label)?;
    for prefix in prefixes {
        if !prefix.ends_with('/') {
            return Err(format!("{label} entry {prefix} is not a directory prefix"));
        }
        validate_relative_path(Path::new(prefix.trim_end_matches('/')))?;
    }
    Ok(())
}

fn validate_sorted_prefixes(prefixes: &[String]) -> Result<(), String> {
    validate_sorted_strings(prefixes, "protected prefixes")?;
    for prefix in prefixes {
        if !prefix.ends_with('/') && !prefix.ends_with('_') {
            return Err(format!("protected prefix {prefix} has no boundary suffix"));
        }
        validate_relative_path(Path::new(prefix.trim_end_matches(['/', '_'])))?;
    }
    Ok(())
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    path.starts_with(prefix)
        || prefix
            .strip_suffix('/')
            .is_some_and(|prefix_root| path == prefix_root)
}

fn validate_sorted_strings(values: &[String], label: &str) -> Result<(), String> {
    if values.is_empty() || values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(format!("{label} must be nonempty, sorted, and unique"));
    }
    Ok(())
}

fn validate_sorted_strings_allow_empty(values: &[String], label: &str) -> Result<(), String> {
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(format!("{label} must be sorted and unique"));
    }
    Ok(())
}

fn validate_sorted_values<T: Ord>(values: &[T], label: &str) -> Result<(), String> {
    if values.is_empty() || values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(format!("{label} must be nonempty, sorted, and unique"));
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe policy path {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::BTreeMap,
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    const AUTHORITY_ROOT: &str = "/authority";
    const CANDIDATE_ROOT: &str = "/candidate";
    const BASE_OID: &str = "1111111111111111111111111111111111111111";
    const HEAD_OID: &str = "2222222222222222222222222222222222222222";
    const OTHER_OID: &str = "3333333333333333333333333333333333333333";
    const ROOT_OID: &str = "4444444444444444444444444444444444444444";
    const REPOSITORY: &str = "github.com/vicondoa/d2b";
    static NEXT_TEST_REPOSITORY: AtomicU64 = AtomicU64::new(1);

    struct TestRepository {
        root: PathBuf,
    }

    impl TestRepository {
        fn new(name: &str) -> Self {
            let executable = std::env::current_exe().expect("current test executable");
            let parent = executable.parent().expect("test executable directory");
            let unique = NEXT_TEST_REPOSITORY.fetch_add(1, Ordering::Relaxed);
            let root = parent.join(format!(
                "wave-policy-{name}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir(&root).expect("create test repository");
            run_test_git(&root, &["init", "--quiet", "--initial-branch=main"]);
            run_test_git(&root, &["config", "user.email", "test@example.invalid"]);
            run_test_git(&root, &["config", "user.name", "Wave Policy Test"]);
            Self { root }
        }

        fn write(&self, path: &str, bytes: &[u8]) {
            let path = self.root.join(path);
            std::fs::create_dir_all(path.parent().expect("test file parent"))
                .expect("create test file parent");
            std::fs::write(path, bytes).expect("write test file");
        }

        fn commit(&self, message: &str) -> String {
            run_test_git(&self.root, &["add", "--all"]);
            self.commit_index(message)
        }

        fn commit_index(&self, message: &str) -> String {
            run_test_git(&self.root, &["commit", "--quiet", "-m", message]);
            run_test_git(&self.root, &["rev-parse", "HEAD"])
        }
    }

    impl Drop for TestRepository {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).expect("remove test repository");
        }
    }

    fn run_test_git(root: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .arg("--no-replace-objects")
            .arg("-C")
            .arg(root)
            .args(arguments)
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .output()
            .expect("run test Git command");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("test Git output is UTF-8")
            .trim()
            .to_owned()
    }

    fn policy() -> SharedContractPolicy {
        read_policy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("repository root"),
        )
        .expect("checked-in policy")
    }

    fn ordinary_pr(
        branch: &str,
        head_oid: &str,
        parent: &str,
        base_oid: &str,
    ) -> OrdinaryPullRequest {
        OrdinaryPullRequest {
            repository: REPOSITORY.to_owned(),
            state: PullRequestState::Open,
            base_ref: parent.to_owned(),
            base_oid: base_oid.to_owned(),
            head_repository: REPOSITORY.to_owned(),
            head_ref: branch.to_owned(),
            head_oid: head_oid.to_owned(),
            is_in_merge_queue: false,
        }
    }

    struct FakeProbe {
        branch: String,
        parent: String,
        parent_oid: String,
        authority_oid: String,
        head_oid: String,
        pull_request: OrdinaryPullRequest,
        ancestor: bool,
        changed_paths: Vec<String>,
        ancestor_parents: BTreeMap<String, String>,
        ancestor_refs: BTreeMap<String, String>,
        ancestor_pull_requests: BTreeMap<String, OrdinaryPullRequest>,
        blob_reads: RefCell<Vec<(PathBuf, String, String)>>,
        rewrite_metadata: BTreeMap<PathBuf, String>,
    }

    impl FakeProbe {
        fn valid() -> Self {
            Self {
                branch: "adr0045-w5-control".to_owned(),
                parent: "adr0045-post-w4-contracts".to_owned(),
                parent_oid: BASE_OID.to_owned(),
                authority_oid: BASE_OID.to_owned(),
                head_oid: HEAD_OID.to_owned(),
                pull_request: ordinary_pr(
                    "adr0045-w5-control",
                    HEAD_OID,
                    "adr0045-post-w4-contracts",
                    BASE_OID,
                ),
                ancestor: true,
                changed_paths: vec!["packages/d2bd/src/service_v2.rs".to_owned()],
                ancestor_parents: BTreeMap::new(),
                ancestor_refs: BTreeMap::new(),
                ancestor_pull_requests: BTreeMap::new(),
                blob_reads: RefCell::new(Vec::new()),
                rewrite_metadata: BTreeMap::new(),
            }
        }
    }

    impl OwnershipProbe for FakeProbe {
        fn canonical_root(&self, root: &Path) -> Result<PathBuf, String> {
            Ok(root.to_path_buf())
        }

        fn reject_history_rewrites(&self, root: &Path) -> Result<(), String> {
            if let Some(label) = self.rewrite_metadata.get(root) {
                Err(format!("repository contains forbidden {label} metadata"))
            } else {
                Ok(())
            }
        }

        fn repository_identity(&self, _root: &Path) -> Result<String, String> {
            Ok(REPOSITORY.to_owned())
        }

        fn is_dirty(&self, _root: &Path) -> Result<bool, String> {
            Ok(false)
        }

        fn current_branch(&self, root: &Path) -> Result<String, String> {
            if root == Path::new(CANDIDATE_ROOT) {
                Ok(self.branch.clone())
            } else {
                Err("current branch requested outside candidate".to_owned())
            }
        }

        fn git_town_parent(&self, root: &Path, branch: &str) -> Result<String, String> {
            if root == Path::new(CANDIDATE_ROOT) && branch == self.branch {
                Ok(self.parent.clone())
            } else if root == Path::new(CANDIDATE_ROOT) {
                self.ancestor_parents
                    .get(branch)
                    .cloned()
                    .ok_or_else(|| "unexpected Git Town parent request".to_owned())
            } else {
                Err("unexpected Git Town parent request".to_owned())
            }
        }

        fn resolve_commit(&self, root: &Path, revision: &str) -> Result<String, String> {
            if root == Path::new(AUTHORITY_ROOT) && revision == "HEAD" {
                return Ok(self.authority_oid.clone());
            }
            if root == Path::new(CANDIDATE_ROOT) {
                if revision == "HEAD" || revision == self.branch {
                    return Ok(self.head_oid.clone());
                }
                if revision == self.parent {
                    return Ok(self.parent_oid.clone());
                }
                if let Some(oid) = self.ancestor_refs.get(revision) {
                    return Ok(oid.clone());
                }
            }
            Err(format!("unexpected revision {revision}"))
        }

        fn is_ancestor(&self, root: &Path, base: &str, head: &str) -> Result<bool, String> {
            let _ = (base, head);
            if root == Path::new(CANDIDATE_ROOT) {
                Ok(self.ancestor)
            } else {
                Err("unexpected ancestry request".to_owned())
            }
        }

        fn open_pull_request(
            &self,
            repository: &str,
            branch: &str,
        ) -> Result<OrdinaryPullRequest, String> {
            if repository == REPOSITORY && branch == self.branch {
                Ok(self.pull_request.clone())
            } else if repository == REPOSITORY {
                self.ancestor_pull_requests
                    .get(branch)
                    .cloned()
                    .ok_or_else(|| "unexpected pull request lookup".to_owned())
            } else {
                Err("unexpected pull request lookup".to_owned())
            }
        }

        fn tracked_blob(&self, root: &Path, commit: &str, path: &str) -> Result<Vec<u8>, String> {
            self.blob_reads.borrow_mut().push((
                root.to_path_buf(),
                commit.to_owned(),
                path.to_owned(),
            ));
            if root == Path::new(AUTHORITY_ROOT)
                && commit == self.authority_oid
                && path == POLICY_PATH
            {
                return Ok(include_bytes!("../../../delivery/shared-contracts.json").to_vec());
            }
            if root == Path::new(CANDIDATE_ROOT)
                && commit == self.head_oid
                && path == "delivery/manifests/w5.json"
            {
                return Ok(include_bytes!("../../../delivery/manifests/w5.json").to_vec());
            }
            if root == Path::new(CANDIDATE_ROOT)
                && commit == self.head_oid
                && path == "delivery/manifests/w6.json"
            {
                return Ok(include_bytes!("../../../delivery/manifests/w6.json").to_vec());
            }
            if root == Path::new(CANDIDATE_ROOT)
                && commit == self.head_oid
                && path == "delivery/manifests/w7.json"
            {
                return Ok(include_bytes!("../../../delivery/manifests/w7.json").to_vec());
            }
            if root == Path::new(CANDIDATE_ROOT) && commit == self.head_oid && path == POLICY_PATH {
                return Ok(br#"{"schema_version":999,"waves":[]}"#.to_vec());
            }
            Err(format!("unexpected blob {commit}:{path}"))
        }

        fn changed_paths(
            &self,
            root: &Path,
            base: &str,
            head: &str,
        ) -> Result<Vec<String>, String> {
            if root == Path::new(CANDIDATE_ROOT) && base == self.parent_oid && head == self.head_oid
            {
                Ok(self.changed_paths.clone())
            } else {
                Err("unexpected diff request".to_owned())
            }
        }
    }

    #[test]
    fn own_manifest_and_wave_local_files_are_allowed() {
        let policy = policy();
        check_changed_paths(
            &policy,
            "w5",
            &[
                "CHANGELOG.md".to_owned(),
                "delivery/manifests/w5.json".to_owned(),
                "docs/reference/daemon-api.md".to_owned(),
                "packages/d2bd/src/service_v2.rs".to_owned(),
            ],
        )
        .expect("wave-local paths");
    }

    #[test]
    fn positive_partition_rejects_unowned_frozen_and_prefix_root_paths() {
        let policy = policy();
        for wave in ["w5", "w6", "w7"] {
            let paths = [
                "docs/adr/0099-wave-escape.md",
                "packages/d2b-provider-runtime-local/src/lib.rs",
                "packages/d2bd-escape/src/lib.rs",
                "scripts/wave-escape.sh",
            ]
            .map(str::to_owned);
            let error = check_changed_paths(&policy, wave, &paths)
                .expect_err("unowned and frozen implementation paths");
            for path in paths {
                assert!(error.contains(&path), "{error}");
            }
        }

        for (wave, roots) in [
            ("w5", ["packages/d2bd", "packages/d2b-userd"]),
            ("w6", ["packages/d2b-userd", "nixos-modules"]),
            ("w7", ["nixos-modules", "packages/d2bd"]),
        ] {
            let roots = roots.map(str::to_owned);
            let error = check_changed_paths(&policy, wave, &roots)
                .expect_err("symlink or gitlink implementation root");
            for root in roots {
                assert!(error.contains(&root), "{error}");
                assert!(error.contains("prefix root"), "{error}");
            }
        }
    }

    #[test]
    fn each_wave_rejects_other_wave_implementation_prefixes() {
        let policy = policy();
        for (wave, allowed, foreign) in [
            (
                "w5",
                "packages/d2bd/src/service_v2.rs",
                [
                    "packages/d2b-userd/src/main.rs",
                    "nixos-modules/processes-json.nix",
                ],
            ),
            (
                "w6",
                "packages/d2b-wayland-proxy/src/control.rs",
                [
                    "packages/d2b-priv-broker/src/service_v2.rs",
                    "nixos-modules/processes-json.nix",
                ],
            ),
            (
                "w7",
                "nixos-modules/processes-json.nix",
                [
                    "packages/d2bd/src/service_v2.rs",
                    "packages/d2b-clipd/src/protocol.rs",
                ],
            ),
        ] {
            check_changed_paths(&policy, wave, &[allowed.to_owned()])
                .unwrap_or_else(|error| panic!("{wave} rejected its own prefix: {error}"));
            let foreign = foreign.map(str::to_owned);
            let error = check_changed_paths(&policy, wave, &foreign)
                .expect_err("foreign wave implementation paths");
            for path in foreign {
                assert!(error.contains(&path), "{error}");
            }
        }
    }

    #[test]
    fn shared_lock_contract_and_foreign_manifest_are_rejected() {
        let policy = policy();
        let error = check_changed_paths(
            &policy,
            "w6",
            &[
                "packages/Cargo.lock".to_owned(),
                "packages/d2b-contracts/proto/v2/user.proto".to_owned(),
                "delivery/manifests/w7.json".to_owned(),
            ],
        )
        .expect_err("shared paths");
        for path in ["Cargo.lock", "user.proto", "w7.json"] {
            assert!(error.contains(path), "{error}");
        }
    }

    #[test]
    fn provider_registry_extension_is_owned_only_by_declarative_wave() {
        let policy = policy();
        let path = "packages/d2b-contracts/src/provider_registry_v2.rs".to_owned();
        check_changed_paths(&policy, "w7", std::slice::from_ref(&path))
            .expect("w7 provider registry ownership");
        assert!(check_changed_paths(&policy, "w5", &[path]).is_err());
    }

    #[test]
    fn fully_linearized_parent_graph_is_verified_for_w7() {
        let mut probe = FakeProbe::valid();
        probe.branch = "adr0045-w7-host-emission".to_owned();
        probe.parent = "adr0045-w6-user-services".to_owned();
        probe.parent_oid = BASE_OID.to_owned();
        probe.pull_request = ordinary_pr(&probe.branch, HEAD_OID, &probe.parent, BASE_OID);
        probe.changed_paths = vec!["nixos-modules/processes-json.nix".to_owned()];
        probe.ancestor_parents.insert(
            "adr0045-w6-user-services".to_owned(),
            "adr0045-w5-control".to_owned(),
        );
        probe.ancestor_parents.insert(
            "adr0045-w5-control".to_owned(),
            "adr0045-post-w4-contracts".to_owned(),
        );
        probe
            .ancestor_refs
            .insert("adr0045-w5-control".to_owned(), OTHER_OID.to_owned());
        probe
            .ancestor_refs
            .insert("adr0045-post-w4-contracts".to_owned(), ROOT_OID.to_owned());
        probe.ancestor_pull_requests.insert(
            "adr0045-w6-user-services".to_owned(),
            ordinary_pr(
                "adr0045-w6-user-services",
                BASE_OID,
                "adr0045-w5-control",
                OTHER_OID,
            ),
        );
        probe.ancestor_pull_requests.insert(
            "adr0045-w5-control".to_owned(),
            ordinary_pr(
                "adr0045-w5-control",
                OTHER_OID,
                "adr0045-post-w4-contracts",
                ROOT_OID,
            ),
        );

        let verified =
            verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
                .expect("linearized W7 graph");
        assert_eq!(verified.wave, "w7");
        assert_eq!(verified.base_oid, BASE_OID);
    }

    #[test]
    fn partial_linearization_is_not_a_valid_w7_parent_graph() {
        let mut probe = FakeProbe::valid();
        probe.branch = "adr0045-w7-host-emission".to_owned();
        probe.parent = "adr0045-w6-user-services".to_owned();
        probe.parent_oid = BASE_OID.to_owned();
        probe.pull_request = ordinary_pr(&probe.branch, HEAD_OID, &probe.parent, BASE_OID);
        probe.changed_paths = vec!["nixos-modules/processes-json.nix".to_owned()];
        probe.ancestor_parents.insert(
            "adr0045-w6-user-services".to_owned(),
            "adr0045-post-w4-contracts".to_owned(),
        );
        probe
            .ancestor_refs
            .insert("adr0045-post-w4-contracts".to_owned(), OTHER_OID.to_owned());
        probe.ancestor_pull_requests.insert(
            "adr0045-w6-user-services".to_owned(),
            ordinary_pr(
                "adr0045-w6-user-services",
                BASE_OID,
                "adr0045-post-w4-contracts",
                OTHER_OID,
            ),
        );

        let error = verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
            .expect_err("partial W7 linearization");
        assert!(
            error.contains("fully linearized authority chain"),
            "{error}"
        );
    }

    #[test]
    fn trusted_parent_policy_rejects_candidate_checker_and_policy_changes() {
        let mut probe = FakeProbe::valid();
        probe.changed_paths = vec![
            POLICY_PATH.to_owned(),
            "packages/xtask/src/wave_policy.rs".to_owned(),
        ];
        let error = verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
            .expect_err("candidate-controlled checker and policy");
        assert!(error.contains(POLICY_PATH), "{error}");
        assert!(
            error.contains("packages/xtask/src/wave_policy.rs"),
            "{error}"
        );
        let reads = probe.blob_reads.borrow();
        assert!(reads.iter().any(|(root, commit, path)| {
            root == Path::new(AUTHORITY_ROOT) && commit == BASE_OID && path == POLICY_PATH
        }));
        assert!(!reads.iter().any(|(root, commit, path)| {
            root == Path::new(CANDIDATE_ROOT) && commit == HEAD_OID && path == POLICY_PATH
        }));
    }

    #[test]
    fn replace_metadata_fails_before_policy_manifest_or_diff_reads() {
        for (root, label) in [
            (AUTHORITY_ROOT, "refs/replace trusted policy"),
            (CANDIDATE_ROOT, "refs/replace candidate manifest and diff"),
        ] {
            let mut probe = FakeProbe::valid();
            probe
                .rewrite_metadata
                .insert(PathBuf::from(root), label.to_owned());
            let error =
                verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
                    .expect_err("replacement metadata");
            assert!(error.contains(label), "{error}");
            assert!(probe.blob_reads.borrow().is_empty());
        }
    }

    #[test]
    fn process_probe_ignores_substituted_objects_and_rejects_rewrite_metadata() {
        let repository = TestRepository::new("replace");
        repository.write(POLICY_PATH, b"trusted policy\n");
        let base = repository.commit("trusted base");

        repository.write("delivery/manifests/w5.json", b"trusted manifest\n");
        repository.write("packages/d2bd/src/real.rs", b"real diff\n");
        let head = repository.commit("real candidate");

        run_test_git(
            &repository.root,
            &["checkout", "--quiet", "-b", "attacker", &base],
        );
        repository.write(POLICY_PATH, b"substituted policy\n");
        repository.write("delivery/manifests/w5.json", b"substituted manifest\n");
        let replacement = repository.commit("replacement objects");
        run_test_git(&repository.root, &["replace", &base, &replacement]);
        run_test_git(&repository.root, &["replace", &head, &replacement]);

        let probe = ProcessOwnershipProbe::default();
        assert_eq!(
            probe
                .tracked_blob(&repository.root, &base, POLICY_PATH)
                .expect("read trusted policy"),
            b"trusted policy\n"
        );
        assert_eq!(
            probe
                .tracked_blob(&repository.root, &head, "delivery/manifests/w5.json")
                .expect("read trusted manifest"),
            b"trusted manifest\n"
        );
        let changed = probe
            .changed_paths(&repository.root, &base, &head)
            .expect("read real diff");
        assert!(
            changed
                .iter()
                .any(|path| path == "packages/d2bd/src/real.rs")
        );
        let error = probe
            .reject_history_rewrites(&repository.root)
            .expect_err("replace refs");
        assert!(error.contains("refs/replace"), "{error}");

        run_test_git(&repository.root, &["replace", "-d", &base]);
        run_test_git(&repository.root, &["replace", "-d", &head]);
        let common_dir = repository.root.join(".git");
        repository.write(".git/info/grafts", format!("{base}\n").as_bytes());
        let error = probe
            .reject_history_rewrites(&repository.root)
            .expect_err("graft metadata");
        assert!(error.contains("graft"), "{error}");
        std::fs::remove_file(common_dir.join("info/grafts")).expect("remove graft metadata");

        repository.write(".git/shallow", format!("{base}\n").as_bytes());
        let error = probe
            .reject_history_rewrites(&repository.root)
            .expect_err("shallow metadata");
        assert!(error.contains("shallow"), "{error}");
    }

    #[test]
    fn local_ignore_submodules_cannot_hide_gitlink_ownership_changes() {
        let repository = TestRepository::new("gitlinks");
        repository.write("marker", b"base\n");
        repository.write("packages/unowned", b"regular file\n");
        let base = repository.commit("base");

        std::fs::remove_file(repository.root.join("packages/unowned"))
            .expect("remove regular file before gitlink type change");
        for path in [
            "packages/d2b-contracts",
            "packages/d2b-userd",
            "packages/unowned",
        ] {
            run_test_git(
                &repository.root,
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("160000,{base},{path}"),
                ],
            );
        }
        let head = repository.commit_index("gitlinks");
        run_test_git(
            &repository.root,
            &["config", "diff.ignoreSubmodules", "all"],
        );

        let hidden = run_test_git(
            &repository.root,
            &["diff", "--name-only", &base, &head, "--"],
        );
        assert!(!hidden.contains("packages/d2b-contracts"));
        assert!(!hidden.contains("packages/d2b-userd"));

        let probe = ProcessOwnershipProbe::default();
        let changed = probe
            .changed_paths(&repository.root, &base, &head)
            .expect("ownership diff");
        for path in [
            "packages/d2b-contracts",
            "packages/d2b-userd",
            "packages/unowned",
        ] {
            assert!(changed.iter().any(|changed| changed == path), "{changed:?}");
        }
        let error = check_changed_paths(&policy(), "w5", &changed)
            .expect_err("protected, foreign, and unowned gitlink roots");
        for path in changed {
            assert!(error.contains(&path), "{error}");
        }

        let canonical = GitProbe::new(ProcessCommandOutput)
            .canonical_diff(&repository.root, &base, &head, &[])
            .expect("canonical diff");
        let canonical = String::from_utf8(canonical).expect("canonical diff is UTF-8");
        for path in [
            "packages/d2b-contracts",
            "packages/d2b-userd",
            "packages/unowned",
        ] {
            assert!(canonical.contains(path), "{canonical}");
        }

        assert!(run_test_git(&repository.root, &["status", "--porcelain=v1"]).is_empty());
        assert!(
            probe
                .is_dirty(&repository.root)
                .expect("forced submodule cleanliness")
        );
    }

    #[test]
    fn fake_git_town_parent_cannot_override_pull_request_authority() {
        let mut probe = FakeProbe::valid();
        probe.parent = "fake-parent".to_owned();
        probe.parent_oid = OTHER_OID.to_owned();
        let error = verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
            .expect_err("fake Git Town parent");
        assert!(
            error.contains("Git Town parent and ordinary GitHub PR base"),
            "{error}"
        );
    }

    #[test]
    fn fake_parent_matching_pull_request_is_not_a_wave_authority() {
        let mut probe = FakeProbe::valid();
        probe.parent = "adr0045-w9-fake".to_owned();
        probe.pull_request.base_ref = probe.parent.clone();
        let error = verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
            .expect_err("foreign parent authority");
        assert!(error.contains("cannot use Git Town parent"), "{error}");
    }

    #[test]
    fn authority_checker_must_be_built_from_exact_parent_commit() {
        let mut probe = FakeProbe::valid();
        probe.authority_oid = OTHER_OID.to_owned();
        let error = verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
            .expect_err("wrong checker authority");
        assert!(error.contains("exact verified parent commit"), "{error}");
    }

    #[test]
    fn head_cannot_be_selected_as_ownership_base() {
        let mut probe = FakeProbe::valid();
        probe.parent_oid = HEAD_OID.to_owned();
        probe.authority_oid = HEAD_OID.to_owned();
        probe.pull_request.base_oid = HEAD_OID.to_owned();
        let error = verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
            .expect_err("HEAD as base");
        assert!(
            error.contains("HEAD cannot be its own ownership base"),
            "{error}"
        );
    }

    #[test]
    fn caller_cannot_select_wave_or_base() {
        let arguments = [
            "check".to_owned(),
            "--wave".to_owned(),
            "w7".to_owned(),
            "--base".to_owned(),
            "HEAD".to_owned(),
        ];
        assert_eq!(parse_candidate_root(&arguments), Err(USAGE.to_owned()));
    }
}
