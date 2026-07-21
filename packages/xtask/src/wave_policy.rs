use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::ExitCode,
};

use serde::{
    Deserialize,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;

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
const W5_BROKER_WIRE_PATH: &str = "packages/d2b-contracts/src/broker_wire.rs";
const W5_PRIVILEGES_PATH: &str = "packages/d2b-core/src/privileges.rs";
const W5_PRIVILEGES_W3_PATH: &str = "packages/d2b-core/src/privileges_w3.rs";
const W5_PRIVILEGES_PARITY_PATH: &str = "packages/d2b-contract-tests/tests/privileges_parity.rs";
const W5_BROKER_DISPOSITIONS_DOC_PATH: &str = "docs/reference/broker-w2-dispositions.md";
const W5_DAEMON_API_PATH: &str = "docs/reference/daemon-api.md";
const W5_PRIVILEGES_DOC_PATH: &str = "docs/reference/privileges.md";
const W5_PRIVILEGES_SCHEMA_PATH: &str = "docs/reference/schemas/v2/privileges.json";
const W5_WIRE_SCHEMA_PATH: &str = "docs/reference/schemas/v2/wire-protocol.json";

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
    pub daemon_typed_methods: Vec<TypedBrokerMethod>,
    pub guest_typed_methods: Vec<TypedBrokerMethod>,
    pub service_dependency_edges: Vec<ServiceDependencyEdge>,
    pub w5_contract_retirements: Vec<W5ContractRetirement>,
    #[serde(default)]
    pub w5_successor_pin_paths: Vec<String>,
    pub w7_contract_test_migrations: Vec<W7ContractTestMigration>,
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
    #[serde(default)]
    pub integration_only_protected_paths: Vec<String>,
    /// Waves whose already-landed implementation prefixes this wave is
    /// authorized to continue editing (a "successor" grant). Empty for a
    /// peer/parallel wave; populated only for an integration wave that
    /// picks up already-merged predecessor territory.
    #[serde(default)]
    pub inherits_prefixes_from: Vec<String>,
    /// A landed trunk ref (e.g. "main") that stands in for the shared root
    /// once this wave's predecessors have already merged and their branches
    /// are gone. `None` for peer waves that still branch off the live
    /// `shared_root_branch`.
    #[serde(default)]
    pub landed_predecessor_ref: Option<String>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(deny_unknown_fields)]
pub struct W5ContractRetirement {
    pub component: String,
    pub diff_policy: String,
    pub operation: String,
    pub source_paths: Vec<String>,
    pub test_selectors: Vec<ContractTestSelector>,
    pub companion_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(deny_unknown_fields)]
pub struct ContractTestSelector {
    pub test_file: String,
    pub test_names: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(deny_unknown_fields)]
pub struct W7ContractTestMigration {
    pub component: String,
    pub test_file: String,
    pub test_names: Vec<String>,
    pub source_paths: Vec<String>,
    pub companion_paths: Vec<String>,
}

