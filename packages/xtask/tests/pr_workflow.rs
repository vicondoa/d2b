#![forbid(unsafe_code)]

use std::path::PathBuf;

const REQUIRED_AGGREGATE_JOBS: &[&str] = &[
    "tier0",
    "policy-tooling",
    "policy-local",
    "rust-main",
    "rust-broker",
    "rust-guest",
    "rust-local",
    "nix-eval",
    "nix-unit",
    "nix-realized",
    "nix-aarch64",
    "fixtures-proofs",
];

fn workflow() -> String {
    let relative = ".github/workflows/pr-l1-static-fast.yml";
    if let Some(root) = std::env::var_os("D2B_BAZEL_SOURCE_ROOT")
        .or_else(|| std::env::var_os("D2B_REPO_ROOT"))
    {
        let path = PathBuf::from(root).join(relative);
        return std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "tested PR workflow is not readable at {}: {error}",
                path.display()
            )
        });
    }

    let mut candidates = Vec::new();
    for variable in ["TEST_SRCDIR", "RUNFILES_DIR"] {
        if let Some(base) = std::env::var_os(variable).map(PathBuf::from) {
            if let Some(workspace) = std::env::var_os("TEST_WORKSPACE") {
                candidates.push(base.join(workspace).join(relative));
            }
            candidates.push(base.join("_main").join(relative));
        }
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join(relative));
    }
    for path in candidates {
        if let Ok(workflow) = std::fs::read_to_string(&path) {
            return workflow;
        }
    }
    panic!("PR workflow is not discoverable through Bazel runfiles")
}

fn job_block<'a>(workflow: &'a str, job: &str) -> &'a str {
    let header = format!("  {job}:\n");
    let start = workflow
        .find(&header)
        .unwrap_or_else(|| panic!("workflow job is missing: {job}"));
    let body = &workflow[start + header.len()..];
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if line.starts_with("  ") && !line.starts_with("    ") {
            return &body[..offset];
        }
        offset += line.len();
    }
    body
}

fn job_names(workflow: &str) -> Vec<&str> {
    workflow
        .lines()
        .skip_while(|line| *line != "jobs:")
        .skip(1)
        .filter_map(|line| {
            let name = line.strip_prefix("  ")?.strip_suffix(':')?;
            (!name.starts_with(' ')).then_some(name)
        })
        .collect()
}

fn needs_entries(block: &str) -> Vec<&str> {
    let value = block
        .lines()
        .find_map(|line| line.trim().strip_prefix("needs:"))
        .expect("aggregate job needs a dependency list")
        .trim()
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .expect("aggregate dependencies use an inline list");
    value.split(',').map(str::trim).collect()
}

