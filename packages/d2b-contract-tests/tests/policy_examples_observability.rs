//! Source-policy lints migrated from
//! `tests/examples-with-observability-eval.sh`.
//!
//! The legacy bash gate mixed three layers. The realized `nix flake check`
//! remains in the shell gate, resolved-config value assertions live in
//! `tests/unit/nix/cases/examples-with-observability.nix`, and these tests
//! keep the file-presence / source-grep assertions in the Rust policy layer.

use std::path::Path;

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use regex::Regex;

/// Whether any single line of `content` matches `pattern`, matching the legacy
/// gate's `grep -E` source checks without allowing `\s*` to span lines.
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

fn assert_repo_file(rel: &str) {
    let path = repo_root().join(Path::new(rel));
    assert!(
        repo_path_exists(rel),
        "examples-with-observability-eval: missing: {rel}"
    );
    assert!(
        path.is_file(),
        "examples-with-observability-eval: expected repo path to be a file: {rel}"
    );
}

#[test]
fn with_observability_example_layout_files_exist() {
    for rel in [
        "examples/with-observability/flake.nix",
        "examples/with-observability/configuration.nix",
        "examples/with-observability/README.md",
    ] {
        assert_repo_file(rel);
    }
}

#[test]
fn with_observability_flake_imports_configuration() {
    let flake_rel = "examples/with-observability/flake.nix";
    let flake = read_repo_file(flake_rel);
    assert!(
        any_line_matches(&flake, r"\./configuration\.nix"),
        "examples-with-observability-eval: flake.nix does not import ./configuration.nix"
    );
}

#[test]
fn with_observability_configuration_sets_operator_toggles() {
    let config_rel = "examples/with-observability/configuration.nix";
    let config = read_repo_file(config_rel);

    for (pattern, label) in [
        (
            r"d2b\.observability[[:space:]]*=|d2b\.observability\.enable[[:space:]]*=[[:space:]]*true",
            "d2b.observability.enable = true",
        ),
        (
            r"d2b\.envs\.work[[:space:]]*=",
            "workload env d2b.envs.work",
        ),
        (
            r"d2b\.vms\.work-app[[:space:]]*=",
            "workload VM d2b.vms.work-app",
        ),
        (
            r"observability\.enable[[:space:]]*=[[:space:]]*true",
            "per-VM observability.enable = true on work-app",
        ),
    ] {
        assert!(
            any_line_matches(&config, pattern),
            "examples-with-observability-eval: configuration.nix missing {label}"
        );
    }
}
