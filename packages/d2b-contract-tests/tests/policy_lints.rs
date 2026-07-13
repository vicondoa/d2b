//! Policy/source/doc cross-reference lints (the "H-group"), migrated from the
//! `tests/*-eval.sh` bash gates. Each test reads the real repo files (via the
//! `d2b_contract_tests::read_repo_file` helper) and asserts a structural or
//! documentation invariant. This crate runs only from
//! `tests/tools/rust-workspace-checks.sh` against the real checkout (it is excluded
//! from the hermetic Nix sandbox workspace build), so repo-file access is
//! sound.

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;

/// Assert `haystack` contains a line matching `pattern` (multi-line, `^`/`$`
/// anchor lines), with a descriptive failure message.
fn assert_line_matches(haystack: &str, pattern: &str, ctx: &str) {
    let re = Regex::new(pattern).expect("valid regex");
    assert!(re.is_match(haystack), "{ctx}: no line matched /{pattern}/");
}

// Migrated from tests/daemon-experimental-warning-eval.sh.
#[test]
fn daemon_experimental_option_documents_default_with_migration() {
    let opts = read_repo_file("nixos-modules/options-daemon.nix");
    assert!(
        opts.contains("consumers should leave it at its default"),
        "options-daemon.nix must document daemonExperimental as a leave-at-default option"
    );
    let guide = read_repo_file("docs/how-to/migrate-d2b-v1-0-to-v1-1.md");
    assert!(
        guide.contains("Remove `d2b.daemonExperimental.enable`"),
        "the v1.0->v1.1 migration guide must instruct removing d2b.daemonExperimental.enable"
    );
}

// Migrated from tests/v1.1-kernel-floor-eval.sh.
#[test]
fn v1_1_kernel_floor_declared_in_adr_and_migration_guide() {
    let adr = read_repo_file("docs/adr/0008-supported-platforms-and-rejected-targets.md");
    let adr_floor = Regex::new(r"(>=\s*6\.9|≥\s*6\.9|6\.9\+|kernel-floor uplift)").unwrap();
    assert!(
        adr_floor.is_match(&adr),
        "ADR 0008 must declare the v1.1 >=6.9 kernel floor"
    );
    let guide = read_repo_file("docs/how-to/migrate-d2b-v1-0-to-v1-1.md");
    let guide_floor = Regex::new(r"kernel\s*≥?\s*6\.9|kernel\s*>=\s*6\.9").unwrap();
    assert!(
        guide_floor.is_match(&guide),
        "the v1.0->v1.1 migration guide must mention the v1.1 kernel-floor prerequisite"
    );
}

// Migrated from tests/adr-0015-presence-eval.sh.
#[test]
fn adr_0015_present_with_header_and_cross_references() {
    for f in [
        "docs/adr/0015-daemon-only-clean-break.md",
        "AGENTS.md",
        "docs/adr/README.md",
    ] {
        assert!(repo_path_exists(f), "missing {f}");
    }
    let adr = read_repo_file("docs/adr/0015-daemon-only-clean-break.md");
    assert_line_matches(&adr, r"(?m)^# 0015\. ", "ADR 0015 title");
    assert_line_matches(&adr, r"(?m)^- Status: Accepted$", "ADR 0015 Status header");
    assert_line_matches(&adr, r"(?m)^- Wave: P6$", "ADR 0015 Wave header");
    assert_line_matches(
        &adr,
        r"(?m)^- Date: [0-9]{4}-[0-9]{2}-[0-9]{2}$",
        "ADR 0015 ISO Date header",
    );
    for section in [
        r"(?m)^## Context$",
        r"(?m)^## Decision$",
        r"(?m)^## Consequences$",
    ] {
        assert_line_matches(&adr, section, "ADR 0015 required section");
    }

    let agents = read_repo_file("AGENTS.md");
    assert!(
        agents.contains("0015-daemon-only-clean-break.md"),
        "AGENTS.md must cross-reference 0015-daemon-only-clean-break.md"
    );
    let adr_index = read_repo_file("docs/adr/README.md");
    assert!(
        adr_index.contains("0015-daemon-only-clean-break.md"),
        "docs/adr/README.md index must list 0015-daemon-only-clean-break.md"
    );
}

