#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::process::Command;

use nixling_contract_tests::repo_root;
use regex::Regex;

const ALLOWLISTED_WORKFLOWS: &[&str] = &[
    ".github/workflows/eval-with-entra-id.yml",
    ".github/workflows/pr-eval-shell-tests.yml",
    ".github/workflows/release-host-binaries.yml",
];

fn git_tracked_files(pathspecs: &[&str]) -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["ls-files", "-z", "--"])
        .args(pathspecs)
        .output()
        .expect("run `git ls-files -z`");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8(entry.to_vec()).expect("tracked paths are UTF-8"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn read_repo_file(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).expect("read repo file")
}

fn make_target_regex() -> Regex {
    Regex::new(concat!(
        r"(^|[[:space:]])make[[:space:]]+(",
        r"check|check-ci|check-all|check-fast|check-tier0|",
        r"test|test-unit|test-lint|test-rust|test-proofs|test-drift|",
        r"test-flake|test-nix-unit|test-policy|test-integration|",
        r"test-host-integration|test-hardware|perf|check-inventory|ledger-regen",
        r")([[:space:]]|$)",
    ))
    .expect("valid make-target regex")
}

#[test]
fn github_workflows_use_make_targets_or_explicit_allowlist() {
    let workflows = git_tracked_files(&[".github/workflows/*.yml"]);
    assert!(
        !workflows.is_empty(),
        "ci-uses-make: no .github/workflows/*.yml files found"
    );
    let make_target = make_target_regex();
    let allowlisted = ALLOWLISTED_WORKFLOWS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut violations = Vec::new();
    for rel in workflows {
        let content = read_repo_file(&rel);
        if make_target.is_match(&content) || allowlisted.contains(rel.as_str()) {
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
