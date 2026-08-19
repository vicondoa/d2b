#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const CONTRACTS_CRATE: &str = "d2b-contracts";
const EXCLUDED_WORKSPACES: &[&str] = &[];
const INDEPENDENT_WORKSPACE_ROOTS: &[&str] = &[
    "packages/d2b-bus/tests/ui/public-api-mutations",
    "packages/d2b-controller-toolkit/tests/ui/external-seals",
    "packages/d2b-core/fuzz",
    "packages/d2b-resource-api/tests/ui/external-seals",
];
const RELEASE_WORKFLOW: &str = ".github/workflows/release-host-binaries.yml";
const RELEASE_BINARY_SELECTORS: &[(&str, &str, &str)] = &[
    ("d2bd", "d2bd", "Cargo.toml"),
    ("d2b", "d2b", "Cargo.toml"),
    ("d2b-wayland-proxy", "d2b-wayland-proxy", "Cargo.toml"),
    (
        "d2b-unsafe-local-helper",
        "d2b-unsafe-local-helper",
        "Cargo.toml",
    ),
    ("d2b-host", "d2b-activation-helper", "Cargo.toml"),
    ("d2b-priv-broker", "d2b-priv-broker", "Cargo.toml"),
];
fn repo_root() -> PathBuf {
    let mut candidates = Vec::new();
    for variable in ["D2B_REPO_ROOT", "TEST_SRCDIR", "RUNFILES_DIR"] {
        if let Some(base) = std::env::var_os(variable).map(PathBuf::from) {
            candidates.push(base.clone());
            if let Some(workspace) = std::env::var_os("TEST_WORKSPACE") {
                candidates.push(base.join(workspace));
            }
            candidates.push(base.join("_main"));
        }
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir);
    }
    for candidate in candidates {
        let mut path = candidate;
        if path.is_file() {
            path.pop();
        }
        loop {
            if path.join("Cargo.toml").is_file()
                && path.join("BUILD.bazel").is_file()
                && path.join("flake.nix").is_file()
            {
                return path;
            }
            if !path.pop() {
                break;
            }
        }
    }
    panic!("repository root with Cargo.toml, BUILD.bazel, and flake.nix is not discoverable")
}

fn read_repo_file(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).expect("read repo file")
}

fn release_build_block(workflow: &str) -> &str {
    let start = workflow
        .find("      - name: Build release binaries\n")
        .expect("release build step");
    let remainder = &workflow[start..];
    let end = remainder
        .find("      - uses: actions/upload-artifact")
        .expect("release artifact upload step");
    &remainder[..end]
}

fn release_publication_violations(workflow: &str) -> Vec<String> {
    let Some(start) = workflow.find("  release:\n") else {
        return vec!["release job is missing".to_owned()];
    };
    let block = &workflow[start..];
    let mut violations = Vec::new();
    for required in [
        "contents: write",
        "defaults: { run: { shell: bash } }",
        "release-notes.md",
        "release_body_file",
        ".body",
        "cmp -s",
        "asset_download_dir",
        "mktemp -d",
        "application/octet-stream",
        "remote_asset_url",
        "remote_asset_id",
        "remote_download",
        "remote_download_digest",
        "sha256sum \"$remote_download\"",
        "[ \"$remote_download_digest\" = \"$expected_digest\" ] || {",
        "rm -rf \"$asset_download_dir\"",
        "no provable bytes",
        "conflicting bytes",
    ] {
        if !block.contains(required) {
            violations.push(format!(
                "release publication is missing the guarded path `{required}`"
            ));
        }
    }
    if block.contains("tests/tools/ci-shell") {
        violations.push(
            "contents:write release publication must not invoke the repository CI shell".to_owned(),
        );
    }
    if block.contains("uses: ./") {
        violations
            .push("contents:write release publication must not invoke a local action".to_owned());
    }
    if block.contains("bash tests/") || block.contains("sh tests/") {
        violations.push("contents:write token steps must not invoke repository scripts".to_owned());
    }
    violations
}