#[test]
fn delivery_tool_sources_and_toolchains_are_exactly_pinned() {
    let tools = read_repo_file("pkgs/delivery-tools.nix");
    for pin in [
        r#"ghVersion = "2.92.0";"#,
        r#"ghStackVersion = "0.0.7";"#,
        r#"cargoUdepsVersion = "0.1.61";"#,
        r#"cargoUdepsNightlyDate = "2025-12-01";"#,
        r#"cargoSemverChecksVersion = "0.47.0";"#,
        r#"rustStableVersion = "1.94.1";"#,
        r#"hash = "sha256-mD76Ef2b1loiyd807s9zuV0OD9tmRTJLLKT3WCyssug=";"#,
        r#"vendorHash = "sha256-Qs46cUUQjdF/pU5TgSAkQ583JpVrFt22kg6g6TDCpG4=";"#,
        r#"hash = "sha256-yT/EJWGGhQapbU1o1Gus1Vk5cAhso5ALTBecB3BH46g=";"#,
        r#"cargoHash = "sha256-DGfAsBucFRFJkjmJkpTpNfQO79jaNa5NezXKf7hYYeM=";"#,
        r#"hash = "sha256-1D6WFsiMOl/bJr0J+mmvLlgnRSKN6rPhDSnDsdLTC9E=";"#,
        r#"cargoHash = "sha256-YbtYIHj899eJSrp5n5jODgTkL9L26EnruzECwBrBF00=";"#,
    ] {
        assert!(
            tools.contains(pin),
            "delivery tooling is missing exact pin {pin}"
        );
    }
    assert!(
        !tools.contains("fakeHash") && !tools.contains("fakeSha256"),
        "delivery tooling must not contain placeholder hashes"
    );
    assert!(
        !tools.contains("curl") && !tools.contains("rustup"),
        "delivery tools must not download toolchains or binaries at runtime"
    );

    let flake = read_repo_file("flake.nix");
    assert!(
        flake.contains(r#"inputs.nixpkgs.follows = "nixpkgs";"#)
            && flake.contains("rust-overlay.overlays.default"),
        "the locked rust-overlay input must follow nixpkgs and remain scoped to delivery tooling"
    );
    assert!(
        flake.contains("devShells = forAllSystems")
            && flake.contains("cargo-udeps-nightly = deliveryTools.cargoUdepsNightly;")
            && flake.contains("cargo-semver-checks = deliveryTools.cargoSemverChecks;"),
        "supported systems must expose the pinned delivery tools"
    );
    assert!(
        flake.contains("overlays.default = _final: _prev: { };"),
        "developer tooling must not expand the public overlay"
    );
    let lock = read_repo_file("flake.lock");
    for pin in [
        r#""rev": "e013376c32a8fcf07ddb6ec71739552bc118b7bd""#,
        r#""narHash": "sha256-DsSIQSRMrLOz40LrGZ03sp2RlJ9sz3wKpd8XPTOzXnw=""#,
    ] {
        assert!(
            lock.contains(pin),
            "rust-overlay lock is missing exact pin {pin}"
        );
    }

    let delivery_command = read_repo_file("packages/xtask/src/delivery/command.rs");
    assert!(
        delivery_command.contains(r#""GET".to_owned()"#)
            && delivery_command.contains("no fallback stack mutation is permitted"),
        "xtask must encode read-only private-preview inspection and fail-closed fallback"
    );
    for mutation in [
        r#""POST".to_owned()"#,
        r#""PUT".to_owned()"#,
        r#""PATCH".to_owned()"#,
        r#""DELETE".to_owned()"#,
    ] {
        assert!(
            !delivery_command.contains(mutation),
            "xtask must not implement a GitHub stack mutation with {mutation}"
        );
    }
}

#[test]
fn non_generated_pr_workflows_cover_stacked_bases_safely() {
    for path in [
        ".github/workflows/pr-eval-shell-tests.yml",
        ".github/workflows/eval-with-entra-id.yml",
    ] {
        let workflow = read_repo_file(path);
        assert!(
            workflow.contains("  pull_request: {}"),
            "{path} must run for pull requests targeting feature branches"
        );
        assert!(
            !workflow.contains("pull_request_target"),
            "{path} must not execute untrusted code through pull_request_target"
        );
        assert!(
            workflow.contains("permissions:\n  contents: read"),
            "{path} must retain read-only workflow permissions"
        );
        assert!(
            workflow.contains("GITHUB_STEP_SUMMARY"),
            "{path} must report the checked head and outcomes"
        );
    }

    let reference = read_repo_file("docs/reference/delivery-tooling.md");
    assert!(
        reference.contains("Official `gh-stack` is the only stack mutator")
            && reference.contains("There is no fallback stack mutation")
            && reference.contains("never add them to the reviewed tree"),
        "delivery reference must fail closed and keep evidence external"
    );
}
