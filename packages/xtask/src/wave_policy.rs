use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, ExitCode},
};

use serde::Deserialize;

use crate::delivery::model::{
    DeliveryManifest, expected_wave_manifest_path, is_authoritative_manifest_path,
    validate_wave_identifier,
};

const POLICY_PATH: &str = "delivery/shared-contracts.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SharedContractPolicy {
    pub schema_version: u32,
    pub shared_root_branch: String,
    pub waves: Vec<WaveOwnership>,
    pub protected_paths: Vec<String>,
    pub protected_prefixes: Vec<String>,
    pub frozen_service_packages: Vec<String>,
    pub broker_typed_methods: Vec<TypedBrokerMethod>,
    pub workspace_dependencies: Vec<WorkspaceDependency>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WaveOwnership {
    pub wave: String,
    pub manifest_path: String,
    pub responsibility: String,
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

impl SharedContractPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported shared-contract policy schema".to_owned());
        }
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
        for wave in &self.waves {
            validate_wave_identifier(&wave.wave).map_err(|error| error.to_string())?;
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
            if !wave.additional_protected_paths.is_empty() {
                validate_sorted_paths(&wave.additional_protected_paths)?;
            }
            if !wave.allowed_protected_paths.is_empty() {
                validate_sorted_paths(&wave.allowed_protected_paths)?;
            }
        }
        validate_sorted_paths(&self.protected_paths)?;
        validate_sorted_prefixes(&self.protected_prefixes)?;
        validate_sorted_strings(&self.frozen_service_packages, "frozen service packages")?;
        validate_sorted_values(&self.broker_typed_methods, "typed broker methods")?;
        validate_sorted_values(&self.workspace_dependencies, "workspace dependencies")?;
        Ok(())
    }

    fn wave(&self, wave: &str) -> Result<&WaveOwnership, String> {
        self.waves
            .iter()
            .find(|entry| entry.wave == wave)
            .ok_or_else(|| format!("wave {wave} is not governed by the shared-contract policy"))
    }
}

pub fn run_cli(args: &[String]) -> ExitCode {
    match run(args) {
        Ok(()) => {
            println!("wave ownership policy: ok");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("wave ownership policy failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let [action, wave_flag, wave, base_flag, base] = args else {
        return Err(
            "usage: cargo xtask wave-policy check --wave <w5|w6|w7> --base <ref>".to_owned(),
        );
    };
    if action != "check" || wave_flag != "--wave" || base_flag != "--base" {
        return Err(
            "usage: cargo xtask wave-policy check --wave <w5|w6|w7> --base <ref>".to_owned(),
        );
    }
    let root = repository_root()?;
    let policy = read_policy(&root)?;
    let ownership = policy.wave(wave)?;
    verify_checked_in_manifest(&root, ownership)?;
    ensure_clean(&root)?;
    let base_oid = git_stdout(
        &root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{base}^{{commit}}"),
        ],
    )?;
    let ancestor = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["merge-base", "--is-ancestor", &base_oid, "HEAD"])
        .status()
        .map_err(|error| format!("cannot verify policy base: {error}"))?;
    if !ancestor.success() {
        return Err("policy base is not an ancestor of HEAD".to_owned());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "diff",
            "--no-renames",
            "--name-only",
            "-z",
            &format!("{base_oid}..HEAD"),
            "--",
        ])
        .output()
        .map_err(|error| format!("cannot inspect wave diff: {error}"))?;
    if !output.status.success() {
        return Err("git diff failed while checking wave ownership".to_owned());
    }
    let paths = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            std::str::from_utf8(entry)
                .map(str::to_owned)
                .map_err(|_| "wave diff path is not UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    check_changed_paths(&policy, wave, &paths)
}

pub fn read_policy(root: &Path) -> Result<SharedContractPolicy, String> {
    let bytes =
        fs::read(root.join(POLICY_PATH)).map_err(|error| format!("cannot read policy: {error}"))?;
    let policy: SharedContractPolicy =
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid policy JSON: {error}"))?;
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
        if is_authoritative_manifest_path(candidate)
            || policy.protected_paths.binary_search(path).is_ok()
            || policy
                .protected_prefixes
                .iter()
                .any(|prefix| path.starts_with(prefix))
            || ownership
                .additional_protected_paths
                .binary_search(path)
                .is_ok()
        {
            violations.push(path.clone());
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{wave} changed shared or foreign authority paths; return these changes to {}:\n{}",
            policy.shared_root_branch,
            violations.join("\n")
        ))
    }
}

fn verify_checked_in_manifest(root: &Path, ownership: &WaveOwnership) -> Result<(), String> {
    let bytes = fs::read(root.join(&ownership.manifest_path)).map_err(|error| {
        format!(
            "{} authority is not checked in at {}: {error}",
            ownership.wave, ownership.manifest_path
        )
    })?;
    let manifest: DeliveryManifest = serde_json::from_slice(&bytes)
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

fn ensure_clean(root: &Path) -> Result<(), String> {
    if git_stdout(
        root,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )?
    .is_empty()
    {
        Ok(())
    } else {
        Err("wave ownership checks require a clean worktree".to_owned())
    }
}

fn git_stdout(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if !output.status.success() {
        return Err(format!("git {} failed", args.join(" ")));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| "git output is not UTF-8".to_owned())
}

fn validate_sorted_paths(paths: &[String]) -> Result<(), String> {
    validate_sorted_strings(paths, "protected paths")?;
    for path in paths {
        validate_relative_path(Path::new(path))?;
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

fn validate_sorted_strings(values: &[String], label: &str) -> Result<(), String> {
    if values.is_empty() || values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(format!("{label} must be nonempty, sorted, and unique"));
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
    use super::*;

    fn policy() -> SharedContractPolicy {
        read_policy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("repository root"),
        )
        .expect("checked-in policy")
    }

    #[test]
    fn own_manifest_and_wave_local_files_are_allowed() {
        let policy = policy();
        check_changed_paths(
            &policy,
            "w5",
            &[
                "delivery/manifests/w5.json".to_owned(),
                "packages/d2bd/src/service_v2.rs".to_owned(),
            ],
        )
        .expect("wave-local paths");
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
}
