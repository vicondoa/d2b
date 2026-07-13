#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const ALLOWLISTED_WORKFLOWS: &[&str] = &[
    ".github/workflows/eval-with-entra-id.yml",
    ".github/workflows/pr-eval-shell-tests.yml",
    ".github/workflows/release-host-binaries.yml",
];

const APPROVED_MAKE_TARGETS: &[&str] = &[
    "check",
    "check-ci",
    "check-all",
    "check-fast",
    "check-tier0",
    "test",
    "test-unit",
    "test-lint",
    "test-rust",
    "test-proofs",
    "test-drift",
    "test-flake",
    "test-nix-unit",
    "test-policy",
    "test-integration",
    "test-host-integration",
    "test-hardware",
    "perf",
    "check-inventory",
    "ledger-regen",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask lives under packages/xtask")
        .to_path_buf()
}

fn workflow_files() -> Vec<String> {
    let root = repo_root();
    let workflow_dir = root.join(".github/workflows");
    let mut files = std::fs::read_dir(&workflow_dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", workflow_dir.display()))
        .map(|entry| {
            let entry = entry.expect("read workflow entry");
            entry.path()
        })
        .filter(|path| path.extension().is_some_and(|ext| ext == "yml"))
        .map(|path| {
            path.strip_prefix(&root)
                .expect("workflow path under repo root")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn read_repo_file(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).expect("read repo file")
}

fn calls_approved_make_target(content: &str) -> bool {
    let approved = APPROVED_MAKE_TARGETS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    content.lines().any(|line| {
        let mut words = line.split_whitespace();
        while let Some(word) = words.next() {
            if word == "make" {
                for candidate in words.by_ref() {
                    if matches!(candidate, "--" | "-s" | "--silent" | "--no-print-directory") {
                        continue;
                    }
                    return approved.contains(candidate.trim_end_matches([')', ';']));
                }
            }
        }
        false
    })
}

#[test]
fn approved_make_detection_handles_wrapper_clears_and_option_termination() {
    assert!(calls_approved_make_target(
        r#"run: RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" make -- test-lint"#
    ));
    assert!(calls_approved_make_target(
        r#"checks=$(RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" make -s -- test-flake)"#
    ));
    assert!(!calls_approved_make_target("run: make -- --version"));
}

#[test]
fn github_workflows_use_make_targets_or_explicit_allowlist() {
    let workflows = workflow_files();
    assert!(
        !workflows.is_empty(),
        "ci-uses-make: no .github/workflows/*.yml files found"
    );
    let allowlisted = ALLOWLISTED_WORKFLOWS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut violations = Vec::new();
    for rel in workflows {
        let content = read_repo_file(&rel);
        if calls_approved_make_target(&content) || allowlisted.contains(rel.as_str()) {
            continue;
        }
        violations.push(rel);
    }
    assert!(
        violations.is_empty(),
        "workflows neither call an approved make target nor appear in the allowlist:\n{}",
        violations.join("\n")
    );
}

#[test]
fn ci_uses_make_allowlist_is_intentional_and_bounded() {
    assert_eq!(
        ALLOWLISTED_WORKFLOWS,
        &[
            ".github/workflows/eval-with-entra-id.yml",
            ".github/workflows/pr-eval-shell-tests.yml",
            ".github/workflows/release-host-binaries.yml",
        ],
        "workflow make-target exceptions must stay reviewed and bounded"
    );
}
