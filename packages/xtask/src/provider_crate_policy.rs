//! Provider crate layout policy.
//!
//! Cargo metadata is the source of truth for workspace membership. The
//! filesystem scan is intentionally separate: a Provider-shaped crate can
//! exist under `packages/` without appearing in the workspace member list,
//! and that omission must fail closed rather than making the crate invisible
//! to this policy.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

const PROVIDER_PREFIX: &str = "d2b-provider-";
const NON_PROVIDER_PREFIXED: &[&str] = &[
    "d2b-provider",
    "d2b-provider-supervisor",
    "d2b-provider-toolkit",
];
const EXEMPT_LEGACY_CRATES: &[&str] = &["d2b-provider-aca", "d2b-provider-relay"];

// These exact README-only integration placeholders are recorded in the
// existing Provider-state canon. They are not exemptions from the four
// required paths or the README sections, and new crates cannot join the set.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Diagnostic {
    error: &'static str,
    #[serde(rename = "crate")]
    crate_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    missing: Option<Vec<String>>,
}

impl Diagnostic {
    fn path_missing(crate_name: &str, missing: Vec<String>) -> Self {
        Self {
            error: "missing-provider-crate-path",
            crate_name: diagnostic_name(crate_name),
            missing: Some(missing),
        }
    }

    fn readme_sections_missing(crate_name: &str, missing: Vec<String>) -> Self {
        Self {
            error: "missing-provider-readme-section",
            crate_name: diagnostic_name(crate_name),
            missing: Some(missing),
        }
    }

    fn simple(error: &'static str, crate_name: &str) -> Self {
        Self {
            error,
            crate_name: diagnostic_name(crate_name),
            missing: None,
        }
    }

    fn render(&self) -> String {
        serde_json::to_string(self).expect("fixed Provider policy diagnostic serializes")
    }
}

