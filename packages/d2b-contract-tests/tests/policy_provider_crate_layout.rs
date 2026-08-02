//! Provider crate layout policy.
//!
//! This policy is deliberately driven by Cargo metadata rather than the
//! spelling of the `members` array. It also scans the on-disk `packages/`
//! directory, because a Provider crate omitted from the workspace must not
//! disappear from policy coverage.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU32, Ordering},
};

use d2b_contract_tests::repo_root;
use serde::Deserialize;

const PROVIDER_PREFIX: &str = "d2b-provider-";
const NON_PROVIDER_PREFIXED: &[&str] = &[
    "d2b-provider",
    "d2b-provider-supervisor",
    "d2b-provider-toolkit",
];
const EXEMPT_LEGACY_CRATES: &[&str] = &["d2b-provider-aca", "d2b-provider-relay"];
const README_ONLY_INTEGRATION_RATCHET: &[&str] = &[
    "d2b-provider-credential-entra",
    "d2b-provider-credential-managed-identity",
    "d2b-provider-credential-secret-service",
    "d2b-provider-system-core",
    "d2b-provider-system-minijail",
    "d2b-provider-system-systemd",
    "d2b-provider-volume-virtiofs",
];
const REQUIRED_PATHS: &[&str] = &["src", "tests", "integration", "README.md"];
const REQUIRED_README_SECTIONS: &[&str] = &[
    "Provider identity",
    "Config schema",
    "Exported resource types",
    "Controllers / services / workers / binaries",
    "Placement and dependencies",
    "RBAC requirements",
    "Security posture",
    "State and telemetry",
    "Build and test",
];

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    workspace_members: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    manifest_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceMember {
    package_name: String,
    crate_dir: PathBuf,
    manifest_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OnDiskProvider {
    directory_name: String,
    manifest_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderNameKind {
    NonProvider,
    Legacy,
    Provider,
    Malformed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Violation {
    error: &'static str,
    crate_name: String,
    missing: Vec<String>,
}

impl Violation {
    fn render(&self) -> String {
        if self.missing.is_empty() {
            format!("{}: {}", self.error, diagnostic_name(&self.crate_name))
        } else {
            format!(
                "{}: {} ({})",
                self.error,
                diagnostic_name(&self.crate_name),
                self.missing.join(", ")
            )
        }
    }
}

fn workspace_policy(root: &Path) -> Result<Vec<Violation>, String> {
    let root = root
        .canonicalize()
        .map_err(|_| "provider-crate-layout-input-unreadable".to_owned())?;
    let members = cargo_workspace_members(&root)?;
    let on_disk = on_disk_providers(&root)?;
    let member_by_manifest: BTreeMap<PathBuf, &WorkspaceMember> = members
        .iter()
        .map(|member| (member.manifest_path.clone(), member))
        .collect();
    let mut violations = Vec::new();

    for member in &members {
        match provider_name_kind(&member.package_name) {
            ProviderNameKind::Provider => {
                if !is_provider_directory(&root, &member.crate_dir, &member.package_name) {
                    violations.push(Violation {
                        error: "provider-crate-location-invalid",
                        crate_name: member.package_name.clone(),
                        missing: Vec::new(),
                    });
                } else {
                    violations.extend(inspect_crate(member)?);
                }
            }
            ProviderNameKind::Malformed => violations.push(Violation {
                error: "provider-crate-name-invalid",
                crate_name: member.package_name.clone(),
                missing: Vec::new(),
            }),
            ProviderNameKind::NonProvider | ProviderNameKind::Legacy => {}
        }
    }

    for disk in on_disk {
        match member_by_manifest.get(&disk.manifest_path) {
            None => violations.push(Violation {
                error: "provider-crate-not-workspace-member",
                crate_name: disk.directory_name,
                missing: Vec::new(),
            }),
            Some(member) => {
                if member.package_name != disk.directory_name {
                    violations.push(Violation {
                        error: "provider-crate-name-mismatch",
                        crate_name: disk.directory_name.clone(),
                        missing: Vec::new(),
                    });
                }
                if provider_name_kind(&disk.directory_name) == ProviderNameKind::Malformed {
                    violations.push(Violation {
                        error: "provider-crate-name-invalid",
                        crate_name: disk.directory_name,
                        missing: Vec::new(),
                    });
                }
            }
        }
    }

    violations.sort_by(|left, right| {
        left.crate_name
            .cmp(&right.crate_name)
            .then_with(|| left.error.cmp(right.error))
            .then_with(|| left.missing.cmp(&right.missing))
    });
    violations.dedup();
    Ok(violations)
}

fn cargo_workspace_members(root: &Path) -> Result<Vec<WorkspaceMember>, String> {
    let metadata = cargo_metadata(root)?;
    let packages_by_id: BTreeMap<&str, &CargoPackage> = metadata
        .packages
        .iter()
        .map(|package| (package.id.as_str(), package))
        .collect();
    let mut members = Vec::new();
    for member_id in metadata.workspace_members {
        let package = packages_by_id
            .get(member_id.as_str())
            .ok_or_else(|| "provider-crate-layout-metadata-member-missing".to_owned())?;
        let manifest_path = package
            .manifest_path
            .canonicalize()
            .map_err(|_| "provider-crate-layout-member-invalid".to_owned())?;
        let crate_dir = manifest_path
            .parent()
            .ok_or_else(|| "provider-crate-layout-member-invalid".to_owned())?
            .to_owned();
        members.push(WorkspaceMember {
            package_name: package.name.clone(),
            crate_dir,
            manifest_path,
        });
    }
    if members.is_empty() {
        return Err("provider-crate-layout-members-empty".to_owned());
    }
    Ok(members)
}

fn cargo_metadata(root: &Path) -> Result<CargoMetadata, String> {
    let output = Command::new("cargo")
        .current_dir(root)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(root.join("packages/Cargo.toml"))
        .output()
        .map_err(|_| "provider-crate-layout-metadata-unavailable".to_owned())?;
    if !output.status.success() {
        return Err("provider-crate-layout-metadata-failed".to_owned());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| "provider-crate-layout-metadata-malformed".to_owned())
}

fn on_disk_providers(root: &Path) -> Result<Vec<OnDiskProvider>, String> {
    let entries = fs::read_dir(root.join("packages"))
        .map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?;
    let mut providers = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?;
        if !entry
            .file_type()
            .map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?
            .is_dir()
        {
            continue;
        }
        let directory_name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| "provider-crate-layout-member-invalid".to_owned())?
            .to_owned();
        if !directory_name.starts_with(PROVIDER_PREFIX) {
            continue;
        }
        let manifest_path = entry.path().join("Cargo.toml");
        if !manifest_path.is_file() {
            continue;
        }
        if matches!(
            provider_name_kind(&directory_name),
            ProviderNameKind::NonProvider | ProviderNameKind::Legacy
        ) {
            continue;
        }
        providers.push(OnDiskProvider {
            directory_name,
            manifest_path: manifest_path
                .canonicalize()
                .map_err(|_| "provider-crate-layout-member-invalid".to_owned())?,
        });
    }
    providers.sort_by(|left, right| left.directory_name.cmp(&right.directory_name));
    Ok(providers)
}

fn provider_name_kind(name: &str) -> ProviderNameKind {
    if NON_PROVIDER_PREFIXED.contains(&name) {
        return ProviderNameKind::NonProvider;
    }
    if EXEMPT_LEGACY_CRATES.contains(&name) {
        return ProviderNameKind::Legacy;
    }
    let Some(rest) = name.strip_prefix(PROVIDER_PREFIX) else {
        return ProviderNameKind::NonProvider;
    };
    let segments: Vec<_> = rest.split('-').collect();
    if segments.len() < 2 || segments.iter().any(|segment| !valid_name_segment(segment)) {
        ProviderNameKind::Malformed
    } else {
        ProviderNameKind::Provider
    }
}

fn valid_name_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 64
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn diagnostic_name(name: &str) -> String {
    if name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        name.to_owned()
    } else {
        "<invalid-provider-crate>".to_owned()
    }
}