fn assert_trusted_workflow_contract(workflow: &str) {
    assert!(
        workflow.contains("pull_request_target:\n    branches: [v3]"),
        "the credential-bearing workflow must be owned by protected v3"
    );
    assert!(
        workflow.contains("push:\n    branches: [v3]"),
        "trusted seeding must run only from protected v3"
    );
    assert!(
        !workflow.contains("\n  pull_request:\n"),
        "the secret-bearing gate must not use an untrusted pull_request workflow"
    );
    assert!(
        workflow.contains("permissions:\n  contents: read")
            && !workflow.contains("contents: write")
            && !workflow.contains("pull-requests: write"),
        "the trusted gate must use a read-only GitHub token"
    );
    assert!(
        !workflow.contains("D2B_BAZEL_UNTRUSTED"),
        "the trusted gate must not opt into the untrusted BuildBuddy boundary"
    );
    for line in workflow
        .lines()
        .filter(|line| line.trim_start().starts_with("- uses:"))
    {
        let reference = line
            .split_once('@')
            .map(|(_, reference)| reference.trim())
            .expect("pinned action reference");
        assert!(
            reference.len() == 40
                && reference
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "every workflow action must be pinned to a commit: {line}"
        );
    }
    assert!(
        workflow.contains("path: trusted") && workflow.contains("path: workspace"),
        "every gate must separate trusted bootstrap and tested source"
    );
    assert!(
        workflow.contains("shell: sh trusted/tests/tools/ci-shell {0}"),
        "workflow commands must use the trusted shell wrapper"
    );
    assert_eq!(
        workflow.matches("make -C trusted").count(),
        13,
        "every Layer-1 step must invoke its fixed public Make alias from trusted v3"
    );
    assert!(
        workflow.contains("ref: ${{ github.event.pull_request.base.sha || github.sha }}"),
        "trusted bootstrap must bind to the event base or pushed v3 commit"
    );
    assert!(
        workflow.contains(
            "ref: ${{ github.event_name == 'push' && github.sha || github.event.pull_request.merge_commit_sha }}"
        ),
        "the tested checkout must bind to the immutable merge or pushed commit"
    );
    assert!(
        workflow.contains("D2B_BAZEL_SOURCE_ROOT")
            && workflow.contains("D2B_BAZEL_TRUSTED_ROOT")
            && workflow.contains("D2B_BAZEL_TRUSTED_SHA"),
        "the facade must receive separate source and trusted roots"
    );
    assert!(
        !workflow.contains("github.event.pull_request.merge_commit_sha || github.sha"),
        "PR jobs must not substitute the default-branch SHA for a missing merge SHA"
    );
    assert!(
        workflow.contains("./trusted/tests/tools/bazel-check-bootstrap")
            && !workflow.contains("python3 ./trusted/tests/tools/bazel-check-bootstrap")
            && workflow.contains("env -u D2B_BUILDBUDDY_API_KEY")
            && workflow.contains("printf '%s' \"$D2B_BUILDBUDDY_API_KEY\"")
            && workflow.contains("D2B_BUILDBUDDY_API_KEY: ${{ secrets.D2B_BUILDBUDDY_API_KEY }}")
            && !workflow.contains("secret=\"${{ secrets.D2B_BUILDBUDDY_API_KEY }}\""),
        "the BuildBuddy secret must cross the trusted bootstrap only over stdin"
    );
    assert!(
        workflow.contains("secrets.D2B_BUILDBUDDY_API_KEY"),
        "the remote gate must use the repository BuildBuddy secret"
    );
    assert!(
        workflow.matches("persist-credentials: false").count() >= 2,
        "all checkouts must disable persisted GitHub credentials"
    );
    for line in workflow.lines().filter(|line| line.contains("make -C")) {
        assert!(
            line.contains("make -C trusted"),
            "workflow commands must use the trusted v3 Makefile: {line}"
        );
    }

    for job in [
        "policy-tooling",
        "rust-main",
        "rust-broker",
        "rust-guest",
    ] {
        let block = job_block(workflow, job);
        assert!(
            block.contains(
                "D2B_BAZEL_PROFILE: ${{ github.event_name == 'push' && 'trusted-seed' || 'remote' }}"
            ),
            "{job} must use the trusted remote/seeding BuildBuddy profile"
        );
        assert!(
            block.contains("D2B_BAZEL_REQUIRE_REMOTE: \"1\""),
            "{job} must fail closed instead of reducing to a local gate"
        );
        assert!(
            block.contains("D2B_BAZEL_TEST_TAG_FILTERS:")
                && block.contains("-local,-no-remote-exec"),
            "{job} must exclude local actions from credential-bearing CI"
        );
        assert!(
            block.contains("D2B_BUILDBUDDY_API_KEY: ${{ secrets.D2B_BUILDBUDDY_API_KEY }}")
                && block.contains("bazel-check-bootstrap"),
            "{job} must broker the credential through the trusted bootstrap"
        );
        assert!(
            block.contains(
                "if: ${{ github.event_name == 'push' || github.event.pull_request.merge_commit_sha != '' }}"
            ),
            "{job} must fail closed when the PR merge SHA is unavailable"
        );
    }
    let tier0 = job_block(workflow, "tier0");
    assert!(
        tier0.contains("D2B_BAZEL_PROFILE: local")
            && !tier0.contains("D2B_BAZEL_REQUIRE_REMOTE")
            && !tier0.contains("D2B_BUILDBUDDY_API_KEY")
            && !tier0.contains("bazel-check-bootstrap"),
        "tier0 must remain a credential-free local preflight"
    );
    let policy_local = job_block(workflow, "policy-local");
    assert!(
        policy_local.contains("D2B_BAZEL_TEST_TAG_FILTERS: \"-manual,-gpu,-kvm\"")
            && policy_local.contains("name: Local policy-only suite")
            && policy_local.contains("D2B_BAZEL_PROFILE: local")
            && policy_local.contains("D2B_BAZEL_REQUIRE_REMOTE: \"0\""),
        "policy-only local tests must be split from the credential-bearing remote step"
    );
    for job in [
        "policy-local",
        "tier0",
        "rust-local",
        "nix-eval",
        "nix-unit",
        "nix-realized",
        "nix-aarch64",
        "fixtures-proofs",
        "test-performance-budgets",
    ] {
        let block = job_block(workflow, job);
        assert!(
            block.contains(
                "if: ${{ github.event_name == 'push' || github.event.pull_request.merge_commit_sha != '' }}"
            ),
            "{job} must fail closed when the PR merge SHA is unavailable"
        );
        assert!(
            block.contains("D2B_BAZEL_PROFILE: local"),
            "{job} must remain local-only"
        );
        assert!(
            !block.contains("D2B_BUILDBUDDY_API_KEY")
                && !block.contains("bazel-check-bootstrap"),
            "{job} must not receive the BuildBuddy credential"
        );
    }
}

