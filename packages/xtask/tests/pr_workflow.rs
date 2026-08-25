#![forbid(unsafe_code)]

use std::path::PathBuf;

const REQUIRED_AGGREGATE_JOBS: &[&str] = &[
    "tier0",
    "policy-tooling",
    "rust-main",
    "rust-broker",
    "rust-guest",
    "nix-eval",
    "nix-unit",
    "nix-realized",
    "nix-aarch64",
    "fixtures-proofs",
];

fn workflow() -> String {
    let relative = ".github/workflows/pr-l1-static-fast.yml";
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