fn is_provider_directory(root: &Path, crate_dir: &Path, package_name: &str) -> bool {
    let packages_dir = root.join("packages");
    crate_dir.parent() == Some(packages_dir.as_path())
        && crate_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == package_name)
}

fn inspect_crate(member: &WorkspaceMember) -> Result<Vec<Violation>, String> {
    let mut violations = Vec::new();
    let mut missing = Vec::new();
    for required in REQUIRED_PATHS {
        let path = member.crate_dir.join(required);
        let present = if *required == "README.md" {
            path.is_file()
        } else {
            path.is_dir()
        };
        if !present {
            missing.push((*required).to_owned());
        }
    }
    if member.crate_dir.join("src").is_dir() && !contains_rust_file(&member.crate_dir.join("src"))?
    {
        missing.push("src/*.rs".to_owned());
    }
    if member.crate_dir.join("tests").is_dir()
        && !contains_rust_file(&member.crate_dir.join("tests"))?
    {
        missing.push("tests/*.rs".to_owned());
    }
    let integration = member.crate_dir.join("integration");
    if integration.is_dir() {
        if !integration.join("README.md").is_file() {
            missing.push("integration/README.md".to_owned());
        }
        let has_rust_scenario = integration_has_rust_scenario(&integration)?;
        if !has_rust_scenario
            && !README_ONLY_INTEGRATION_RATCHET.contains(&member.package_name.as_str())
        {
            missing.push("integration/*.rs".to_owned());
        } else if has_rust_scenario
            && README_ONLY_INTEGRATION_RATCHET.contains(&member.package_name.as_str())
        {
            return Err("provider-crate-layout-stale-exemption".to_owned());
        }
    }
    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        violations.push(Violation {
            error: "missing-provider-crate-path",
            crate_name: member.package_name.clone(),
            missing,
        });
    }

    let readme = member.crate_dir.join("README.md");
    if readme.is_file() {
        let text = fs::read_to_string(&readme)
            .map_err(|_| "provider-crate-layout-readme-unreadable".to_owned())?;
        let present: BTreeSet<String> = text.lines().filter_map(heading_text).collect();
        let missing_sections = REQUIRED_README_SECTIONS
            .iter()
            .filter(|section| !present.contains(&section.to_lowercase()))
            .map(|section| format!("README.md section: {section}"))
            .collect::<Vec<_>>();
        if !missing_sections.is_empty() {
            violations.push(Violation {
                error: "missing-provider-readme-section",
                crate_name: member.package_name.clone(),
                missing: missing_sections,
            });
        }
    }
    Ok(violations)
}

