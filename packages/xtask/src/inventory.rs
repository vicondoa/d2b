use std::{
    collections::BTreeSet,
    env, fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};

use serde::Serialize;

const INVENTORY_SCHEMA_VERSION: u32 = 1;
const TOP_FILE_LIMIT: usize = 25;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct Inventory {
    schema_version: u32,
    source: &'static str,
    path_policy: PathPolicy,
    totals: Totals,
    largest_rust_source_files: Vec<FileMetric>,
    largest_nix_modules: Vec<FileMetric>,
    crates: Vec<CrateInfo>,
    workspace: WorkspaceInfo,
    candidate_compatibility_markers: Vec<CompatibilityMarker>,
    generated_artifact_surfaces: Vec<ClassifiedPath>,
    test_driver_surfaces: Vec<ClassifiedPath>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct PathPolicy {
    repository_relative_paths_only: bool,
    includes_absolute_host_paths: bool,
    includes_timestamps_or_pids: bool,
    includes_line_content: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct Totals {
    tracked_files: usize,
    rust_source_files: usize,
    nix_files: usize,
    crate_manifests: usize,
    compatibility_marker_locations: usize,
    generated_artifact_surfaces: usize,
    test_driver_surfaces: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct FileMetric {
    path: String,
    bytes: u64,
    lines: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CrateInfo {
    name: String,
    manifest_path: String,
    package_dir: String,
    workspace_role: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct WorkspaceInfo {
    root_manifest: String,
    members: Vec<String>,
    excludes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CompatibilityMarker {
    path: String,
    line: usize,
    marker: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct ClassifiedPath {
    path: String,
    kind: &'static str,
}

pub(crate) fn emit_adr0035_inventory(
    output_path: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = super::repo_root()?;
    let tracked_files = git_tracked_files(repo_root)?;
    let inventory = build_inventory(repo_root, &tracked_files)?;
    let mut json = serde_json::to_string_pretty(&inventory)?;
    json.push('\n');

    if let Some(path) = output_path {
        validate_output_path(repo_root, path, &tracked_files)?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)?;
    } else {
        io::stdout().write_all(json.as_bytes())?;
    }

    Ok(())
}

fn git_tracked_files(repo_root: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("ls-files")
        .arg("-z")
        .output()?;
    if !output.status.success() {
        return Err(format!("git ls-files failed with status {}", output.status).into());
    }

    let mut files = Vec::new();
    for raw in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|raw| !raw.is_empty())
    {
        let path = String::from_utf8(raw.to_vec())?;
        validate_repo_relative_path(&path)?;
        files.push(path);
    }
    files.sort();
    Ok(files)
}

fn build_inventory(
    repo_root: &Path,
    tracked_files: &[String],
) -> Result<Inventory, Box<dyn std::error::Error>> {
    let mut rust_metrics = Vec::new();
    let mut nix_metrics = Vec::new();
    let mut crate_manifest_paths = Vec::new();
    let mut compatibility_markers = Vec::new();
    let mut generated_surfaces = Vec::new();
    let mut test_driver_surfaces = Vec::new();

    for path in tracked_files {
        if is_rust_source(path) {
            rust_metrics.push(file_metric(repo_root, path)?);
        }
        if is_nix_file(path) {
            nix_metrics.push(file_metric(repo_root, path)?);
        }
        if is_crate_manifest(path) {
            crate_manifest_paths.push(path.clone());
        }
        if let Some(kind) = classify_generated_surface(path) {
            generated_surfaces.push(ClassifiedPath {
                path: path.clone(),
                kind,
            });
        }
        if let Some(kind) = classify_test_driver_surface(path) {
            test_driver_surfaces.push(ClassifiedPath {
                path: path.clone(),
                kind,
            });
        }
        compatibility_markers.extend(scan_compatibility_markers(repo_root, path)?);
    }

    sort_metrics(&mut rust_metrics);
    sort_metrics(&mut nix_metrics);
    generated_surfaces
        .sort_by(|left, right| left.kind.cmp(right.kind).then(left.path.cmp(&right.path)));
    test_driver_surfaces
        .sort_by(|left, right| left.kind.cmp(right.kind).then(left.path.cmp(&right.path)));
    compatibility_markers.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.marker.cmp(&right.marker))
    });

    let workspace = workspace_info(repo_root)?;
    let crates = crate_info(repo_root, &crate_manifest_paths, &workspace)?;

    let totals = Totals {
        tracked_files: tracked_files.len(),
        rust_source_files: rust_metrics.len(),
        nix_files: nix_metrics.len(),
        crate_manifests: crate_manifest_paths.len(),
        compatibility_marker_locations: compatibility_markers.len(),
        generated_artifact_surfaces: generated_surfaces.len(),
        test_driver_surfaces: test_driver_surfaces.len(),
    };

    rust_metrics.truncate(TOP_FILE_LIMIT);
    nix_metrics.truncate(TOP_FILE_LIMIT);

    Ok(Inventory {
        schema_version: INVENTORY_SCHEMA_VERSION,
        source: "git ls-files -z",
        path_policy: PathPolicy {
            repository_relative_paths_only: true,
            includes_absolute_host_paths: false,
            includes_timestamps_or_pids: false,
            includes_line_content: false,
        },
        totals,
        largest_rust_source_files: rust_metrics,
        largest_nix_modules: nix_metrics,
        crates,
        workspace,
        candidate_compatibility_markers: compatibility_markers,
        generated_artifact_surfaces: generated_surfaces,
        test_driver_surfaces,
    })
}

fn validate_repo_relative_path(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let candidate = Path::new(path);
    if candidate.is_absolute()
        || candidate.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("git reported non-repository-relative path: {path}").into());
    }
    Ok(())
}