impl SharedContractPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 11 {
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
        if waves != BTreeSet::from(["w5", "w6", "w7", "w8"]) || waves.len() != self.waves.len() {
            return Err("shared-contract policy must define exactly w5, w6, w7, and w8".to_owned());
        }
        let expected_branch_stems = BTreeMap::from([
            ("w5", "adr0045-w5"),
            ("w6", "adr0045-w6"),
            ("w7", "adr0045-w7"),
            ("w8", "adr0045-w8-integration"),
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
            // W8 is the integrated W5/W6/W7 successor: its single immediate
            // ownership parent-wave is W7 (mirroring the W6->W5 and W7->W6
            // single-link convention), while `inherits_prefixes_from` below
            // separately grants it the full transitive W5+W6+W7 implementation
            // territory those waves already landed.
            let expected_parent_waves: &[&str] = match wave.wave.as_str() {
                "w5" => &[],
                "w6" => &["w5"],
                "w7" => &["w6"],
                "w8" => &["w7"],
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
            let expected_prefix_inheritance: &[&str] = match wave.wave.as_str() {
                "w5" | "w6" | "w7" => &[],
                "w8" => &["w5", "w6", "w7"],
                _ => unreachable!("wave set was validated"),
            };
            validate_sorted_strings_allow_empty(
                &wave.inherits_prefixes_from,
                "prefix inheritance",
            )?;
            if !wave
                .inherits_prefixes_from
                .iter()
                .map(String::as_str)
                .eq(expected_prefix_inheritance.iter().copied())
            {
                return Err(format!(
                    "{} prefix inheritance does not match the delivery graph",
                    wave.wave
                ));
            }
            let expected_landed_predecessor_ref: Option<&str> = match wave.wave.as_str() {
                "w5" | "w6" | "w7" => None,
                "w8" => Some("main"),
                _ => unreachable!("wave set was validated"),
            };
            if wave.landed_predecessor_ref.as_deref() != expected_landed_predecessor_ref {
                return Err(format!(
                    "{} landed-predecessor ref does not match the delivery graph",
                    wave.wave
                ));
            }
            if let Some(landed_predecessor_ref) = &wave.landed_predecessor_ref {
                validate_git_ref(landed_predecessor_ref, "landed-predecessor ref")
                    .map_err(|error| error.to_string())?;
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
            // A pure integration/successor wave may own zero *new* prefixes of
            // its own (its positive ownership comes entirely from
            // `inherits_prefixes_from`); every other wave must own at least
            // one prefix outright.
            if wave.allowed_prefixes.is_empty() {
                if wave.inherits_prefixes_from.is_empty() {
                    return Err(format!(
                        "{} owns no implementation prefixes and inherits none",
                        wave.wave
                    ));
                }
                validate_sorted_strings_allow_empty(
                    &wave.allowed_prefixes,
                    "allowed implementation prefixes",
                )?;
            } else {
                validate_sorted_directory_prefixes(
                    &wave.allowed_prefixes,
                    "allowed implementation prefixes",
                )?;
            }
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
            if !wave.integration_only_protected_paths.is_empty() {
                validate_sorted_paths(&wave.integration_only_protected_paths)?;
            }
            let expected_integration_only: &[&str] = match wave.wave.as_str() {
                "w5" | "w6" | "w7" => &[],
                "w8" => &[
                    "delivery/README.md",
                    "packages/Cargo.lock",
                    "packages/Cargo.toml",
                    "packages/xtask/tests/delivery_cli.rs",
                    "packages/xtask/tests/delivery_w8.rs",
                    "packages/xtask/tests/policy_workspace.rs",
                ],
                _ => unreachable!("wave set was validated"),
            };
            if !wave
                .integration_only_protected_paths
                .iter()
                .map(String::as_str)
                .eq(expected_integration_only.iter().copied())
            {
                return Err(format!(
                    "{} integration-only protected paths do not match the delivery graph",
                    wave.wave
                ));
            }
        }
        for wave in &self.waves {
            for inherited in &wave.inherits_prefixes_from {
                if !waves.contains(inherited.as_str()) {
                    return Err(format!(
                        "{} inherits prefixes from unknown wave {inherited}",
                        wave.wave
                    ));
                }
                if inherited == &wave.wave {
                    return Err(format!("{} cannot inherit prefixes from itself", wave.wave));
                }
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
        validate_sorted_values(&self.daemon_typed_methods, "typed daemon methods")?;
        validate_sorted_values(&self.guest_typed_methods, "typed guest methods")?;
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
        validate_sorted_values(&self.w5_contract_retirements, "W5 contract retirements")?;
        if self.w5_contract_retirements.len() != 1 {
            return Err(
                "W5 contract retirement authority must contain exactly one migration row"
                    .to_owned(),
            );
        }
        let retirement = &self.w5_contract_retirements[0];
        if retirement.component != "guest-signing"
            || retirement.diff_policy != "guest-control-sign-v1"
            || retirement.operation != "GuestControlSign"
        {
            return Err(
                "W5 contract retirement authority is limited to GuestControlSign".to_owned(),
            );
        }
        validate_sorted_relative_paths(
            &retirement.source_paths,
            "W5 contract retirement source paths",
        )?;
        if !retirement.test_selectors.is_empty() {
            validate_sorted_values(
                &retirement.test_selectors,
                "W5 contract retirement test selectors",
            )?;
        }
        for selector in &retirement.test_selectors {
            validate_relative_path(Path::new(&selector.test_file))?;
            if !selector.test_file.ends_with(".rs") {
                return Err(format!(
                    "W5 contract retirement selector is not a Rust source: {}",
                    selector.test_file
                ));
            }
            validate_sorted_strings(&selector.test_names, "W5 contract retirement test names")?;
            for name in &selector.test_names {
                validate_identifier(name, "W5 contract retirement test name")
                    .map_err(|error| error.to_string())?;
            }
        }
        validate_sorted_relative_paths(
            &retirement.companion_paths,
            "W5 contract retirement companion paths",
        )?;
        let retirement_paths = retirement
            .source_paths
            .iter()
            .chain(
                retirement
                    .test_selectors
                    .iter()
                    .map(|selector| &selector.test_file),
            )
            .chain(retirement.companion_paths.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        validate_sorted_relative_paths(&self.w5_successor_pin_paths, "W5 successor pin paths")?;
        if self
            .w5_successor_pin_paths
            .iter()
            .any(|path| !path.starts_with("tests/golden/pinned/") || !path.ends_with(".txt"))
        {
            return Err(
                "W5 successor pin paths must be exact tests/golden/pinned/*.txt files".to_owned(),
            );
        }
        let w5_protected_paths = retirement_paths
            .iter()
            .chain(self.w5_successor_pin_paths.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        let w5 = self
            .wave("w5")
            .map_err(|_| "shared-contract policy has no W5 owner".to_owned())?;
        if w5
            .additional_protected_paths
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            != w5_protected_paths
            || w5
                .allowed_protected_paths
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                != w5_protected_paths
        {
            return Err(
                "W5 protected exceptions must exactly match the contract retirement inventory"
                    .to_owned(),
            );
        }
        validate_sorted_values(
            &self.w7_contract_test_migrations,
            "W7 contract-test migrations",
        )?;
        let mut migration_test_files = BTreeSet::new();
        let mut migration_companions = BTreeSet::new();
        for migration in &self.w7_contract_test_migrations {
            validate_identifier(&migration.component, "W7 migration component")
                .map_err(|error| error.to_string())?;
            validate_relative_path(Path::new(&migration.test_file))?;
            if !migration
                .test_file
                .starts_with("packages/d2b-contract-tests/tests/")
                || !migration.test_file.ends_with(".rs")
            {
                return Err(format!(
                    "W7 migration test file is outside the frozen contract-test crate: {}",
                    migration.test_file
                ));
            }
            validate_sorted_strings(&migration.test_names, "W7 migration test names")?;
            for name in &migration.test_names {
                validate_identifier(name, "W7 migration test name")
                    .map_err(|error| error.to_string())?;
            }
            validate_sorted_relative_paths(&migration.source_paths, "W7 migration source paths")?;
            validate_sorted_relative_paths_allow_empty(
                &migration.companion_paths,
                "W7 migration companion paths",
            )?;
            migration_test_files.insert(migration.test_file.clone());
            migration_companions.extend(migration.companion_paths.iter().cloned());
        }
        let w7 = self
            .wave("w7")
            .map_err(|_| "shared-contract policy has no W7 owner".to_owned())?;
        let allowed_contract_tests = w7
            .allowed_protected_paths
            .iter()
            .filter(|path| path.starts_with("packages/d2b-contract-tests/tests/"))
            .cloned()
            .collect::<BTreeSet<_>>();
        if allowed_contract_tests != migration_test_files {
            return Err(
                "W7 contract-test exceptions must exactly match the migration inventory".to_owned(),
            );
        }
        let allowed_companions = w7
            .allowed_protected_paths
            .iter()
            .filter(|path| {
                path.as_str() == "tests/migration-ledger.toml"
                    || path.starts_with("tests/migration-state.d/")
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        if allowed_companions != migration_companions {
            return Err(
                "W7 migration pin exceptions must exactly match the migration inventory".to_owned(),
            );
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
            for path in wave
                .allowed_protected_paths
                .iter()
                .chain(&wave.integration_only_protected_paths)
            {
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
                "branch {branch} does not identify a governed w5, w6, w7, or w8 wave"
            )),
            _ => Err(format!("branch {branch} ambiguously identifies a wave")),
        }
    }

    fn parent_is_allowed(&self, ownership: &WaveOwnership, parent: &str) -> bool {
        if parent == self.shared_root_branch {
            return ownership.landed_predecessor_ref.is_none();
        }
        if ownership.landed_predecessor_ref.as_deref() == Some(parent) {
            return true;
        }
        self.wave_for_branch(parent).is_ok_and(|parent_wave| {
            ownership
                .allowed_parent_waves
                .binary_search(&parent_wave.wave)
                .is_ok()
        })
    }

    /// Whether `wave` may edit a prefix whose founding owner is `owner`,
    /// either because it is the founding owner itself or because it is an
    /// authorized successor that inherited that owner's prefixes.
    fn wave_may_edit_owned_prefix(&self, wave: &str, owner: &str) -> bool {
        owner == wave
            || self
                .wave(wave)
                .is_ok_and(|ownership| ownership.inherits_prefixes_from.iter().any(|p| p == owner))
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
    check_changed_paths_for_branch(&policy, &ownership.wave, Some(&branch), &paths)?;
    verify_w5_contract_retirement(
        probe,
        &candidate_root,
        &base_oid,
        &head_oid,
        &policy,
        ownership,
        &paths,
    )?;
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
    while branch != policy.shared_root_branch
        && ownership.landed_predecessor_ref.as_deref() != Some(branch.as_str())
    {
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
        "w8" => waves.is_empty() || waves == ["w7", "w6", "w5"],
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
    check_changed_paths_for_branch(policy, wave, None, paths)
}

fn check_changed_paths_for_branch(
    policy: &SharedContractPolicy,
    wave: &str,
    branch: Option<&str>,
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
        if branch == Some(ownership.branch_stem.as_str())
            && ownership
                .integration_only_protected_paths
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
                ImplementationOwner::Wave(owner)
                    if policy.wave_may_edit_owned_prefix(wave, owner) =>
                {
                    continue;
                }
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

fn verify_w5_contract_retirement<P: OwnershipProbe>(
    probe: &P,
    candidate_root: &Path,
    base_oid: &str,
    head_oid: &str,
    policy: &SharedContractPolicy,
    ownership: &WaveOwnership,
    changed_paths: &[String],
) -> Result<(), String> {
    if ownership.wave != "w5" {
        return Ok(());
    }
    let retirement = policy
        .w5_contract_retirements
        .first()
        .ok_or_else(|| "W5 contract retirement policy is missing".to_owned())?;
    let authorized = retirement
        .source_paths
        .iter()
        .chain(
            retirement
                .test_selectors
                .iter()
                .map(|selector| &selector.test_file),
        )
        .chain(retirement.companion_paths.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let changed_retirement_paths = changed_paths
        .iter()
        .filter(|path| authorized.contains(*path))
        .cloned()
        .collect::<BTreeSet<_>>();
    if changed_retirement_paths.is_empty() {
        return Ok(());
    }
    if changed_retirement_paths != authorized {
        return Err(
            "GuestControlSign retirement must update its exact complete path inventory".to_owned(),
        );
    }

    let mut parent_blobs = BTreeMap::new();
    let mut candidate_blobs = BTreeMap::new();
    for path in &authorized {
        let parent = probe
            .tracked_blob(candidate_root, base_oid, path)
            .map_err(|_| format!("GuestControlSign retirement parent blob is missing: {path}"))?;
        let candidate = probe
            .tracked_blob(candidate_root, head_oid, path)
            .map_err(|_| {
                format!("GuestControlSign retirement candidate blob is missing: {path}")
            })?;
        parent_blobs.insert(path.clone(), parent);
        candidate_blobs.insert(path.clone(), candidate);
    }
    verify_w5_contract_retirement_contents(retirement, &parent_blobs, &candidate_blobs)
}

fn verify_w5_contract_retirement_contents(
    retirement: &W5ContractRetirement,
    parent_blobs: &BTreeMap<String, Vec<u8>>,
    candidate_blobs: &BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    if retirement.diff_policy != "guest-control-sign-v1"
        || retirement.operation != "GuestControlSign"
    {
        return Err("unsupported W5 contract retirement diff policy".to_owned());
    }

    let mut expected = BTreeMap::new();
    for path in retirement
        .source_paths
        .iter()
        .chain(
            retirement
                .test_selectors
                .iter()
                .map(|selector| &selector.test_file),
        )
        .chain(retirement.companion_paths.iter())
        .filter(|path| path.as_str() != W5_DAEMON_API_PATH)
    {
        let parent = parent_blobs
            .get(path)
            .ok_or_else(|| format!("missing parent retirement fixture: {path}"))?;
        expected.insert(
            path.clone(),
            expected_guest_signing_retirement_blob(path, parent)?,
        );
    }

    let parent_daemon_api = parent_blobs
        .get(W5_DAEMON_API_PATH)
        .ok_or_else(|| "missing parent daemon API retirement fixture".to_owned())?;
    let expected_broker_wire = expected
        .get(W5_BROKER_WIRE_PATH)
        .ok_or_else(|| "missing transformed broker wire retirement fixture".to_owned())?;
    expected.insert(
        W5_DAEMON_API_PATH.to_owned(),
        expected_daemon_api_retirement(parent_daemon_api, expected_broker_wire)?,
    );

    for (path, expected_bytes) in expected {
        let actual = candidate_blobs
            .get(&path)
            .ok_or_else(|| format!("missing candidate retirement fixture: {path}"))?;
        let matches = if path == W5_PRIVILEGES_SCHEMA_PATH || path == W5_WIRE_SCHEMA_PATH {
            parse_json_without_duplicates(actual, &path)?
                == parse_json_without_duplicates(&expected_bytes, &path)?
        } else {
            actual == &expected_bytes
        };
        if !matches {
            return Err(format!(
                "GuestControlSign retirement changed noncanonical content in {path}"
            ));
        }
    }
    Ok(())
}

fn expected_guest_signing_retirement_blob(path: &str, parent: &[u8]) -> Result<Vec<u8>, String> {
    match path {
        W5_PRIVILEGES_SCHEMA_PATH => transform_privileges_schema(parent),
        W5_WIRE_SCHEMA_PATH => transform_wire_schema(parent),
        W5_BROKER_WIRE_PATH => transform_broker_wire(parent),
        W5_PRIVILEGES_PATH => transform_privileges_source(parent),
        W5_PRIVILEGES_W3_PATH => transform_privileges_w3_source(parent),
        W5_PRIVILEGES_PARITY_PATH => transform_privileges_parity(parent),
        W5_BROKER_DISPOSITIONS_DOC_PATH => transform_broker_dispositions_doc(parent),
        W5_PRIVILEGES_DOC_PATH => transform_privileges_doc(parent),
        other => Err(format!(
            "no canonical GuestControlSign retirement transform for {other}"
        )),
    }
}

fn utf8_retirement_blob<'a>(path: &str, bytes: &'a [u8]) -> Result<&'a str, String> {
    std::str::from_utf8(bytes)
        .map_err(|_| format!("GuestControlSign retirement source is not UTF-8: {path}"))
}

fn remove_exact_once(source: &mut String, needle: &str, label: &str) -> Result<(), String> {
    if source.match_indices(needle).count() != 1 {
        return Err(format!(
            "GuestControlSign parent contract has unexpected {label} shape"
        ));
    }
    *source = source.replacen(needle, "", 1);
    Ok(())
}

fn replace_exact_once(
    source: &mut String,
    needle: &str,
    replacement: &str,
    label: &str,
) -> Result<(), String> {
    if source.match_indices(needle).count() != 1 {
        return Err(format!(
            "GuestControlSign parent contract has unexpected {label} shape"
        ));
    }
    *source = source.replacen(needle, replacement, 1);
    Ok(())
}

fn remove_rust_item(source: &mut String, marker: &str, label: &str) -> Result<(), String> {
    if source.match_indices(marker).count() != 1 {
        return Err(format!(
            "GuestControlSign parent Rust contract has unexpected {label} item"
        ));
    }
    let marker_offset = source.find(marker).expect("counted marker");
    let mut start = source[..marker_offset]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    loop {
        if start == 0 {
            break;
        }
        let previous_end = start - 1;
        let previous_start = source[..previous_end]
            .rfind('\n')
            .map_or(0, |offset| offset + 1);
        if source[previous_start..previous_end]
            .trim_start()
            .starts_with("#[")
        {
            start = previous_start;
        } else {
            break;
        }
    }

    let declaration_end = source[marker_offset..]
        .find('\n')
        .map_or(source.len(), |offset| marker_offset + offset);
    let declaration = &source[marker_offset..declaration_end];
    let mut end = if declaration.contains(';') && !declaration.contains('{') {
        declaration_end
    } else {
        let open = source[marker_offset..]
            .find('{')
            .map(|offset| marker_offset + offset)
            .ok_or_else(|| format!("GuestControlSign {label} item has no body"))?;
        let mut depth = 0usize;
        let mut close = None;
        for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth = depth
                        .checked_sub(1)
                        .ok_or_else(|| format!("GuestControlSign {label} item is malformed"))?;
                    if depth == 0 {
                        close = Some(open + offset + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        close.ok_or_else(|| format!("GuestControlSign {label} item is unterminated"))?
    };
    if source.as_bytes().get(end) == Some(&b'\n') {
        end += 1;
    }
    source.replace_range(start..end, "");
    Ok(())
}

fn transform_privileges_source(parent: &[u8]) -> Result<Vec<u8>, String> {
    let mut source = utf8_retirement_blob(W5_PRIVILEGES_PATH, parent)?.to_owned();
    remove_exact_once(
        &mut source,
        "    row(\n        \"GuestControlSign\",\n        \"guest-control token\",\n        \"per-VM\",\n        &[\"d2bd\"],\n        false,\n        SecretAccess::RedactedOnly,\n        BrokerRequirement::Yes,\n        AuditMode::Yes,\n    ),\n",
        "privilege row",
    )?;
    Ok(source.into_bytes())
}

fn transform_privileges_w3_source(parent: &[u8]) -> Result<Vec<u8>, String> {
    let mut source = utf8_retirement_blob(W5_PRIVILEGES_W3_PATH, parent)?.to_owned();
    for (needle, label) in [
        ("    GuestControlSign,\n", "W3 enum variant"),
        (
            "            Self::GuestControlSign => \"GuestControlSign\",\n",
            "W3 wire-tag arm",
        ),
        (
            "            Self::GuestControlSign,\n",
            "W3 operation inventory entry",
        ),
        (
            "            Self::GuestControlSign => W3OperationFlags {\n                audit: true,\n                destructive: false,\n                secret_access: true,\n            },\n",
            "W3 operation flags arm",
        ),
    ] {
        remove_exact_once(&mut source, needle, label)?;
    }
    remove_rust_item(
        &mut source,
        "fn only_guest_control_sign_grants_secret_access()",
        "W3 secret-access test",
    )?;
    normalize_retired_rust_spacing(&mut source);
    Ok(source.into_bytes())
}

fn transform_broker_wire(parent: &[u8]) -> Result<Vec<u8>, String> {
    let mut source = utf8_retirement_blob(W5_BROKER_WIRE_PATH, parent)?.to_owned();
    for (needle, label) in [
        (
            "use crate::guest_auth::AUTH_NONCE_LEN;\n",
            "guest-auth nonce import",
        ),
        (
            "    GuestControlSign(GuestControlSignRequest),\n",
            "broker request variant",
        ),
        (
            "            Self::GuestControlSign(_) => \"GuestControlSign\",\n",
            "broker operation-name arm",
        ),
        (
            "            Self::GuestControlSign(_) => \"guest-control-auth\",\n",
            "broker target arm",
        ),
        (
            "    GuestControlSign(GuestControlSignResponse),\n",
            "broker response variant",
        ),
    ] {
        remove_exact_once(&mut source, needle, label)?;
    }
    for (marker, label) in [
        ("pub enum GuestControlProofRole", "guest-control proof-role"),
        ("pub enum GuestControlDirection", "guest-control direction"),
        ("pub enum GuestControlAuthPurpose", "guest-control purpose"),
        ("pub struct GuestBootIdWire", "guest boot identifier"),
        ("impl GuestBootIdWire {", "guest boot identifier methods"),
        (
            "impl JsonSchema for GuestBootIdWire",
            "guest boot identifier schema",
        ),
        (
            "pub struct GuestControlSignRequest",
            "guest-control signing request",
        ),
        (
            "impl GuestControlSignRequest",
            "guest-control signing validation",
        ),
        (
            "pub struct GuestControlSignResponse",
            "guest-control signing response",
        ),
    ] {
        remove_rust_item(&mut source, marker, label)?;
    }
    normalize_retired_rust_spacing(&mut source);
    Ok(source.into_bytes())
}

fn normalize_retired_rust_spacing(source: &mut String) {
    while source.contains("\n\n\n") {
        *source = source.replace("\n\n\n", "\n\n");
    }
}

fn transform_privileges_parity(parent: &[u8]) -> Result<Vec<u8>, String> {
    let mut source = utf8_retirement_blob(W5_PRIVILEGES_PARITY_PATH, parent)?.to_owned();
    replace_exact_once(
        &mut source,
        "    let rendered = load_privileges_fixture_from_env();\n    let rust = PrivilegesJson::w1(rendered.schema_version.clone());\n",
        "    let mut rendered = load_privileges_fixture_from_env();\n    let retired = rendered\n        .broker_operations\n        .iter()\n        .position(|operation| operation.operation == \"GuestControlSign\")\n        .expect(\"Nix privilege emitter must retain GuestControlSign until declarative retirement\");\n    rendered.broker_operations.remove(retired);\n    let rust = PrivilegesJson::w1(rendered.schema_version.clone());\n",
        "privileges parity selector",
    )?;
    Ok(source.into_bytes())
}

fn transform_broker_dispositions_doc(parent: &[u8]) -> Result<Vec<u8>, String> {
    let mut source = utf8_retirement_blob(W5_BROKER_DISPOSITIONS_DOC_PATH, parent)?.to_owned();
    remove_markdown_row(
        &mut source,
        "| GuestControlSign |",
        "broker disposition row",
    )?;
    Ok(source.into_bytes())
}

fn transform_privileges_doc(parent: &[u8]) -> Result<Vec<u8>, String> {
    let mut source = utf8_retirement_blob(W5_PRIVILEGES_DOC_PATH, parent)?.to_owned();
    replace_exact_once(
        &mut source,
        "- **secret** — `yes` for operations whose implementation reads secret\n  material or whose audit record may reference secret-material\n  identifiers. `redacted-only` rows carry only derived/redacted metadata:\n  for example `GuestControlSign` records token-transcript metadata\n  (`transcript_len`, `peer_cid_present`, `capabilities_hash_present`),\n  and `UsbipBind` records normalized device identity plus serial HMAC\n  correlations, never the per-VM token, signature bytes, raw serial, raw\n  sysfs path, or device path.\n",
        "- **secret** — `yes` for operations whose implementation reads secret\n  material or whose audit record may reference secret-material\n  identifiers. `redacted-only` rows carry only derived/redacted metadata:\n  `UsbipBind` records normalized device identity plus serial HMAC\n  correlations, never the raw serial, raw sysfs path, or device path.\n",
        "privilege secret-metadata paragraph",
    )?;
    remove_markdown_row(
        &mut source,
        "| `GuestControlSign` |",
        "privilege matrix row",
    )?;
    Ok(source.into_bytes())
}

fn remove_markdown_row(source: &mut String, row_prefix: &str, label: &str) -> Result<(), String> {
    let matches = source
        .match_indices(row_prefix)
        .filter_map(|(offset, _)| {
            (offset == 0 || source.as_bytes().get(offset - 1) == Some(&b'\n')).then_some(offset)
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "GuestControlSign parent documentation has unexpected {label} shape"
        ));
    }
    let start = matches[0];
    let end = source[start..]
        .find('\n')
        .map_or(source.len(), |offset| start + offset + 1);
    source.replace_range(start..end, "");
    Ok(())
}

fn transform_privileges_schema(parent: &[u8]) -> Result<Vec<u8>, String> {
    let mut schema = parse_json_without_duplicates(parent, W5_PRIVILEGES_SCHEMA_PATH)?;
    let values = schema
        .pointer_mut("/definitions/OperationAuthz/properties/operation/enum")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "parent privileges schema has no operation enum".to_owned())?;
    let matches = values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (value.as_str() == Some("GuestControlSign")).then_some(index))
        .collect::<Vec<_>>();
    let [index] = matches.as_slice() else {
        return Err(
            "parent privileges schema has noncanonical GuestControlSign entries".to_owned(),
        );
    };
    values.remove(*index);
    canonical_json_bytes(&schema)
}

fn transform_wire_schema(parent: &[u8]) -> Result<Vec<u8>, String> {
    let mut schema = parse_json_without_duplicates(parent, W5_WIRE_SCHEMA_PATH)?;
    remove_schema_variant(
        &mut schema,
        "BrokerRequest",
        "#/definitions/GuestControlSignRequest",
    )?;
    remove_schema_variant(
        &mut schema,
        "BrokerResponse",
        "#/definitions/GuestControlSignResponse",
    )?;
    let definitions = schema
        .get_mut("definitions")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "parent wire schema has no definitions".to_owned())?;
    for name in [
        "GuestBootIdWire",
        "GuestControlAuthPurpose",
        "GuestControlDirection",
        "GuestControlProofRole",
        "GuestControlSignRequest",
        "GuestControlSignResponse",
    ] {
        if definitions.remove(name).is_none() {
            return Err(format!(
                "parent wire schema is missing GuestControlSign definition {name}"
            ));
        }
    }
    canonical_json_bytes(&schema)
}

fn remove_schema_variant(
    schema: &mut Value,
    definition: &str,
    expected_payload: &str,
) -> Result<(), String> {
    let variants = schema
        .pointer_mut(&format!("/definitions/{definition}/oneOf"))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("parent wire schema has no {definition} variants"))?;
    let matches = variants
        .iter()
        .enumerate()
        .filter_map(|(index, variant)| {
            let names = variant
                .pointer("/properties/kind/enum")
                .and_then(Value::as_array)?;
            (names.len() == 1 && names[0].as_str() == Some("GuestControlSign")).then_some(index)
        })
        .collect::<Vec<_>>();
    let [index] = matches.as_slice() else {
        return Err(format!(
            "parent wire schema has noncanonical GuestControlSign {definition} variants"
        ));
    };
    if variants[*index]
        .pointer("/properties/payload/$ref")
        .and_then(Value::as_str)
        != Some(expected_payload)
    {
        return Err(format!(
            "parent wire schema has unexpected GuestControlSign {definition} payload"
        ));
    }
    variants.remove(*index);
    Ok(())
}

fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("cannot canonicalize retirement schema: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

struct DuplicateFreeJson(Value);

impl<'de> Deserialize<'de> for DuplicateFreeJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(DuplicateFreeJsonVisitor)
    }
}

struct DuplicateFreeJsonVisitor;

impl<'de> Visitor<'de> for DuplicateFreeJsonVisitor {
    type Value = DuplicateFreeJson;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(DuplicateFreeJson(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(DuplicateFreeJson(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(DuplicateFreeJson(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .map(DuplicateFreeJson)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(DuplicateFreeJson(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(DuplicateFreeJson(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(DuplicateFreeJson(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(DuplicateFreeJson(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        DuplicateFreeJson::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<DuplicateFreeJson>()? {
            values.push(value.0);
        }
        Ok(DuplicateFreeJson(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(<A::Error as serde::de::Error>::custom(
                    "duplicate JSON object key",
                ));
            }
            let value = map.next_value::<DuplicateFreeJson>()?;
            values.insert(key, value.0);
        }
        Ok(DuplicateFreeJson(Value::Object(values)))
    }
}

fn parse_json_without_duplicates(bytes: &[u8], path: &str) -> Result<Value, String> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = DuplicateFreeJson::deserialize(&mut deserializer)
        .map_err(|error| format!("invalid retirement JSON in {path}: {error}"))?;
    deserializer
        .end()
        .map_err(|error| format!("trailing retirement JSON in {path}: {error}"))?;
    Ok(value.0)
}

fn expected_daemon_api_retirement(
    parent: &[u8],
    expected_broker_wire: &[u8],
) -> Result<Vec<u8>, String> {
    let mut doc = utf8_retirement_blob(W5_DAEMON_API_PATH, parent)?.to_owned();
    for (needle, label) in [
        (
            "`GuestControlSign` — (GuestControlSignRequest); ",
            "daemon request variant",
        ),
        (
            "`GuestControlSign` — (GuestControlSignResponse); ",
            "daemon response variant",
        ),
    ] {
        remove_exact_once(&mut doc, needle, label)?;
    }
    for (row_prefix, label) in [
        (
            "| `GuestControlSignRequest` |",
            "daemon signing request row",
        ),
        (
            "| `GuestControlSignResponse` |",
            "daemon signing response row",
        ),
        ("| `GuestControlProofRole` |", "daemon proof-role row"),
        ("| `GuestControlDirection` |", "daemon direction row"),
        ("| `GuestControlAuthPurpose` |", "daemon purpose row"),
    ] {
        remove_markdown_row(&mut doc, row_prefix, label)?;
    }
    let broker_wire = utf8_retirement_blob(W5_BROKER_WIRE_PATH, expected_broker_wire)?;
    Ok(rebind_broker_wire_links(&doc, broker_wire)?.into_bytes())
}

fn rebind_broker_wire_links(doc: &str, broker_wire: &str) -> Result<String, String> {
    const PREFIX: &str = "../../packages/d2b-contracts/src/broker_wire.rs#L";
    let mut output = String::with_capacity(doc.len());
    let mut cursor = 0usize;
    while let Some(relative) = doc[cursor..].find(PREFIX) {
        let offset = cursor + relative;
        output.push_str(&doc[cursor..offset]);
        output.push_str(PREFIX);
        let label_start = doc[..offset]
            .rfind("[`")
            .ok_or_else(|| "daemon API broker-wire link has no item label".to_owned())?;
        let label_end = doc[label_start + 2..offset]
            .find("`](")
            .map(|end| label_start + 2 + end)
            .ok_or_else(|| "daemon API broker-wire link has malformed item label".to_owned())?;
        if label_end + 3 != offset {
            return Err("daemon API broker-wire link is not canonical".to_owned());
        }
        let label = &doc[label_start + 2..label_end];
        let line = rust_item_line(broker_wire, label)?;
        output.push_str(&line.to_string());
        let digits_start = offset + PREFIX.len();
        let digits = doc[digits_start..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if digits == 0 {
            return Err("daemon API broker-wire link has no line number".to_owned());
        }
        cursor = digits_start + digits;
    }
    output.push_str(&doc[cursor..]);
    Ok(output)
}

fn rust_item_line(source: &str, name: &str) -> Result<usize, String> {
    let struct_prefix = format!("pub struct {name}");
    let enum_prefix = format!("pub enum {name}");
    let matches = source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim_start();
            (rust_declaration_matches(line, &struct_prefix)
                || rust_declaration_matches(line, &enum_prefix))
            .then_some(index + 1)
        })
        .collect::<Vec<_>>();
    let [line] = matches.as_slice() else {
        return Err(format!(
            "transformed broker wire has no unique daemon API item {name}"
        ));
    };
    Ok(*line)
}

fn rust_declaration_matches(line: &str, prefix: &str) -> bool {
    line.strip_prefix(prefix).is_some_and(|suffix| {
        suffix.is_empty()
            || suffix.starts_with(char::is_whitespace)
            || suffix.starts_with('{')
            || suffix.starts_with('(')
            || suffix.starts_with('<')
    })
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

fn validate_sorted_relative_paths_allow_empty(paths: &[String], label: &str) -> Result<(), String> {
    validate_sorted_strings_allow_empty(paths, label)?;
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
        blobs: BTreeMap<(String, String), Vec<u8>>,
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
                blobs: BTreeMap::new(),
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
            if root == Path::new(CANDIDATE_ROOT)
                && let Some(bytes) = self.blobs.get(&(commit.to_owned(), path.to_owned()))
            {
                return Ok(bytes.clone());
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

    fn retirement_parent_blobs(retirement: &W5ContractRetirement) -> BTreeMap<String, Vec<u8>> {
        let privileges = concat!(
            "const ROWS: &[()] = &[\n",
            "    row(\n",
            "        \"GuestControlSign\",\n",
            "        \"guest-control token\",\n",
            "        \"per-VM\",\n",
            "        &[\"d2bd\"],\n",
            "        false,\n",
            "        SecretAccess::RedactedOnly,\n",
            "        BrokerRequirement::Yes,\n",
            "        AuditMode::Yes,\n",
            "    ),\n",
            "    row(\"InjectSecretById\"),\n",
            "];\n",
        );
        let privileges_w3 = concat!(
            "pub enum W3BrokerOperation {\n",
            "    GuestControlSign,\n",
            "    ModprobeIfAllowed,\n",
            "}\n\n",
            "impl W3BrokerOperation {\n",
            "    fn wire_tag(self) -> &'static str {\n",
            "        match self {\n",
            "            Self::GuestControlSign => \"GuestControlSign\",\n",
            "            Self::ModprobeIfAllowed => \"ModprobeIfAllowed\",\n",
            "        }\n",
            "    }\n\n",
            "    fn all() -> &'static [Self] {\n",
            "        &[\n",
            "            Self::GuestControlSign,\n",
            "            Self::ModprobeIfAllowed,\n",
            "        ]\n",
            "    }\n\n",
            "    fn flags(self) -> W3OperationFlags {\n",
            "        match self {\n",
            "            Self::GuestControlSign => W3OperationFlags {\n",
            "                audit: true,\n",
            "                destructive: false,\n",
            "                secret_access: true,\n",
            "            },\n",
            "            Self::ModprobeIfAllowed => W3OperationFlags {\n",
            "                audit: true,\n",
            "                destructive: true,\n",
            "                secret_access: false,\n",
            "            },\n",
            "        }\n",
            "    }\n",
            "}\n\n",
            "#[cfg(test)]\n",
            "mod tests {\n",
            "    #[test]\n",
            "    fn only_guest_control_sign_grants_secret_access() {\n",
            "        assert!(true);\n",
            "    }\n",
            "}\n",
        );
        let privileges_parity = concat!(
            "fn rendered_privileges_matches_rust_matrix() {\n",
            "    let rendered = load_privileges_fixture_from_env();\n",
            "    let rust = PrivilegesJson::w1(rendered.schema_version.clone());\n",
            "}\n",
        );
        let broker_wire = concat!(
            "use crate::guest_auth::AUTH_NONCE_LEN;\n\n",
            "pub enum BrokerRequest {\n",
            "    GuestControlSign(GuestControlSignRequest),\n",
            "    Other,\n",
            "}\n\n",
            "impl BrokerRequest {\n",
            "    fn op_name(&self) -> &'static str {\n",
            "        match self {\n",
            "            Self::GuestControlSign(_) => \"GuestControlSign\",\n",
            "            Self::Other => \"Other\",\n",
            "        }\n",
            "    }\n\n",
            "    fn opaque_target_id(&self) -> &'static str {\n",
            "        match self {\n",
            "            Self::GuestControlSign(_) => \"guest-control-auth\",\n",
            "            Self::Other => \"operation\",\n",
            "        }\n",
            "    }\n",
            "}\n\n",
            "pub enum BrokerResponse {\n",
            "    GuestControlSign(GuestControlSignResponse),\n",
            "    Ack,\n",
            "}\n\n",
            "#[derive(Debug)]\n",
            "pub enum GuestControlProofRole {\n",
            "    HostProof,\n",
            "}\n\n",
            "#[derive(Debug)]\n",
            "pub enum GuestControlDirection {\n",
            "    HostToGuest,\n",
            "}\n\n",
            "#[derive(Debug)]\n",
            "pub enum GuestControlAuthPurpose {\n",
            "    GuestControlAuthV1,\n",
            "}\n\n",
            "#[derive(Debug)]\n",
            "pub struct GuestBootIdWire(pub String);\n\n",
            "impl GuestBootIdWire {\n",
            "    fn as_str(&self) -> &str { &self.0 }\n",
            "}\n\n",
            "impl JsonSchema for GuestBootIdWire {\n",
            "    fn marker() {}\n",
            "}\n\n",
            "#[derive(Debug)]\n",
            "pub struct GuestControlSignRequest {\n",
            "    nonce: [u8; AUTH_NONCE_LEN],\n",
            "}\n\n",
            "impl GuestControlSignRequest {\n",
            "    fn validate_shape(&self) -> bool { self.nonce.len() == AUTH_NONCE_LEN }\n",
            "}\n\n",
            "#[derive(Debug)]\n",
            "pub struct GuestControlSignResponse {\n",
            "    tag: [u8; 32],\n",
            "}\n",
        );
        let privileges_doc = concat!(
            "# Privileges\n\n",
            "- **secret** — `yes` for operations whose implementation reads secret\n",
            "  material or whose audit record may reference secret-material\n",
            "  identifiers. `redacted-only` rows carry only derived/redacted metadata:\n",
            "  for example `GuestControlSign` records token-transcript metadata\n",
            "  (`transcript_len`, `peer_cid_present`, `capabilities_hash_present`),\n",
            "  and `UsbipBind` records normalized device identity plus serial HMAC\n",
            "  correlations, never the per-VM token, signature bytes, raw serial, raw\n",
            "  sysfs path, or device path.\n\n",
            "Unknown variants and unknown fields are denied.\n\n",
            "| Operation | Subject |\n",
            "| --- | --- |\n",
            "| `GuestControlSign` | guest-control token |\n",
            "| `InjectSecretById` | secret |\n",
        );
        let daemon_api = concat!(
            "# Daemon API\n\n",
            "| Type | Kind | Source | Shape |\n",
            "| --- | --- | --- | --- |\n",
            "| `BrokerRequest` | enum | [`BrokerRequest`](../../packages/d2b-contracts/src/broker_wire.rs#L3) | `GuestControlSign` — (GuestControlSignRequest); `Other` |\n",
            "| `GuestControlSignRequest` | struct | source | request |\n",
            "| `GuestControlProofRole` | enum | source | role |\n",
            "| `GuestControlDirection` | enum | source | direction |\n",
            "| `GuestControlAuthPurpose` | enum | source | purpose |\n",
            "| `BrokerResponse` | enum | [`BrokerResponse`](../../packages/d2b-contracts/src/broker_wire.rs#L24) | `GuestControlSign` — (GuestControlSignResponse); `Ack` |\n",
            "| `GuestControlSignResponse` | struct | source | response |\n",
        );
        let privileges_schema = serde_json::json!({
            "definitions": {
                "OperationAuthz": {
                    "properties": {
                        "operation": {
                            "enum": ["InjectSecretById", "GuestControlSign"]
                        }
                    }
                }
            }
        });
        let signing_variant = |payload: &str| {
            serde_json::json!({
                "properties": {
                    "kind": {"enum": ["GuestControlSign"]},
                    "payload": {"$ref": payload}
                }
            })
        };
        let wire_schema = serde_json::json!({
            "definitions": {
                "BrokerRequest": {
                    "oneOf": [
                        {"properties": {"kind": {"enum": ["Other"]}}},
                        signing_variant("#/definitions/GuestControlSignRequest")
                    ]
                },
                "BrokerResponse": {
                    "oneOf": [
                        {"properties": {"kind": {"enum": ["Ack"]}}},
                        signing_variant("#/definitions/GuestControlSignResponse")
                    ]
                },
                "GuestBootIdWire": {},
                "GuestControlAuthPurpose": {},
                "GuestControlDirection": {},
                "GuestControlProofRole": {},
                "GuestControlSignRequest": {},
                "GuestControlSignResponse": {},
                "Other": {}
            }
        });
        let fixtures = BTreeMap::from([
            (
                W5_PRIVILEGES_PARITY_PATH.to_owned(),
                privileges_parity.as_bytes().to_vec(),
            ),
            (W5_BROKER_WIRE_PATH.to_owned(), broker_wire.as_bytes().to_vec()),
            (W5_PRIVILEGES_PATH.to_owned(), privileges.as_bytes().to_vec()),
            (
                W5_PRIVILEGES_W3_PATH.to_owned(),
                privileges_w3.as_bytes().to_vec(),
            ),
            (
                W5_BROKER_DISPOSITIONS_DOC_PATH.to_owned(),
                b"| Variant | Disposition |\n| --- | --- |\n| GuestControlSign | callable-read-only |\n| Other | deny |\n"
                    .to_vec(),
            ),
            (
                W5_DAEMON_API_PATH.to_owned(),
                daemon_api.as_bytes().to_vec(),
            ),
            (
                W5_PRIVILEGES_DOC_PATH.to_owned(),
                privileges_doc.as_bytes().to_vec(),
            ),
            (
                W5_PRIVILEGES_SCHEMA_PATH.to_owned(),
                canonical_json_bytes(&privileges_schema).unwrap(),
            ),
            (
                W5_WIRE_SCHEMA_PATH.to_owned(),
                canonical_json_bytes(&wire_schema).unwrap(),
            ),
        ]);
        let authorized = retirement_paths(retirement)
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            fixtures.keys().cloned().collect::<BTreeSet<_>>(),
            authorized,
            "synthetic retirement fixtures must cover every authorized path"
        );
        fixtures
    }

    fn retirement_candidate_blobs(
        retirement: &W5ContractRetirement,
        parent: &BTreeMap<String, Vec<u8>>,
    ) -> BTreeMap<String, Vec<u8>> {
        let mut candidate = BTreeMap::new();
        for (path, bytes) in parent {
            if path != W5_DAEMON_API_PATH {
                candidate.insert(
                    path.clone(),
                    expected_guest_signing_retirement_blob(path, bytes)
                        .unwrap_or_else(|error| panic!("transform {path}: {error}")),
                );
            }
        }
        candidate.insert(
            W5_DAEMON_API_PATH.to_owned(),
            expected_daemon_api_retirement(
                parent
                    .get(W5_DAEMON_API_PATH)
                    .expect("parent daemon API fixture"),
                {
                    let broker_wire = candidate
                        .get(W5_BROKER_WIRE_PATH)
                        .expect("candidate broker wire fixture");
                    assert!(
                        String::from_utf8_lossy(broker_wire).contains("pub enum BrokerRequest"),
                        "canonical broker transform removed BrokerRequest"
                    );
                    broker_wire
                },
            )
            .expect("transform daemon API"),
        );
        assert_eq!(candidate.len(), parent.len());
        assert_eq!(
            candidate.keys().collect::<BTreeSet<_>>(),
            parent.keys().collect::<BTreeSet<_>>()
        );
        assert_eq!(retirement.operation, "GuestControlSign");
        candidate
    }

    fn retirement_paths(retirement: &W5ContractRetirement) -> Vec<String> {
        retirement
            .source_paths
            .iter()
            .chain(
                retirement
                    .test_selectors
                    .iter()
                    .map(|selector| &selector.test_file),
            )
            .chain(retirement.companion_paths.iter())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn probe_with_retirement_fixtures() -> FakeProbe {
        let policy = policy();
        let retirement = &policy.w5_contract_retirements[0];
        let parent = retirement_parent_blobs(retirement);
        let candidate = retirement_candidate_blobs(retirement, &parent);
        let mut probe = FakeProbe::valid();
        probe.changed_paths = retirement_paths(retirement);
        for (path, bytes) in parent {
            probe.blobs.insert((BASE_OID.to_owned(), path), bytes);
        }
        for (path, bytes) in candidate {
            probe.blobs.insert((HEAD_OID.to_owned(), path), bytes);
        }
        probe
    }

    #[test]
    fn w5_successor_pin_exceptions_are_exact_and_separate_from_contract_retirement() {
        let policy = policy();
        assert_eq!(policy.w5_successor_pin_paths.len(), 13);
        assert!(
            policy
                .w5_successor_pin_paths
                .iter()
                .all(|path| { path.starts_with("tests/golden/pinned/") && path.ends_with(".txt") })
        );

        let mut incomplete = policy.clone();
        incomplete.w5_successor_pin_paths.pop();
        assert!(
            incomplete
                .validate()
                .expect_err("incomplete W5 pin inventory must fail")
                .contains("W5 protected exceptions must exactly match")
        );
    }

    #[test]
    fn canonical_guest_signing_retirement_fixtures_are_accepted() {
        let policy = policy();
        let retirement = &policy.w5_contract_retirements[0];
        let parent = retirement_parent_blobs(retirement);
        let candidate = retirement_candidate_blobs(retirement, &parent);
        verify_w5_contract_retirement_contents(retirement, &parent, &candidate)
            .expect("canonical GuestControlSign retirement");

        let probe = probe_with_retirement_fixtures();
        let verified =
            verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
                .expect("parent-authoritative canonical retirement");
        assert_eq!(verified.wave, "w5");
        assert!(
            probe
                .blob_reads
                .borrow()
                .iter()
                .any(|(root, commit, path)| {
                    root == Path::new(CANDIDATE_ROOT)
                        && commit == BASE_OID
                        && path == W5_BROKER_WIRE_PATH
                })
        );
    }

    #[test]
    fn canonical_retirement_fixtures_ignore_current_tree_operation_state() {
        let policy = policy();
        let retirement = &policy.w5_contract_retirements[0];
        let synthetic_parent = retirement_parent_blobs(retirement);
        let synthetic_candidate = retirement_candidate_blobs(retirement, &synthetic_parent);
        let simulated_current_trees = [&synthetic_parent, &synthetic_candidate];
        let states = simulated_current_trees.map(|current| {
            String::from_utf8_lossy(current.get(W5_BROKER_WIRE_PATH).unwrap())
                .contains("GuestControlSign")
        });
        assert_eq!(states, [true, false]);

        for _current_tree in simulated_current_trees {
            let parent = retirement_parent_blobs(retirement);
            let candidate = retirement_candidate_blobs(retirement, &parent);
            verify_w5_contract_retirement_contents(retirement, &parent, &candidate)
                .expect("tree-independent canonical retirement fixtures");
        }
    }

    fn assert_retirement_mutation_rejected(
        label: &str,
        mutate: impl FnOnce(&mut BTreeMap<String, Vec<u8>>),
    ) {
        let policy = policy();
        let retirement = &policy.w5_contract_retirements[0];
        let parent = retirement_parent_blobs(retirement);
        let mut candidate = retirement_candidate_blobs(retirement, &parent);
        mutate(&mut candidate);
        let error = verify_w5_contract_retirement_contents(retirement, &parent, &candidate)
            .expect_err(label);
        assert!(
            error.contains("noncanonical") || error.contains("missing"),
            "{error}"
        );
    }

    #[test]
    fn signing_retirement_rejects_unrelated_rust_rows_and_enums() {
        assert_retirement_mutation_rejected("unrelated privilege row", |candidate| {
            let source = candidate.get_mut(W5_PRIVILEGES_PATH).unwrap();
            let source = String::from_utf8(source.clone()).unwrap().replacen(
                "\"InjectSecretById\"",
                "\"InjectSecretByIdChanged\"",
                1,
            );
            *candidate.get_mut(W5_PRIVILEGES_PATH).unwrap() = source.into_bytes();
        });
        assert_retirement_mutation_rejected("unrelated W3 enum", |candidate| {
            let source = candidate.get_mut(W5_PRIVILEGES_W3_PATH).unwrap();
            let source = String::from_utf8(source.clone()).unwrap().replacen(
                "    ModprobeIfAllowed,\n",
                "    UnrelatedOperation,\n    ModprobeIfAllowed,\n",
                1,
            );
            *candidate.get_mut(W5_PRIVILEGES_W3_PATH).unwrap() = source.into_bytes();
        });
    }

    #[test]
    fn signing_retirement_rejects_unrelated_docs_and_mixed_hunks() {
        assert_retirement_mutation_rejected("unrelated documentation", |candidate| {
            candidate
                .get_mut(W5_PRIVILEGES_DOC_PATH)
                .unwrap()
                .extend_from_slice(b"\nunrelated prose\n");
        });
        assert_retirement_mutation_rejected("unrelated documentation deletion", |candidate| {
            let doc = candidate.get_mut(W5_PRIVILEGES_DOC_PATH).unwrap();
            let changed = String::from_utf8(doc.clone()).unwrap().replacen(
                "Unknown variants and unknown fields",
                "Unknown fields",
                1,
            );
            *doc = changed.into_bytes();
        });
        assert_retirement_mutation_rejected("mixed authorized and unrelated hunks", |candidate| {
            candidate
                .get_mut(W5_BROKER_WIRE_PATH)
                .unwrap()
                .extend_from_slice(b"\nconst UNRELATED: bool = true;\n");
            candidate
                .get_mut(W5_BROKER_DISPOSITIONS_DOC_PATH)
                .unwrap()
                .extend_from_slice(b"\nunrelated disposition\n");
        });
    }

    #[test]
    fn signing_retirement_rejects_schema_and_generated_output_decoys() {
        assert_retirement_mutation_rejected("unrelated privilege schema enum", |candidate| {
            let schema = candidate.get_mut(W5_PRIVILEGES_SCHEMA_PATH).unwrap();
            let mut value: Value = serde_json::from_slice(schema).unwrap();
            value
                .pointer_mut("/definitions/OperationAuthz/properties/operation/enum")
                .and_then(Value::as_array_mut)
                .unwrap()
                .push(Value::String("UnrelatedOperation".to_owned()));
            *schema = canonical_json_bytes(&value).unwrap();
        });
        assert_retirement_mutation_rejected("wire schema definition decoy", |candidate| {
            let schema = candidate.get_mut(W5_WIRE_SCHEMA_PATH).unwrap();
            let mut value: Value = serde_json::from_slice(schema).unwrap();
            value
                .get_mut("definitions")
                .and_then(Value::as_object_mut)
                .unwrap()
                .insert(
                    "GuestControlSignDecoy".to_owned(),
                    serde_json::json!({"type": "null"}),
                );
            *schema = canonical_json_bytes(&value).unwrap();
        });
        assert_retirement_mutation_rejected("generated daemon API decoy", |candidate| {
            candidate
                .get_mut(W5_DAEMON_API_PATH)
                .unwrap()
                .extend_from_slice(b"\n| `GuestControlSignDecoy` |\n");
        });
    }

    #[test]
    fn signing_retirement_rejects_renames_and_partial_inventory() {
        let policy = policy();
        let retirement = &policy.w5_contract_retirements[0];
        let mut probe = probe_with_retirement_fixtures();
        probe
            .blobs
            .remove(&(HEAD_OID.to_owned(), W5_BROKER_WIRE_PATH.to_owned()));
        let error = verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
            .expect_err("renamed retirement file");
        assert!(error.contains("candidate blob is missing"), "{error}");

        let mut probe = FakeProbe::valid();
        probe.changed_paths = retirement_paths(retirement);
        probe.changed_paths.pop();
        let error = verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
            .expect_err("partial retirement inventory");
        assert!(error.contains("exact complete path inventory"), "{error}");
    }

    #[test]
    fn signing_retirement_cannot_replace_the_parent_checker() {
        let mut probe = probe_with_retirement_fixtures();
        probe
            .changed_paths
            .push("packages/xtask/src/wave_policy.rs".to_owned());
        probe.changed_paths.sort();
        let error = verify_ownership(&probe, Path::new(AUTHORITY_ROOT), Path::new(CANDIDATE_ROOT))
            .expect_err("candidate checker replacement");
        assert!(
            error.contains("packages/xtask/src/wave_policy.rs"),
            "{error}"
        );
        assert!(
            probe
                .blob_reads
                .borrow()
                .iter()
                .any(|(root, commit, path)| {
                    root == Path::new(AUTHORITY_ROOT) && commit == BASE_OID && path == POLICY_PATH
                })
        );
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
    fn w8_wave_is_governed_with_canonical_branch_and_successor_grants() {
        let policy = policy();
        let ownership = policy.wave("w8").expect("w8 is governed");
        assert_eq!(ownership.branch_stem, "adr0045-w8-integration");
        assert_eq!(ownership.manifest_path, "delivery/manifests/w8.json");
        assert_eq!(ownership.allowed_parent_waves, vec!["w7".to_owned()]);
        assert!(ownership.allowed_prefixes.is_empty());
        assert_eq!(
            ownership.inherits_prefixes_from,
            vec!["w5".to_owned(), "w6".to_owned(), "w7".to_owned()]
        );
        assert_eq!(
            ownership.integration_only_protected_paths,
            vec![
                "delivery/README.md".to_owned(),
                "packages/Cargo.lock".to_owned(),
                "packages/Cargo.toml".to_owned(),
                "packages/xtask/tests/delivery_cli.rs".to_owned(),
                "packages/xtask/tests/delivery_w8.rs".to_owned(),
                "packages/xtask/tests/policy_workspace.rs".to_owned(),
            ]
        );
        assert!(ownership.allowed_protected_paths.is_empty());
        assert_eq!(ownership.landed_predecessor_ref.as_deref(), Some("main"));
    }

    #[test]
    fn w8_direct_to_main_parent_graph_terminates_without_resurrecting_branches() {
        let policy = policy();
        let ownership = policy.wave("w8").expect("w8 is governed");
        let probe = FakeProbe::valid();
        verify_parent_graph(
            &probe,
            Path::new(CANDIDATE_ROOT),
            REPOSITORY,
            &policy,
            ownership,
            "main",
            BASE_OID,
        )
        .expect("w8 may terminate its parent graph directly at the landed main trunk");
    }

    #[test]
    fn w8_rejects_a_forged_resurrected_w7_parent_chain() {
        let policy = policy();
        let ownership = policy.wave("w8").expect("w8 is governed");
        let mut probe = FakeProbe::valid();
        // A forged branch claiming the w7 stem, whose own Git Town parent is
        // forged straight to `main` -- skipping w6/w5 -- must not be accepted
        // as authority for w8, even though w8 itself may terminate at `main`.
        probe
            .ancestor_refs
            .insert("adr0045-w7-fake".to_owned(), OTHER_OID.to_owned());
        probe
            .ancestor_parents
            .insert("adr0045-w7-fake".to_owned(), "main".to_owned());
        let error = verify_parent_graph(
            &probe,
            Path::new(CANDIDATE_ROOT),
            REPOSITORY,
            &policy,
            ownership,
            "adr0045-w7-fake",
            OTHER_OID,
        )
        .expect_err("forged resurrected w7 parent chain");
        assert!(
            error.contains("w7 cannot use Git Town parent main as ownership authority"),
            "{error}"
        );
    }

    #[test]
    fn w8_rejects_wrong_parent_base_and_branch() {
        let policy = policy();
        let ownership = policy.wave("w8").expect("w8 is governed");

        // Wrong parent: w8 may only chain from w7, not from w6 directly.
        assert!(!policy.parent_is_allowed(ownership, "adr0045-w6-user-services"));
        // Wrong base: an unrelated, non-governed branch is not an authority root.
        assert!(!policy.parent_is_allowed(ownership, "some-unrelated-branch"));
        // W8 must start from the landed predecessor, not bypass W5-W7 by
        // returning to their historical shared root.
        assert!(!policy.parent_is_allowed(ownership, &policy.shared_root_branch));
        // Wrong branch: a near-miss branch name must not match the exact
        // canonical `adr0045-w8-integration` stem.
        assert!(policy.wave_for_branch("adr0045-w8-other").is_err());
        assert!(policy.wave_for_branch("adr0045-w8-integratio").is_err());
        assert!(policy.wave_for_branch("adr0045-w8-integration").is_ok());
        assert!(
            policy
                .wave_for_branch("adr0045-w8-integration-secrets-lifecycle")
                .is_ok()
        );
    }

    #[test]
    fn w8_rejects_unclassified_and_frozen_edits() {
        let policy = policy();
        let paths = [
            "packages/xtask/src/wave_policy.rs".to_owned(),
            "packages/d2b-core/src/privileges.rs".to_owned(),
            "scripts/wave-escape.sh".to_owned(),
        ];
        let error =
            check_changed_paths(&policy, "w8", &paths).expect_err("frozen and unowned paths");
        assert!(
            error.contains("packages/xtask/src/wave_policy.rs"),
            "{error}"
        );
        assert!(
            error.contains("packages/d2b-core/src/privileges.rs"),
            "{error}"
        );
        assert!(error.contains("scripts/wave-escape.sh"), "{error}");
    }

    #[test]
    fn w8_exact_integration_branch_owns_narrow_shared_seams() {
        let policy = policy();
        let paths = [
            "delivery/README.md".to_owned(),
            "packages/Cargo.lock".to_owned(),
            "packages/Cargo.toml".to_owned(),
            "packages/xtask/tests/delivery_cli.rs".to_owned(),
            "packages/xtask/tests/delivery_w8.rs".to_owned(),
            "packages/xtask/tests/policy_workspace.rs".to_owned(),
        ];
        check_changed_paths_for_branch(&policy, "w8", Some("adr0045-w8-integration"), &paths)
            .expect("W8 integration shared seams");
        for branch in [
            None,
            Some("adr0045-w8-integration-systemd-user-shell-routing"),
        ] {
            let error = check_changed_paths_for_branch(&policy, "w8", branch, &paths)
                .expect_err("W8 component workspace registration");
            for path in &paths {
                assert!(error.contains(path), "{error}");
            }
        }
        for wave in ["w5", "w6", "w7"] {
            let error = check_changed_paths(&policy, wave, &paths)
                .expect_err("predecessor wave workspace registration");
            for path in &paths {
                assert!(error.contains(path), "{error}");
            }
        }
    }

    #[test]
    fn w8_inherits_every_w5_w6_w7_implementation_prefix() {
        let policy = policy();
        for source_wave in ["w5", "w6", "w7"] {
            let ownership = policy.wave(source_wave).expect("governed source wave");
            for prefix in &ownership.allowed_prefixes {
                let path = format!("{prefix}w8-integration-probe.rs");
                check_changed_paths(&policy, "w8", std::slice::from_ref(&path)).unwrap_or_else(
                    |error| {
                        panic!(
                            "w8 must inherit {source_wave}-owned prefix {prefix}: {path}: {error}"
                        )
                    },
                );
            }
        }
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
