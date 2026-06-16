//! Source-lint policy migrated from `tests/cli-nix-consumers-eval.sh`.
//!
//! The retired bash gate scanned repo `*.nix`, `*.sh`, and `*.rs` sources for
//! live (pre-comment) consumers of the retired `nixos-modules/cli.nix` outputs.
//! `cli.nix` itself (when present) was the only emitter allowed to mention those
//! bindings; this Rust port also self-exempts this policy file.

use std::collections::BTreeSet;
use std::process::Command;

use nixling_contract_tests::repo_root;
use regex::Regex;

const CLI_NIX: &str = "nixos-modules/cli.nix";
const RETIRED_GATE: &str = "tests/cli-nix-consumers-eval.sh";
const THIS_TEST: &str = "packages/nixling-contract-tests/tests/policy_cli_consumers.rs";
const STATIC_SH: &str = "tests/static.sh";

fn read_repo_file_opt(rel: &str) -> Option<String> {
    std::fs::read_to_string(repo_root().join(rel)).ok()
}

fn git_listed_files(roots: &[&str]) -> Vec<String> {
    let root = repo_root();
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("-c")
        .arg("core.quotePath=false")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .args(roots)
        .output()
        .expect("run `git ls-files`");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut files: BTreeSet<String> = BTreeSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if !line.is_empty() {
            files.insert(line.to_string());
        }
    }
    files.into_iter().collect()
}

fn is_legacy_grep_excluded(rel: &str) -> bool {
    rel.split('/').any(|component| {
        matches!(
            component,
            ".git" | "target" | ".cli-rust-native-cache" | ".tests-tmp"
        ) || component.starts_with(".nl-smoke-cache.")
    })
}

fn source_files_with_extensions(exts: &[&str]) -> Vec<String> {
    git_listed_files(&["."])
        .into_iter()
        .filter(|rel| !is_legacy_grep_excluded(rel))
        .filter(|rel| exts.iter().any(|ext| rel.ends_with(ext)))
        .collect()
}

fn strip_live_comments(line: &str) -> &str {
    let cut = [line.find('#'), line.find("//")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(line.len());
    line[..cut].trim_end()
}

fn live_consumer_violations(pattern: &Regex, allowed_files: &[&str]) -> Vec<String> {
    let mut violations = Vec::new();
    for rel in source_files_with_extensions(&[".nix", ".sh", ".rs"]) {
        if allowed_files.contains(&rel.as_str()) {
            continue;
        }
        let Some(content) = read_repo_file_opt(&rel) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if pattern.is_match(strip_live_comments(line)) {
                violations.push(format!("{rel}:{}:{line}", idx + 1));
            }
        }
    }
    violations
}

#[test]
fn cli_nix_live_consumer_bindings_absent() {
    let base_allowed = [CLI_NIX, RETIRED_GATE, THIS_TEST];
    for (name, pattern) in [
        ("nixling.cliBin", r"nixling\.cliBin"),
        ("audioStateHelperPath", r"audioStateHelperPath"),
        ("_desktopWrappers", r"_desktopWrappers"),
    ] {
        let re = Regex::new(pattern).expect("valid consumer regex");
        let violations = live_consumer_violations(&re, &base_allowed);
        assert!(
            violations.is_empty(),
            "cli-nix-consumers-eval: live {name} consumer(s) found outside \
             {CLI_NIX} / retired gate / this policy test:\n{}",
            violations.join("\n")
        );
    }

    let store_allowed = [CLI_NIX, RETIRED_GATE, THIS_TEST, STATIC_SH];
    let mut store_violations: BTreeSet<String> = BTreeSet::new();
    for pattern in [r"nixling\.store\.package", r"nixling\.store\.generations"] {
        let re = Regex::new(pattern).expect("valid store consumer regex");
        store_violations.extend(live_consumer_violations(&re, &store_allowed));
    }
    assert!(
        store_violations.is_empty(),
        "cli-nix-consumers-eval: nixling.store.package/generations referenced \
         outside {CLI_NIX} + tests/static.sh trio lint:\n{}",
        store_violations.into_iter().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn cli_nix_imports_absent() {
    let import_payload = Regex::new(r"^[^#]*(\bimport\b|imports[[:space:]]*=)[^#]*\./cli\.nix")
        .expect("valid cli.nix import regex");
    let allowed = [CLI_NIX, RETIRED_GATE, THIS_TEST];
    let mut violations = Vec::new();

    for rel in source_files_with_extensions(&[".nix"]) {
        if allowed.contains(&rel.as_str()) {
            continue;
        }
        let Some(content) = read_repo_file_opt(&rel) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if import_payload.is_match(line) {
                violations.push(format!("{rel}:{}:{line}", idx + 1));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "cli-nix-consumers-eval: live ./cli.nix import(s) found outside \
         {CLI_NIX} / retired gate:\n{}",
        violations.join("\n")
    );
}