fn validate_output_path(
    repo_root: &Path,
    output_path: &Path,
    tracked_files: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(repo_relative) = output_repo_relative_path(repo_root, output_path)? {
        if repo_relative == "docs" || repo_relative.starts_with("docs/") {
            return Err("inventory output must not be written under tracked docs/".into());
        }
        if tracked_files.binary_search(&repo_relative).is_ok() {
            return Err(format!(
                "inventory output must not overwrite tracked file {repo_relative}"
            )
            .into());
        }
    }
    Ok(())
}

fn output_repo_relative_path(
    repo_root: &Path,
    output_path: &Path,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let absolute = normalize_path(if output_path.is_absolute() {
        output_path.to_path_buf()
    } else {
        env::current_dir()?.join(output_path)
    });
    match absolute.strip_prefix(repo_root) {
        Ok(relative) => Ok(Some(path_to_repo_string(relative)?)),
        Err(_) => Ok(None),
    }
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn path_to_repo_string(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let value = path
        .to_str()
        .ok_or_else(|| "path is not valid UTF-8".to_owned())?
        .trim_start_matches("./")
        .to_owned();
    validate_repo_relative_path(&value)?;
    Ok(value)
}

fn is_rust_source(path: &str) -> bool {
    path.ends_with(".rs")
}

fn is_nix_file(path: &str) -> bool {
    path.ends_with(".nix")
}

fn is_crate_manifest(path: &str) -> bool {
    path != "packages/Cargo.toml" && path.starts_with("packages/") && path.ends_with("/Cargo.toml")
}

fn file_metric(repo_root: &Path, path: &str) -> Result<FileMetric, Box<dyn std::error::Error>> {
    let absolute = repo_root.join(path);
    let bytes = fs::metadata(&absolute)?.len();
    let data = fs::read(&absolute)?;
    let lines = data.iter().filter(|byte| **byte == b'\n').count()
        + usize::from(data.last().is_some_and(|byte| *byte != b'\n'));
    Ok(FileMetric {
        path: path.to_owned(),
        bytes,
        lines,
    })
}

fn sort_metrics(metrics: &mut [FileMetric]) {
    metrics.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then(left.path.cmp(&right.path))
    });
}

fn workspace_info(repo_root: &Path) -> Result<WorkspaceInfo, Box<dyn std::error::Error>> {
    let root_manifest = "packages/Cargo.toml";
    let root_toml = fs::read_to_string(repo_root.join(root_manifest))?;
    Ok(WorkspaceInfo {
        root_manifest: root_manifest.to_owned(),
        members: parse_toml_string_array(&root_toml, "members"),
        excludes: parse_toml_string_array(&root_toml, "exclude"),
    })
}

