#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::PathBuf;

const ALLOWLISTED_WORKFLOWS: &[&str] = &[
    ".github/workflows/eval-with-entra-id.yml",
    ".github/workflows/pr-eval-shell-tests.yml",
    ".github/workflows/release-host-binaries.yml",
];

const V3_PR_GATE_WORKFLOWS: &[&str] = &[
    ".github/workflows/eval-with-entra-id.yml",
    ".github/workflows/pr-eval-shell-tests.yml",
    ".github/workflows/pr-l1-static-fast.yml",
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
    "test-rust-main",
    "test-rust-broker",
    "test-rust-guest-shell-runner",
    "test-rust-no-bash-ast",
    "test-rust-schema",
    "test-rust-inventory",
    "test-rust-supply-chain",
    "test-proofs",
    "test-drift",
    "test-flake",
    "test-nix-unit",
    "test-policy",
    "bazel-check",
    "test-integration",
    "test-host-integration",
    "test-hardware",
    "perf",
    "check-inventory",
    "ledger-regen",
];

const RETIRED_BAZEL_AUTHORITY_PATHS: &[&str] = &[
    "docs/adr/0052-bazel-rust-build-and-test.md",
    "specs/003-adr052-bazel-rust",
    "changelog.d/adr052-bazel-rust-testing.md",
    "changelog.d/adr0054-broker-hub.md",
    "changelog.d/spec003-adr0054-amend.md",
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

fn workflow_files() -> Vec<String> {
    let root = repo_root();
    let workflow_dir = root.join(".github/workflows");
    let mut files = std::fs::read_dir(&workflow_dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", workflow_dir.display()))
        .map(|entry| {
            let entry = entry.expect("read workflow entry");
            entry.path()
        })
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
        })
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
            if word == "make"
                && let Some(target) = words.next()
            {
                return approved.contains(target);
            }
        }
        false
    })
}

fn contains_bazel_facade_invocation(content: &str) -> bool {
    let mut lines = Vec::new();
    let mut continued_line = String::new();
    for line in content.lines() {
        continued_line.push_str(line.trim_end_matches('\\'));
        continued_line.push(' ');
        if !line.trim_end().ends_with('\\') {
            lines.push(std::mem::take(&mut continued_line));
        }
    }
    if !continued_line.is_empty() {
        lines.push(continued_line);
    }

    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        let trimmed = line.trim_start();
        let run_value = trimmed
            .strip_prefix("- ")
            .unwrap_or(trimmed)
            .strip_prefix("run:")
            .map(str::trim_start);
        if let Some(value) = run_value {
            if value.starts_with('|') || value.starts_with('>') {
                let key_indent = line.len() - trimmed.len();
                let mut block = String::new();
                index += 1;
                while index < lines.len() {
                    let candidate = &lines[index];
                    let candidate_indent = candidate.len() - candidate.trim_start().len();
                    if !candidate.trim().is_empty() && candidate_indent <= key_indent {
                        break;
                    }
                    block.push_str(candidate.trim_end_matches('\\'));
                    block.push(' ');
                    index += 1;
                }
                if shell_command_contains_bazel_facade(&block) {
                    return true;
                }
                continue;
            }
            if shell_command_contains_bazel_facade(value) {
                return true;
            }
        }
        if shell_command_contains_bazel_facade(line) {
            return true;
        }
        index += 1;
    }
    false
}

fn shell_command_contains_bazel_facade(command: &str) -> bool {
    let words = command
        .split(|character: char| {
            character.is_whitespace() || matches!(character, ';' | '&' | '|' | '\'' | '"')
        })
        .filter(|word| !word.is_empty());
    let mut has_make = false;
    let mut has_target = false;
    for word in words {
        has_make |= word == "make" || word == "$(MAKE)" || word.ends_with("/make");
        has_target |= word == "bazel-check";
        if word.ends_with("tests/tools/bazel-check") {
            return true;
        }
    }
    has_make && has_target
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
fn optional_bazel_facade_stays_out_of_ci_schedulers() {
    let mut violations = Vec::new();
    for rel in workflow_files() {
        let content = read_repo_file(&rel);
        if content.contains("tests/tools/bazel-check") {
            violations.push(rel);
        }
    }
    assert!(
        violations.is_empty(),
        "CI must call Bazel through make bazel-check, not the facade script:\n{}",
        violations.join("\n")
    );

    let layer1_manifest = read_repo_file("tests/layer1-jobs.json");
    assert!(
        layer1_manifest.contains("\"ciJobId\": \"bazel-check\""),
        "CI must schedule a local Bazel aggregate"
    );
    assert!(
        layer1_manifest.contains("\"D2B_BAZEL_PROFILE\": \"local\""),
        "CI Bazel must stay on the local profile"
    );
    assert!(
        !layer1_manifest.contains("remote.buildbuddy.io")
            && !layer1_manifest.contains("d2b.buildbuddy.io"),
        "CI must not pin a BuildBuddy endpoint"
    );
}

#[test]
fn bazel_facade_policy_detects_make_variants() {
    for command in [
        "make bazel-check",
        "make  bazel-check",
        "make --no-print-directory bazel-check",
        "make \\\n  bazel-check",
        "tests/tools/bazel-check --profile local",
        "run: \"make bazel-check\"",
        "run: >-\n  make\n  bazel-check",
        "run: |\n  tests/tools/bazel-check --profile local",
    ] {
        assert!(
            contains_bazel_facade_invocation(command),
            "Bazel facade invocation was not detected: {command:?}"
        );
    }
    assert!(!contains_bazel_facade_invocation("make check-tier0"));
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

#[test]
fn obsolete_bazel_authority_is_deleted() {
    let root = repo_root();
    let present = RETIRED_BAZEL_AUTHORITY_PATHS
        .iter()
        .filter(|relative| root.join(relative).exists())
        .copied()
        .collect::<Vec<_>>();
    assert!(
        present.is_empty(),
        "obsolete Bazel authority paths must be deleted: {}",
        present.join(", ")
    );

    let adr_index = read_repo_file("docs/adr/README.md");
    assert!(
        !adr_index.contains("0052-bazel-rust-build-and-test.md"),
        "ADR index must not list deleted ADR 0052"
    );
    let makefile = read_repo_file("Makefile");
    assert!(
        !makefile.contains("test-bazel-rust"),
        "obsolete test-bazel-rust aliases must not return"
    );
}

#[test]
fn v3_pr_gates_are_enabled() {
    for rel in V3_PR_GATE_WORKFLOWS {
        let content = read_repo_file(rel);
        assert!(
            content.contains("pull_request:\n    branches: [main, v3]"),
            "{rel} must run for pull requests targeting main and v3"
        );
        assert!(
            content.contains("push:\n    branches: [main, v3]"),
            "{rel} must run for pushes to main and v3"
        );
    }
}
