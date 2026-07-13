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
    assert_line_matches(
        &adr,
        r"(?m)^- Status: Superseded by \[ADR 0045\]\(0045-provider-and-transport-framework\.md\)$",
        "ADR 0015 Status header",
    );
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