fn release_workspace_violations(workflow: &str) -> Vec<String> {
    let build = release_build_block(workflow);
    let normalized = build.replace("\\\n", " ");
    let mut violations = Vec::new();
    if normalized
        .matches("rustup run \"$PINNED\" cargo build")
        .count()
        != RELEASE_BINARY_SELECTORS.len()
    {
        violations.push(
            "release build must use the pinned cargo command for all six binaries".to_owned(),
        );
    }
    for (package, binary, manifest) in RELEASE_BINARY_SELECTORS {
        let selector = format!("--package {package} --bin {binary}");
        if normalized.matches(&selector).count() != 1 {
            violations.push(format!("release selector is not unique: {selector}"));
        }
        let command = normalized
            .split("rustup run \"$PINNED\" cargo build")
            .find(|command| command.contains(&selector));
        if !command.is_some_and(|command| command.contains(&format!("--manifest-path {manifest}")))
        {
            violations.push(format!(
                "release selector has no governed manifest path: {selector}"
            ));
        }
    }
    if !normalized.contains("--locked") {
        violations.push("release build must keep Cargo locked".to_owned());
    }
    if normalized.contains("--workspace")
        || normalized.contains("--all-features")
        || normalized.contains("--features")
    {
        violations.push("release build must not broaden the governed package scope".to_owned());
    }
    violations
}

fn git_tracked_files() -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["ls-files", "-z"])
        .output()
        .expect("run git ls-files");
    if output.status.success() {
        return output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf8(entry.to_vec()).expect("tracked paths are UTF-8"))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }

    let root = repo_root();
    let mut files = BTreeSet::new();
    collect_runfiles(&root, &root, &mut files);
    files.into_iter().collect()
}

fn collect_runfiles(root: &Path, path: &Path, files: &mut BTreeSet<String>) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        if let Ok(relative) = path.strip_prefix(root) {
            files.insert(relative.to_string_lossy().replace('\\', "/"));
        }
        return;
    }
    if !metadata.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if child
            .file_name()
            .is_some_and(|name| name == ".git" || name == "target")
        {
            continue;
        }
        collect_runfiles(root, &child, files);
    }
}

#[test]
fn workspace_names_contract_crate_by_role() {
    let workspace = read_repo_file("Cargo.toml");
    assert!(
        workspace.contains(&format!("\"packages/{CONTRACTS_CRATE}\"")),
        "main workspace must include the contract/DTO crate by role"
    );
    assert!(
        !workspace.contains(&format!("\"{}{}\"", "d2b", "-ipc")),
        "main workspace must not reintroduce the old transport-shaped contract crate name"
    );

    let manifest = read_repo_file("packages/d2b-contracts/Cargo.toml");
    assert!(
        manifest.contains(&format!("name = \"{CONTRACTS_CRATE}\"")),
        "contract crate manifest must use the role-based package name"
    );
}