fn heading_text(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let stripped = trimmed.strip_prefix('#')?;
    Some(stripped.trim_start_matches('#').trim().to_lowercase())
}

fn contains_rust_file(root: &Path) -> Result<bool, String> {
    let entries =
        fs::read_dir(root).map_err(|_| "provider-crate-layout-source-unreadable".to_owned())?;
    for entry in entries {
        let entry = entry.map_err(|_| "provider-crate-layout-source-unreadable".to_owned())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "provider-crate-layout-source-unreadable".to_owned())?;
        if file_type.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "rs")
        {
            return Ok(true);
        }
        if file_type.is_dir() && contains_rust_file(&entry.path())? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn integration_has_rust_scenario(integration: &Path) -> Result<bool, String> {
    let entries = fs::read_dir(integration)
        .map_err(|_| "provider-crate-layout-integration-unreadable".to_owned())?;
    for entry in entries {
        let entry = entry.map_err(|_| "provider-crate-layout-integration-unreadable".to_owned())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "provider-crate-layout-integration-unreadable".to_owned())?;
        if file_type.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "rs")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[test]
fn every_workspace_provider_has_the_normative_layout() {
    let violations = workspace_policy(&repo_root()).expect("read Provider workspace metadata");
    assert!(
        violations.is_empty(),
        "Provider crate layout policy violations:\n{}",
        violations
            .iter()
            .map(Violation::render)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn non_provider_workspace_helpers_are_not_in_layout_scope() {
    let root = repo_root();
    let members = cargo_workspace_members(&root).expect("read Cargo metadata");
    assert!(
        members
            .iter()
            .any(|member| { NON_PROVIDER_PREFIXED.contains(&member.package_name.as_str()) }),
        "expected a non-Provider helper member in the workspace"
    );
    let violations = workspace_policy(&root).expect("run Provider layout policy");
    assert!(
        violations
            .iter()
            .all(|violation| !NON_PROVIDER_PREFIXED.contains(&violation.crate_name.as_str())),
        "non-Provider helper was included in policy output: {violations:?}"
    );
}

#[test]
fn an_on_disk_provider_omitted_from_workspace_is_rejected() {
    let fixture = Fixture::from_repo_fixture("non-member");
    let violations = workspace_policy(&fixture.root).expect("run non-member fixture policy");
    assert!(
        violations.iter().any(|violation| {
            violation.error == "provider-crate-not-workspace-member"
                && violation.crate_name == "d2b-provider-fixture-omitted"
        }),
        "omitted Provider was not rejected: {violations:?}"
    );
}

#[test]
fn malformed_provider_name_is_rejected_instead_of_ignored() {
    let fixture = Fixture::from_repo_fixture("malformed");
    let violations = workspace_policy(&fixture.root).expect("run malformed fixture policy");
    assert!(
        violations.iter().any(|violation| {
            violation.error == "provider-crate-name-invalid"
                && violation.crate_name == "d2b-provider-fixture"
        }),
        "malformed Provider was not rejected: {violations:?}"
    );
}

static FIXTURE_COUNTER: AtomicU32 = AtomicU32::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn from_repo_fixture(name: &str) -> Self {
        let serial = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "d2b-provider-crate-layout-policy-{}-{serial}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let source = repo_root()
            .join("tests/fixtures/provider-crate-layout")
            .join(name);
        copy_tree(&source, &root).expect("copy Provider policy fixture");
        Self { root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