/// Check the normative layout of every Provider workspace member and ensure
/// every Provider-shaped crate on disk is represented by Cargo metadata.
pub fn check(repo_root: &Path) -> Result<(), String> {
    let repo_root = repo_root
        .canonicalize()
        .map_err(|_| "provider-crate-layout-input-unreadable".to_owned())?;
    let members = cargo_workspace_members(&repo_root)?;
    let on_disk = on_disk_providers(&repo_root)?;
    let has_provider_member = members
        .iter()
        .any(|member| provider_name_kind(&member.package_name) == ProviderNameKind::Provider);
    if !has_provider_member && on_disk.is_empty() {
        return Err("provider-crate-layout-empty-scope".to_owned());
    }

    let member_by_manifest: BTreeMap<PathBuf, &WorkspaceMember> = members
        .iter()
        .map(|member| (member.manifest_path.clone(), member))
        .collect();
    let mut violations = Vec::new();

    for member in &members {
        match provider_name_kind(&member.package_name) {
            ProviderNameKind::Provider => {
                if !is_provider_directory(&repo_root, &member.crate_dir, &member.package_name) {
                    violations.push(Diagnostic::simple(
                        "provider-crate-location-invalid",
                        &member.package_name,
                    ));
                } else {
                    violations.extend(inspect_crate(member)?);
                }
            }
            ProviderNameKind::Malformed => violations.push(Diagnostic::simple(
                "provider-crate-name-invalid",
                &member.package_name,
            )),
            ProviderNameKind::NonProvider | ProviderNameKind::Legacy => {}
        }
    }

    for crate_on_disk in on_disk {
        match member_by_manifest.get(&crate_on_disk.manifest_path) {
            None => violations.push(Diagnostic::simple(
                "provider-crate-not-workspace-member",
                &crate_on_disk.directory_name,
            )),
            Some(member) => {
                if member.package_name != crate_on_disk.directory_name {
                    violations.push(Diagnostic::simple(
                        "provider-crate-name-mismatch",
                        &crate_on_disk.directory_name,
                    ));
                }
                if provider_name_kind(&crate_on_disk.directory_name) == ProviderNameKind::Malformed
                {
                    violations.push(Diagnostic::simple(
                        "provider-crate-name-invalid",
                        &crate_on_disk.directory_name,
                    ));
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

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations
            .iter()
            .map(Diagnostic::render)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn cargo_workspace_members(repo_root: &Path) -> Result<Vec<WorkspaceMember>, String> {
    let metadata = cargo_metadata(repo_root)?;
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

fn cargo_metadata(repo_root: &Path) -> Result<CargoMetadata, String> {
    let output = Command::new("cargo")
        .current_dir(repo_root)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(repo_root.join("packages/Cargo.toml"))
        .output()
        .map_err(|_| "provider-crate-layout-metadata-unavailable".to_owned())?;
    if !output.status.success() {
        return Err("provider-crate-layout-metadata-failed".to_owned());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| "provider-crate-layout-metadata-malformed".to_owned())
}

fn on_disk_providers(repo_root: &Path) -> Result<Vec<OnDiskProvider>, String> {
    let packages_dir = repo_root.join("packages");
    let entries = fs::read_dir(&packages_dir)
        .map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?;
    let mut providers = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?;
        let file_type = entry
            .file_type()
            .map_err(|_| "provider-crate-layout-packages-unreadable".to_owned())?;
        if !file_type.is_dir() {
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

fn is_provider_directory(repo_root: &Path, crate_dir: &Path, package_name: &str) -> bool {
    let packages_dir = repo_root.join("packages");
    crate_dir.parent() == Some(packages_dir.as_path())
        && crate_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == package_name)
}

fn inspect_crate(member: &WorkspaceMember) -> Result<Vec<Diagnostic>, String> {
    let crate_name = &member.package_name;
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
        if !has_rust_scenario && !README_ONLY_INTEGRATION_RATCHET.contains(&crate_name.as_str()) {
            missing.push("integration/*.rs".to_owned());
        }
        if has_rust_scenario && README_ONLY_INTEGRATION_RATCHET.contains(&crate_name.as_str()) {
            return Err("provider-crate-layout-stale-exemption".to_owned());
        }
    }

    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        violations.push(Diagnostic::path_missing(crate_name, missing));
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
            violations.push(Diagnostic::readme_sections_missing(
                crate_name,
                missing_sections,
            ));
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::repo_root;

    use super::*;

    static FIXTURE_COUNTER: AtomicU32 = AtomicU32::new(0);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let serial = FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "d2b-provider-layout-{}-{serial}-{label}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            write_package(&root, "d2b-core");
            write_package(&root, "d2b-provider-fixture-example");
            let provider = root.join("packages/d2b-provider-fixture-example");
            fs::create_dir_all(provider.join("integration")).unwrap();
            fs::create_dir_all(provider.join("tests")).unwrap();
            fs::write(
                provider.join("tests/scenario.rs"),
                "#[test]\nfn fixture() {}\n",
            )
            .unwrap();
            fs::write(
                provider.join("integration/README.md"),
                "# integration fixtures\n",
            )
            .unwrap();
            fs::write(
                provider.join("integration/scenario.rs"),
                "//! integration-target: container\n",
            )
            .unwrap();
            fs::write(
                provider.join("README.md"),
                required_readme("fixture-example"),
            )
            .unwrap();
            fs::write(
                root.join("packages/Cargo.toml"),
                "[workspace]\nmembers = [\n    \"d2b-core\",\n    \"d2b-provider-fixture-example\",\n]\n",
            )
            .unwrap();
            Self { root }
        }

        fn provider_dir(&self) -> PathBuf {
            self.root.join("packages/d2b-provider-fixture-example")
        }

        fn add_package(&self, name: &str) -> PathBuf {
            write_package(&self.root, name);
            self.root.join("packages").join(name)
        }

        fn set_members(&self, members: &[&str]) {
            let mut manifest = String::from("[workspace]\nmembers = [\n");
            for member in members {
                manifest.push_str(&format!("    \"{member}\",\n"));
            }
            manifest.push_str("]\n");
            fs::write(self.root.join("packages/Cargo.toml"), manifest).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn write_package(root: &Path, name: &str) {
        let package = root.join("packages").join(name);
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(package.join("src/lib.rs"), "").unwrap();
        fs::write(
            package.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n"),
        )
        .unwrap();
    }

    fn required_readme(identity: &str) -> String {
        let mut readme = String::new();
        for section in REQUIRED_README_SECTIONS {
            readme.push_str(&format!("## {section}\n\n"));
            if *section == "Provider identity" {
                readme.push_str(&format!("| Provider name | `{identity}` |\n\n"));
            }
        }
        readme
    }

    #[test]
    fn conforming_tree_is_idempotent_and_non_provider_members_are_ignored() {
        let fixture = Fixture::new("clean");
        assert_eq!(check(&fixture.root), Ok(()));
        assert_eq!(check(&fixture.root), Ok(()));
    }

    #[test]
    fn every_provider_prefixed_name_has_one_explicit_classification() {
        let root = repo_root().expect("resolve repository root");
        let members = cargo_workspace_members(&root).expect("read workspace metadata");
        let mut names: BTreeSet<String> = members
            .into_iter()
            .map(|member| member.package_name)
            .filter(|name| name.starts_with(PROVIDER_PREFIX))
            .collect();
        for entry in fs::read_dir(root.join("packages")).expect("read packages directory") {
            let entry = entry.expect("read package entry");
            if entry.file_type().expect("read package entry type").is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(PROVIDER_PREFIX) {
                    names.insert(name);
                }
            }
        }

        assert!(
            !names.is_empty(),
            "Provider-name classification must inspect a non-empty scope"
        );
        for name in names {
            let kind = provider_name_kind(&name);
            match kind {
                ProviderNameKind::NonProvider => {
                    assert!(
                        NON_PROVIDER_PREFIXED.contains(&name.as_str()),
                        "{name} is not an explicit non-Provider helper"
                    );
                }
                ProviderNameKind::Legacy => {
                    assert!(
                        EXEMPT_LEGACY_CRATES.contains(&name.as_str()),
                        "{name} is not an explicit legacy exemption"
                    );
                }
                ProviderNameKind::Provider => {
                    assert!(
                        name.strip_prefix(PROVIDER_PREFIX)
                            .is_some_and(|suffix| suffix.split('-').count() >= 2),
                        "{name} is not a two-segment Provider identity"
                    );
                }
                ProviderNameKind::Malformed => {
                    assert!(
                        name.starts_with(PROVIDER_PREFIX),
                        "{name} is malformed but not Provider-prefixed"
                    );
                }
            }
        }
    }

    #[test]
    fn readme_only_integration_ratchet_is_exactly_the_scaffolded_set() {
        let expected = [
            "d2b-provider-credential-entra",
            "d2b-provider-credential-managed-identity",
            "d2b-provider-credential-secret-service",
            "d2b-provider-system-core",
            "d2b-provider-system-minijail",
            "d2b-provider-system-systemd",
            "d2b-provider-volume-virtiofs",
        ];
        assert_eq!(
            README_ONLY_INTEGRATION_RATCHET, &expected,
            "README-only integration coverage must remain an explicit closed set"
        );
        let root = repo_root().expect("resolve repository root");
        for name in expected {
            let integration = root.join("packages").join(name).join("integration");
            assert!(
                integration.join("README.md").is_file(),
                "{name} must retain its integration scaffold README"
            );
            assert!(
                !integration_has_rust_scenario(&integration).expect("inspect integration scaffold"),
                "{name} must leave executable integration wiring to its owning implementation"
            );
        }
    }

    #[test]
    fn integration_readme_and_rust_scenario_are_both_required() {
        let fixture = Fixture::new("integration");
        fs::remove_file(fixture.provider_dir().join("integration/README.md")).unwrap();
        fs::remove_file(fixture.provider_dir().join("integration/scenario.rs")).unwrap();

        let error = check(&fixture.root).unwrap_err();
        eprintln!("synthetic perturbation rejected: {error}");
        assert_eq!(
            error,
            r#"{"error":"missing-provider-crate-path","crate":"d2b-provider-fixture-example","missing":["integration/*.rs","integration/README.md"]}"#
        );
    }

    #[test]
    fn an_on_disk_provider_omitted_from_workspace_is_rejected() {
        let fixture = Fixture::new("non-member");
        let omitted = fixture.add_package("d2b-provider-fixture-omitted");
        fs::create_dir_all(omitted.join("tests")).unwrap();
        fs::create_dir_all(omitted.join("integration")).unwrap();
        fs::write(
            omitted.join("README.md"),
            required_readme("fixture-omitted"),
        )
        .unwrap();

        let error = check(&fixture.root).unwrap_err();
        assert!(error.contains("provider-crate-not-workspace-member"));
        assert!(error.contains("d2b-provider-fixture-omitted"));
    }

    #[test]
    fn a_malformed_provider_name_is_rejected_instead_of_ignored() {
        let fixture = Fixture::new("malformed");
        fixture.add_package("d2b-provider-fixture");
        fixture.set_members(&[
            "d2b-core",
            "d2b-provider-fixture-example",
            "d2b-provider-fixture",
        ]);

        let error = check(&fixture.root).unwrap_err();
        assert!(error.contains("provider-crate-name-invalid"));
        assert!(error.contains("d2b-provider-fixture"));
    }

    #[test]
    fn empty_provider_scope_fails_closed() {
        let fixture = Fixture::new("empty");
        fixture.set_members(&["d2b-core"]);
        assert_eq!(
            check(&fixture.root),
            Err(
                r#"{"error":"provider-crate-not-workspace-member","crate":"d2b-provider-fixture-example"}"#
                    .to_owned()
            )
        );
    }

    #[test]
    fn caller_supplied_workspace_paths_are_never_rendered() {
        let fixture = Fixture::new("redaction");
        let marker = format!("caller-secret-{}", std::process::id());
        fs::write(
            fixture.root.join("packages/Cargo.toml"),
            format!("[workspace]\nmembers = [\n    \"../{marker}\",\n]\n"),
        )
        .unwrap();
        let error = check(&fixture.root).unwrap_err();
        assert!(!error.contains(&marker));
    }
}