fn crate_info(
    repo_root: &Path,
    manifest_paths: &[String],
    workspace: &WorkspaceInfo,
) -> Result<Vec<CrateInfo>, Box<dyn std::error::Error>> {
    let member_dirs: BTreeSet<&str> = workspace.members.iter().map(String::as_str).collect();
    let exclude_dirs: BTreeSet<&str> = workspace.excludes.iter().map(String::as_str).collect();
    let mut crates = Vec::new();
    for manifest_path in manifest_paths {
        let cargo_toml = fs::read_to_string(repo_root.join(manifest_path))?;
        let name = parse_package_name(&cargo_toml)
            .ok_or_else(|| format!("missing [package] name in {manifest_path}"))?;
        let package_dir = manifest_path
            .strip_prefix("packages/")
            .and_then(|path| path.strip_suffix("/Cargo.toml"))
            .ok_or_else(|| format!("unexpected package manifest path {manifest_path}"))?
            .to_owned();
        let workspace_role = if member_dirs.contains(package_dir.as_str()) {
            "workspace-member"
        } else if exclude_dirs.contains(package_dir.as_str()) {
            "workspace-excluded"
        } else {
            "standalone"
        }
        .to_owned();
        crates.push(CrateInfo {
            name,
            manifest_path: manifest_path.clone(),
            package_dir,
            workspace_role,
        });
    }
    crates.sort_by(|left, right| left.package_dir.cmp(&right.package_dir));
    Ok(crates)
}

fn parse_package_name(toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if in_package && trimmed.starts_with('[') {
            return None;
        }
        if in_package && trimmed.starts_with("name") {
            return parse_toml_assignment_string(trimmed, "name");
        }
    }
    None
}

fn parse_toml_string_array(toml: &str, key: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut capturing = false;
    let assignment = format!("{key} =");

    for line in toml.lines() {
        let trimmed = line.trim();
        if !capturing && !trimmed.starts_with(&assignment) {
            continue;
        }
        capturing = true;
        values.extend(extract_quoted_strings(trimmed));
        if trimmed.contains(']') {
            break;
        }
    }

    values.sort();
    values
}

fn parse_toml_assignment_string(line: &str, key: &str) -> Option<String> {
    let assignment = format!("{key} =");
    line.strip_prefix(&assignment)
        .and_then(|value| extract_quoted_strings(value).into_iter().next())
}

fn extract_quoted_strings(input: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut remainder = input;
    while let Some(start) = remainder.find('"') {
        let after_start = &remainder[start + 1..];
        if let Some(end) = after_start.find('"') {
            values.push(after_start[..end].to_owned());
            remainder = &after_start[end + 1..];
        } else {
            break;
        }
    }
    values
}

fn scan_compatibility_markers(
    repo_root: &Path,
    path: &str,
) -> Result<Vec<CompatibilityMarker>, Box<dyn std::error::Error>> {
    let Ok(text) = fs::read_to_string(repo_root.join(path)) else {
        return Ok(Vec::new());
    };
    let mut markers = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        for token in compat_adr_tokens(line) {
            markers.push(CompatibilityMarker {
                path: path.to_owned(),
                line: line_number,
                marker: token,
            });
        }
        let lowered = line.to_ascii_lowercase();
        for term in compatibility_terms(&lowered) {
            markers.push(CompatibilityMarker {
                path: path.to_owned(),
                line: line_number,
                marker: term.to_owned(),
            });
        }
    }
    Ok(markers)
}

fn compat_adr_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut remainder = line;
    while let Some(start) = remainder.find("compat-ADR") {
        let candidate = &remainder[start..];
        let end = candidate
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
            .unwrap_or(candidate.len());
        tokens.push(candidate[..end].to_owned());
        remainder = &candidate[end..];
    }
    tokens
}