#[test]
fn stale_ipc_crate_name_is_absent_from_current_sources() {
    let old_hyphen = format!("{}{}", "d2b", "-ipc");
    let old_underscore = format!("{}{}", "d2b", "_ipc");
    let self_path = "packages/xtask/tests/policy_workspace.rs";
    let mut violations = Vec::new();
    for rel in git_tracked_files() {
        if rel == self_path {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(repo_root().join(&rel)) else {
            continue;
        };
        if content.contains(&old_hyphen) || content.contains(&old_underscore) {
            violations.push(rel);
        }
    }
    assert!(
        violations.is_empty(),
        "stale contract crate references remain:\n{}",
        violations.join("\n")
    );
}

fn assert_fast_dev_profile(manifest: &str, workspace: &str) {
    for required in [
        "[profile.dev]",
        "[profile.dev.package.\"*\"]",
        "[profile.test]",
        "[profile.test.package.\"*\"]",
        "[profile.debugging]",
        "inherits = \"dev\"",
        "debug = \"line-tables-only\"",
        "debug = false",
        "debug = 2",
    ] {
        assert!(
            manifest.contains(required),
            "{workspace} must keep the measured fast-development profile and full-debug escape hatch: missing {required}"
        );
    }
}

#[test]
fn every_tested_workspace_uses_fast_debug_profiles() {
    assert_fast_dev_profile(&read_repo_file("Cargo.toml"), "main workspace");
    for workspace in EXCLUDED_WORKSPACES {
        assert_fast_dev_profile(
            &read_repo_file(&format!("packages/{workspace}/Cargo.toml")),
            workspace,
        );
    }
}

fn discovered_independent_workspace_roots() -> BTreeSet<String> {
    git_tracked_files()
        .into_iter()
        .filter(|rel| {
            rel.starts_with("packages/") && rel.ends_with("/Cargo.toml") && rel != "Cargo.toml"
        })
        .filter_map(|rel| {
            let content = std::fs::read_to_string(repo_root().join(&rel)).ok()?;
            if !content.lines().any(|line| line.trim() == "[workspace]") {
                return None;
            }
            Some(rel.trim_end_matches("/Cargo.toml").to_owned())
        })
        .collect()
}

fn independent_workspace_violations(manifests: &[(String, String)]) -> Vec<String> {
    let allowed = INDEPENDENT_WORKSPACE_ROOTS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    manifests
        .iter()
        .filter_map(|(rel, content)| {
            if !content.lines().any(|line| line.trim() == "[workspace]") {
                return None;
            }
            let root = rel.trim_end_matches("/Cargo.toml");
            (root != "packages" && !allowed.contains(root))
                .then(|| format!("unknown independent workspace root: {root}"))
        })
        .collect()
}

fn heavy_gate_build_violations(makefile: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let Some(start) = makefile.find("heavy-gate-build:\n") else {
        return vec!["heavy-gate-build has no Make rule".to_owned()];
    };
    let block = makefile[start..]
        .split("\n\n## heavy-gate-provision")
        .next()
        .unwrap_or(&makefile[start..]);
    for required in [
        "--manifest-path Cargo.toml",
        "--locked",
        "-p xtask",
        "--bin xtask",
    ] {
        if !block.contains(required) {
            violations.push(format!(
                "heavy-gate-build must use the governed xtask selector `{required}`"
            ));
        }
    }
    if !block.contains("CARGO_TARGET_DIR=") {
        violations
            .push("heavy-gate-build must bind the execution-only target directory".to_owned());
    }
    if block.contains("cd packages") {
        violations.push("heavy-gate-build must execute Cargo from the repository root".to_owned());
    }
    violations
}

#[test]
fn independent_workspace_roots_are_closed_and_explicit() {
    let expected = INDEPENDENT_WORKSPACE_ROOTS
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let discovered = discovered_independent_workspace_roots();
    assert_eq!(
        discovered, expected,
        "every package workspace root must be an explicit supported exception"
    );
}

#[test]
fn independent_workspace_policy_rejects_unknown_top_level_and_nested_roots() {
    let good = INDEPENDENT_WORKSPACE_ROOTS
        .iter()
        .map(|root| (format!("{root}/Cargo.toml"), "[workspace]\n".to_owned()))
        .collect::<Vec<_>>();
    assert!(
        independent_workspace_violations(&good).is_empty(),
        "the supported independent workspace fixture must pass"
    );

    let mut mutated = good;
    mutated.push((
        "packages/unknown-workspace/Cargo.toml".to_owned(),
        "[workspace]\n".to_owned(),
    ));
    mutated.push((
        "packages/d2b-core/unknown-nested-workspace/Cargo.toml".to_owned(),
        "[workspace]\n".to_owned(),
    ));
    let violations = independent_workspace_violations(&mutated);
    assert_eq!(violations.len(), 2);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("packages/unknown-workspace")),
        "unknown top-level workspace must be rejected: {violations:?}"
    );
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("packages/d2b-core/unknown-nested-workspace")),
        "unknown nested workspace must be rejected: {violations:?}"
    );
}

