#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const ALLOWLISTED_WORKFLOWS: &[&str] = &[
    ".github/workflows/pr.yml",
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
            ".github/workflows/pr.yml",
            ".github/workflows/eval-with-entra-id.yml",
            ".github/workflows/pr-eval-shell-tests.yml",
            ".github/workflows/release-host-binaries.yml",
        ],
        "workflow make-target exceptions must stay reviewed and bounded"
    );
}

#[test]
fn main_controlled_buildbuddy_workflows_preserve_trust_contract() {
    let build = read_repo_file(".github/workflows/build.yaml");
    let pr = read_repo_file(".github/workflows/pr.yml");

    assert!(
        pr.contains("pull_request_target:\n    branches: [main, v3]"),
        "PR workflow must cover both protected targets"
    );
    assert!(
        !pr.contains("\n  pull_request:\n"),
        "PR workflow must not run untrusted pull_request controls"
    );
    assert!(
        pr.contains("uses: vicondoa/d2b/.github/workflows/build.yaml@main")
            && !pr.contains("uses: vicondoa/d2b/.github/workflows/build.yaml@refs/heads/main")
            && !pr.contains("D2B_BUILDBUDDY_API_KEY")
            && !pr.contains("secrets: inherit"),
        "PR workflow must call the main-owned reusable build at main without PR secret access"
    );

    assert!(
        build.contains("name: build") && build.contains("workflow_call:"),
        "build.yaml must be the reusable build workflow"
    );
    assert!(
        build.contains("push:\n    branches: [main]"),
        "trusted cache seeding must be limited to main pushes"
    );
    assert!(
        build.contains("permissions:\n  contents: read"),
        "build workflow must retain least-privilege contents access"
    );
    assert!(
        build.contains("github.workflow_sha")
            && build.contains("github.job_workflow_ref")
            && build.contains("github.job_workflow_sha")
            && build.contains("trusted_sha")
            && build.contains("base_sha")
            && build.contains("head_sha")
            && build.contains("merge_sha"),
        "build workflow must bind trusted and tested immutable OIDs"
    );
    assert!(
        build.matches("persist-credentials: false").count() >= 4,
        "trusted and source checkouts must not persist credentials"
    );
    assert!(
        build.contains("D2B_BAZEL_CREDENTIAL_FD")
            && build.contains("D2B_BUILDBUDDY_API_KEY")
            && build.contains("env -u D2B_BUILDBUDDY_API_KEY"),
        "BuildBuddy authentication must use the trusted descriptor bootstrap"
    );
    assert!(
        build.contains("grpcs://d2b.buildbuddy.io")
            && build.contains("d2b/pr/")
            && build.contains("--credential_helper="),
        "remote execution must use the fixed brokered BuildBuddy endpoint and PR namespace"
    );
    assert!(
        !build.contains("--remote_header=") && !build.contains("--bes_header="),
        "API keys must not be passed as direct Bazel headers"
    );
    assert!(
        build.contains("--test_tag_filters=-local,-no-remote-exec,-manual,-exclusive,-gpu,-kvm"),
        "remote execution must exclude local-only and hardware-tagged actions"
    );
    for target in [
        "//bazel/checks:test-rust-main",
        "//bazel/checks:test-rust-broker",
        "//bazel/checks:test-rust-guest-shell-runner",
        "//bazel/checks:test-policy",
    ] {
        assert!(
            build.contains(target),
            "remote target set must retain canonical target {target}"
        );
    }
    assert!(
        build.contains("make check")
            && build.contains("D2B_BAZEL_PROFILE: local")
            && build.contains("D2B_BAZEL_UNTRUSTED: \"1\""),
        "full PR coverage must retain a credential-free local Layer-1 gate"
    );
    let rust_bootstrap = build
        .find("Prepare pinned Rust toolchain for parallel Layer-1 gates")
        .expect("local Layer-1 gate must prepare the shared Rust toolchain");
    let local_gate = build
        .find("Run credential-free local Layer-1 gate")
        .expect("local Layer-1 gate step must be present");
    assert!(
        rust_bootstrap < local_gate
            && build.contains("rustup toolchain install \"$pinned_channel\" --profile minimal")
            && build.contains("--component rustfmt --component clippy"),
        "parallel local Layer-1 jobs must share a preinstalled pinned Rust toolchain"
    );
    assert!(
        build.contains("github.event_name == 'push'")
            && build.contains("github.event_name == 'push' }}"),
        "credential-bearing remote execution must be restricted to trusted main pushes"
    );
    assert!(
        build.contains("refs/heads/$D2B_DEFAULT_BRANCH")
            && build.contains("secret = os.read(9, 4096)")
            && build.contains("9<&0"),
        "PR ref validation and credential bootstrap must match GitHub's default-branch semantics"
    );
    assert!(
        build.contains("redact") && build.contains("^warning:"),
        "remote evidence must be redacted and warning-producing builds must fail closed"
    );
    assert!(
        build.contains("if: ${{ always() }}")
            && build.contains("needs: [metadata, local, remote]")
            && build.contains("name: check"),
        "build workflow must expose one stable aggregate check"
    );
}