fn compatibility_terms(line_lowercase: &str) -> Vec<&'static str> {
    const TERMS: &[&str] = &[
        "back-compat",
        "backward compatibility",
        "compatibility",
        "deprecated",
        "fallback",
        "legacy",
        "retired",
        "shim",
        "superseded",
        "tombstone",
    ];

    TERMS
        .iter()
        .copied()
        .filter(|term| line_lowercase.contains(term))
        .collect()
}

fn classify_generated_surface(path: &str) -> Option<&'static str> {
    if path == ".github/workflows/pr-l1-static-fast.yml" {
        Some("generated-ci-workflow")
    } else if path == "tests/layer1-jobs.json" {
        Some("ci-workflow-source")
    } else if path.starts_with("docs/reference/schemas/") && path.ends_with(".json") {
        Some("contract-schema-json")
    } else if path.starts_with("docs/reference/cli-output/") && path.ends_with(".json") {
        Some("cli-output-schema-json")
    } else if path.starts_with("docs/manpages/") {
        Some("cli-manpage")
    } else if path.starts_with("docs/completions/") {
        Some("cli-completion")
    } else if path.starts_with("packages/nixling-ipc/proto/") && path.ends_with(".proto") {
        Some("guest-control-proto-source")
    } else if path.contains("/src/generated/") {
        Some("generated-rust-binding")
    } else if path == "docs/reference/daemon-api.md" || path == "docs/reference/error-codes.md" {
        Some("generated-reference-doc")
    } else if path == "packages/xtask/src/main.rs" || path.starts_with("packages/xtask/src/bin/") {
        Some("generator-source")
    } else if path.starts_with("nixos-modules/") && path.ends_with("-json.nix") {
        Some("nix-json-emitter")
    } else if path == "nixos-modules/manifest.nix" {
        Some("nix-manifest-emitter")
    } else {
        None
    }
}

fn classify_test_driver_surface(path: &str) -> Option<&'static str> {
    if path == "Makefile" {
        Some("make-targets")
    } else if path == "tests/layer1-jobs.json" {
        Some("layer1-job-manifest")
    } else if path.starts_with(".github/workflows/") && path.ends_with(".yml") {
        Some("ci-workflow")
    } else if path.starts_with("tests/test-") && path.ends_with(".sh") {
        Some("make-target-driver")
    } else if path == "tests/static.sh"
        || path == "tests/static-fast-tier0.sh"
        || path == "tests/runner.sh"
    {
        Some("top-level-shell-driver")
    } else if path.starts_with("tests/tools/") {
        Some("test-tooling")
    } else if path.starts_with("tests/unit/gates/") {
        Some("drift-gate")
    } else if path.starts_with("tests/unit/meta/") {
        Some("policy-gate")
    } else if path == "tests/migration-ledger.toml" || path.starts_with("tests/migration-state.d/")
    {
        Some("migration-ledger")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_string_arrays_are_sorted_and_extracted() {
        let toml = r#"
[workspace]
members = [
    "zeta",
    "alpha",
]
exclude = ["standalone"]
"#;

        assert_eq!(parse_toml_string_array(toml, "members"), ["alpha", "zeta"]);
        assert_eq!(parse_toml_string_array(toml, "exclude"), ["standalone"]);
    }

    #[test]
    fn compatibility_scan_emits_tokens_not_line_content() {
        let line = "compat-ADR0035-added-20260622-cli-example legacy fallback secret=value";

        assert_eq!(
            compat_adr_tokens(line),
            ["compat-ADR0035-added-20260622-cli-example"]
        );
        assert_eq!(
            compatibility_terms(&line.to_ascii_lowercase()),
            ["fallback", "legacy"]
        );
    }

    #[test]
    fn generated_and_test_surfaces_are_classified() {
        assert_eq!(
            classify_generated_surface("docs/reference/schemas/v2/bundle.json"),
            Some("contract-schema-json")
        );
        assert_eq!(
            classify_test_driver_surface("tests/test-rust.sh"),
            Some("make-target-driver")
        );
    }

    #[test]
    fn rejects_non_repo_relative_paths() {
        assert!(validate_repo_relative_path("packages/xtask/src/main.rs").is_ok());
        assert!(validate_repo_relative_path("/home/example/repo/file").is_err());
        assert!(validate_repo_relative_path("../file").is_err());
    }
}