#[test]
fn pr_suites_start_concurrently_and_aggregate_preserves_required_failures() {
    let workflow = workflow();

    for job in job_names(&workflow)
        .into_iter()
        .filter(|job| *job != "check")
    {
        let block = job_block(&workflow, job);
        assert!(
            !block.lines().any(|line| line.trim().starts_with("needs:")),
            "{job} must not wait for another workflow job"
        );
    }

    let performance = job_block(&workflow, "test-performance-budgets");
    assert!(
        performance.contains("continue-on-error: true"),
        "performance budgets must remain advisory"
    );

    let aggregate = job_block(&workflow, "check");
    assert_eq!(needs_entries(aggregate), REQUIRED_AGGREGATE_JOBS);
    assert!(aggregate.contains("if: ${{ always() }}"));
    assert!(aggregate.contains("!contains(needs.*.result, 'failure')"));
    assert!(aggregate.contains("!contains(needs.*.result, 'cancelled')"));
    assert!(aggregate.contains("!contains(needs.*.result, 'skipped')"));
    assert!(aggregate.contains(r#"run: test "$SUITES_OK" = true"#));
    assert!(
        !needs_entries(aggregate).contains(&"test-performance-budgets"),
        "advisory performance budgets must not block the aggregate"
    );
}

#[test]
fn trusted_workflow_rejects_malicious_control_plane_edits() {
    let workflow = workflow();
    assert_trusted_workflow_contract(&workflow);

    for tampered in [
        workflow.replace(
            "pull_request_target:\n    branches: [v3]",
            "pull_request:\n    branches: [v3]",
        ),
        workflow.replace(
            "./trusted/tests/tools/bazel-check-bootstrap",
            "python3 ./workspace/tests/tools/bazel-check-bootstrap",
        ),
        workflow.replace(
            "github.event_name == 'push' && github.sha || github.event.pull_request.merge_commit_sha",
            "github.event.pull_request.merge_commit_sha || github.sha",
        ),
        workflow.replace("make -C trusted", "make"),
        workflow.replace(
            "ref: ${{ github.event.pull_request.base.sha || github.sha }}",
            "ref: main",
        ),
        workflow.replacen(
            "D2B_BAZEL_PROFILE: ${{ github.event_name == 'push' && 'trusted-seed' || 'remote' }}",
            "D2B_BAZEL_PROFILE: local",
            1,
        ),
    ] {
        assert!(
            std::panic::catch_unwind(|| assert_trusted_workflow_contract(&tampered)).is_err(),
            "malicious workflow edit was accepted"
        );
    }
}