#[test]
fn heavy_gate_build_uses_the_locked_xtask_binary_manifest() {
    let makefile = read_repo_file("Makefile");
    let violations = heavy_gate_build_violations(&makefile);
    assert!(
        violations.is_empty(),
        "heavy-gate-build must select the locked xtask binary explicitly:\n{}",
        violations.join("\n")
    );

    for required in [
        "CARGO_TARGET_DIR=",
        "--manifest-path Cargo.toml",
        "--locked",
        "-p xtask",
        "--bin xtask",
    ] {
        let start = makefile
            .find("heavy-gate-build:\n")
            .expect("heavy-gate-build rule");
        let mutated = format!(
            "{}{}",
            &makefile[..start],
            makefile[start..].replacen(required, "", 1)
        );
        let violations = heavy_gate_build_violations(&mutated);
        assert!(
            !violations.is_empty(),
            "removing {required} must invalidate heavy-gate-build"
        );
    }
}

#[test]
fn release_workflow_keeps_exact_locked_workspace_selectors() {
    let workflow = read_repo_file(RELEASE_WORKFLOW);
    assert!(
        release_workspace_violations(&workflow).is_empty(),
        "release workflow workspace selectors drifted:\n{}",
        release_workspace_violations(&workflow).join("\n")
    );

    let mutations = [
        (
            workflow.replace(
                "rustup run \"$PINNED\" cargo build --release --locked",
                "cargo build --release",
            ),
            "pinned cargo invocation",
        ),
        (
            workflow.replace(
                "--package d2bd --bin d2bd",
                "--package d2bd --bin d2bd --features extra",
            ),
            "ordinary package selector",
        ),
        (
            workflow.replace(
                "--manifest-path Cargo.toml \\\n            --package d2b-priv-broker",
                "--manifest-path packages/d2b-priv-broker/Cargo.toml \\\n            --package d2b-priv-broker",
            ),
            "broker manifest selector",
        ),
        (
            workflow.replace(
                "--locked --manifest-path Cargo.toml",
                "--workspace --manifest-path Cargo.toml",
            ),
            "workspace broadening",
        ),
    ];
    for (mutated, label) in mutations {
        assert!(
            !release_workspace_violations(&mutated).is_empty(),
            "release workspace policy missed {label} mutation"
        );
    }
}

#[test]
fn release_publication_verifies_body_bytes_and_runner_shell_isolation() {
    let workflow = read_repo_file(RELEASE_WORKFLOW);
    let violations = release_publication_violations(&workflow);
    assert!(
        violations.is_empty(),
        "release publication policy drifted:\n{}",
        violations.join("\n")
    );

    let repository_shell_mutation = workflow.replacen(
        "shell: sh tests/tools/ci-shell {0}",
        "shell: sh tests/tools/ci-shell-token-observer {0}",
        1,
    );
    assert!(
        release_publication_violations(&repository_shell_mutation).is_empty(),
        "changing the repository shell must not affect privileged publication"
    );

    let mutations = [
        (
            workflow.replacen(
                "defaults: { run: { shell: bash } }",
                "defaults: { run: { shell: sh tests/tools/ci-shell {0} } }",
                1,
            ),
            "repository shell",
        ),
        (
            workflow.replacen(
                "cmp -s \"$release_body_file\" release-notes.md || reject_release_conflict",
                "true",
                1,
            ),
            "release body comparison",
        ),
        (
            workflow.replacen(
                "gh api \\\n                      --header 'Accept: application/octet-stream'",
                "true",
                1,
            ),
            "absent digest download",
        ),
        (
            workflow.replacen(
                "[ \"$remote_download_digest\" = \"$expected_digest\" ] || {",
                "true || {",
                1,
            ),
            "same-size byte comparison",
        ),
        (
            workflow.replacen(
                "      - uses: actions/download-artifact@",
                "      - uses: ./tests/tools/token-observer\n      - uses: actions/download-artifact@",
                1,
            ),
            "local action",
        ),
    ];
    for (mutated, label) in mutations {
        let violations = release_publication_violations(&mutated);
        assert!(
            !violations.is_empty(),
            "release publication policy missed {label} mutation"
        );
    }
}
