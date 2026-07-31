//! Provider crate integration-layout policy.
//!
//! The broader Provider policy in `d2b-contract-tests` owns crate naming,
//! top-level paths, README sections, integration-target declarations,
//! identity, dossier parity, and dependency direction. This check owns only
//! the remaining package-layout rule: an in-scope Provider crate carries an
//! integration README and at least one Rust scenario. It runs directly under
//! the existing policy lane because the repository's shell-gate set is closed.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

const PROVIDER_PREFIX: &str = "d2b-provider-";
const NON_PROVIDER_PREFIXED: &[&str] = &[
    "d2b-provider",
    "d2b-provider-supervisor",
    "d2b-provider-toolkit",
];
const EXEMPT_LEGACY_CRATES: &[&str] = &["d2b-provider-aca", "d2b-provider-relay"];

// These crates predate this check and intentionally carry a README-only
// integration placeholder. The exact set is a ratchet: new crates receive no
// exemption, and an exemption becomes stale as soon as a Rust scenario lands.
const README_ONLY_INTEGRATION_RATCHET: &[&str] = &[
    "d2b-provider-credential-entra",
    "d2b-provider-credential-managed-identity",
    "d2b-provider-credential-secret-service",
    "d2b-provider-system-core",
    "d2b-provider-system-minijail",
    "d2b-provider-system-systemd",
    "d2b-provider-volume-virtiofs",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct MissingLayout {
    error: &'static str,
    #[serde(rename = "crate")]
    crate_name: String,
    missing: Vec<&'static str>,
}

impl MissingLayout {
    fn render(&self) -> String {
        serde_json::to_string(self).expect("fixed Provider layout diagnostic serializes")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceMember {
    name: String,
    relative_dir: PathBuf,
}

/// Check every in-scope Provider workspace member.
///
/// Diagnostics are one JSON object per violating crate. Dynamic content is
/// limited to a validated Cargo package name; missing values are fixed logical
/// crate-relative entries, never filesystem paths.
pub fn check(repo_root: &Path) -> Result<(), String> {
    let manifest_path = repo_root.join("packages/Cargo.toml");
    let manifest = fs::read_to_string(manifest_path)
        .map_err(|_| "provider-crate-layout-input-unreadable".to_owned())?;
    let members = parse_workspace_members(&manifest)?;
    let providers: Vec<_> = members
        .into_iter()
        .filter(|member| is_provider_crate(&member.name))
        .collect();
    if providers.is_empty() {
        return Err("provider-crate-layout-empty-scope".to_owned());
    }

    let mut violations = Vec::new();
    for member in &providers {
        let crate_dir = repo_root.join("packages").join(&member.relative_dir);
        let allow_readme_only = README_ONLY_INTEGRATION_RATCHET.contains(&member.name.as_str());
        if let Some(violation) = inspect_crate(&member.name, &crate_dir, allow_readme_only)? {
            violations.push(violation);
        }
    }
    violations.sort_by(|left, right| left.crate_name.cmp(&right.crate_name));

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations
            .iter()
            .map(MissingLayout::render)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn parse_workspace_members(manifest: &str) -> Result<Vec<WorkspaceMember>, String> {
    let members_start = manifest
        .find("members = [")
        .ok_or_else(|| "provider-crate-layout-members-missing".to_owned())?;
    let members = &manifest[members_start..];
    let members_end = members
        .find(']')
        .ok_or_else(|| "provider-crate-layout-members-malformed".to_owned())?;

    let mut parsed = Vec::new();
    for line in members[..members_end].lines().skip(1) {
        let candidate = line.trim().trim_end_matches(',');
        let Some(relative) = candidate
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        else {
            continue;
        };
        let relative_dir = validate_relative_member(relative)?;
        let name = relative_dir
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "provider-crate-layout-member-invalid".to_owned())?
            .to_owned();
        parsed.push(WorkspaceMember { name, relative_dir });
    }
    if parsed.is_empty() {
        return Err("provider-crate-layout-members-empty".to_owned());
    }
    Ok(parsed)
}

fn validate_relative_member(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            !matches!(
                component,
                std::path::Component::Normal(segment)
                    if segment.to_str().is_some_and(valid_member_segment)
            )
        })
    {
        return Err("provider-crate-layout-member-invalid".to_owned());
    }
    Ok(path.to_owned())
}

fn valid_member_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_provider_crate(name: &str) -> bool {
    if NON_PROVIDER_PREFIXED.contains(&name) || EXEMPT_LEGACY_CRATES.contains(&name) {
        return false;
    }
    let Some(rest) = name.strip_prefix(PROVIDER_PREFIX) else {
        return false;
    };
    rest.split_once('-')
        .is_some_and(|(base, implementation)| !base.is_empty() && !implementation.is_empty())
}

fn inspect_crate(
    crate_name: &str,
    crate_dir: &Path,
    allow_readme_only: bool,
) -> Result<Option<MissingLayout>, String> {
    let mut missing = Vec::new();
    let integration = crate_dir.join("integration");
    if integration.is_dir() {
        if !integration.join("README.md").is_file() {
            missing.push("integration/README.md");
        }
        let has_rust_scenario = integration_has_rust_scenario(&integration)?;
        if !has_rust_scenario && !allow_readme_only {
            missing.push("integration/*.rs");
        }
        if has_rust_scenario && allow_readme_only {
            return Err("provider-crate-layout-stale-exemption".to_owned());
        }
    }

    missing.sort_unstable();
    missing.dedup();
    Ok((!missing.is_empty()).then(|| MissingLayout {
        error: "missing-provider-crate-path",
        crate_name: crate_name.to_owned(),
        missing,
    }))
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
            fs::create_dir_all(root.join("packages/d2b-provider-fixture-example/src")).unwrap();
            fs::create_dir_all(root.join("packages/d2b-provider-fixture-example/tests")).unwrap();
            fs::create_dir_all(root.join("packages/d2b-provider-fixture-example/integration"))
                .unwrap();
            fs::create_dir_all(root.join("packages/d2b-core/src")).unwrap();
            fs::write(
                root.join("packages/Cargo.toml"),
                "[workspace]\nmembers = [\n    \"d2b-core\",\n    \"d2b-provider-fixture-example\",\n]\n",
            )
            .unwrap();
            fs::write(
                root.join("packages/d2b-provider-fixture-example/README.md"),
                "# fixture\n",
            )
            .unwrap();
            fs::write(
                root.join("packages/d2b-provider-fixture-example/integration/README.md"),
                "# fixtures\n",
            )
            .unwrap();
            fs::write(
                root.join("packages/d2b-provider-fixture-example/integration/scenario.rs"),
                "//! integration-target: container\n",
            )
            .unwrap();
            Self { root }
        }

        fn crate_dir(&self) -> PathBuf {
            self.root.join("packages/d2b-provider-fixture-example")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn conforming_tree_is_idempotent_and_non_provider_members_are_ignored() {
        let fixture = Fixture::new("clean");
        assert_eq!(check(&fixture.root), Ok(()));
        assert_eq!(check(&fixture.root), Ok(()));
    }

    #[test]
    fn integration_readme_and_rust_scenario_are_both_required() {
        let fixture = Fixture::new("integration");
        fs::remove_file(fixture.crate_dir().join("integration/README.md")).unwrap();
        fs::remove_file(fixture.crate_dir().join("integration/scenario.rs")).unwrap();

        let error = check(&fixture.root).unwrap_err();
        eprintln!("synthetic perturbation rejected: {error}");
        assert_eq!(
            error,
            r#"{"error":"missing-provider-crate-path","crate":"d2b-provider-fixture-example","missing":["integration/*.rs","integration/README.md"]}"#
        );
    }

    #[test]
    fn empty_provider_scope_fails_closed() {
        let fixture = Fixture::new("empty");
        fs::write(
            fixture.root.join("packages/Cargo.toml"),
            "[workspace]\nmembers = [\n    \"d2b-core\",\n]\n",
        )
        .unwrap();
        assert_eq!(
            check(&fixture.root),
            Err("provider-crate-layout-empty-scope".to_owned())
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
        assert_eq!(error, "provider-crate-layout-member-invalid");
        assert!(!error.contains(&marker));
    }
}
