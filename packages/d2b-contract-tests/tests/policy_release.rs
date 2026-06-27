//! Release/version policy & doc-lint gates (the "H-group"), migrated from the
//! `tests/*-eval.sh` bash gates. Each test reads the real repo (via the
//! `d2b_contract_tests::read_repo_file` / `repo_root` helpers) and asserts a
//! release-hygiene, version-tag, or retired-surface invariant. This crate runs
//! only from `tests/tools/rust-workspace-checks.sh` against the real checkout (it is
//! excluded from the hermetic Nix sandbox workspace build), so repo-file and
//! repo-`git` access are sound.
//!
//! Migrated bash gates:
//!   * tests/changelog-v1-cut-eval.sh
//!   * tests/release-tag-eval.sh
//!   * tests/vfsd-watchdog-retired-eval.sh
//!   * tests/microvm-nix-absent-eval.sh

use std::process::Command;

use d2b_contract_tests::{read_repo_file, repo_root};
use regex::Regex;

// ---------------------------------------------------------------------------
// tests/changelog-v1-cut-eval.sh
//
// Asserts CHANGELOG.md keeps an empty "## [Unreleased]" block above the latest
// dated release, that the latest release header carries a YYYY-MM-DD cut date,
// and that the historical "## [1.0.0]" daemon-only release section still
// enumerates its breaking changes with a cross-reference to ADR 0015 and the
// v0 -> v1 migration guide.
// ---------------------------------------------------------------------------

/// Index of the first line in `lines` (0-based) for which `pred` is true,
/// searching at `start` and beyond.
fn first_line_idx<F: Fn(&str) -> bool>(lines: &[&str], start: usize, pred: F) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(i, l)| if pred(l) { Some(i) } else { None })
}

#[test]
fn changelog_keeps_unreleased_above_latest_dated_release() {
    let changelog = read_repo_file("CHANGELOG.md");
    let lines: Vec<&str> = changelog.lines().collect();

    // (1) "## [Unreleased]" header present.
    let unreleased_re = Regex::new(r"^## \[Unreleased\]$").unwrap();
    let unreleased_idx = first_line_idx(&lines, 0, |l| unreleased_re.is_match(l))
        .expect("no '## [Unreleased]' header found in CHANGELOG.md");

    // (2) latest release = first "## [X.Y.Z] ..." header below Unreleased.
    let release_open_re = Regex::new(r"^## \[[0-9]").unwrap();
    let latest_idx = first_line_idx(&lines, unreleased_idx + 1, |l| release_open_re.is_match(l))
        .expect("no '## [X.Y.Z]' release header found below '## [Unreleased]'");
    let latest_header = lines[latest_idx];

    // The latest release header must carry a YYYY-MM-DD cut date.
    let cut_date_re =
        Regex::new(r"^## \[[0-9]+\.[0-9]+(\.[0-9]+)?\]\s+[—-]\s+[0-9]{4}-[0-9]{2}-[0-9]{2}$")
            .unwrap();
    assert!(
        cut_date_re.is_match(latest_header),
        "latest release header missing 'YYYY-MM-DD' cut date: {latest_header}"
    );

    // Release-cut jobs can opt in to requiring an empty "## [Unreleased]" block
    // via D2B_REQUIRE_EMPTY_UNRELEASED=1. PR CI (default off) allows
    // active-development entries from concurrent work.
    if std::env::var("D2B_REQUIRE_EMPTY_UNRELEASED").as_deref() == Ok("1") {
        let between = &lines[unreleased_idx + 1..latest_idx];
        let entry_re = Regex::new(r"^### ").unwrap();
        let bullet_re = Regex::new(r"^[*-] ").unwrap();
        assert!(
            !between.iter().any(|l| entry_re.is_match(l)),
            "'## [Unreleased]' section is not empty: contains '### ' entry"
        );
        assert!(
            !between.iter().any(|l| bullet_re.is_match(l)),
            "'## [Unreleased]' section is not empty: contains bullet entry"
        );
    }
}

#[test]
fn changelog_v1_0_0_section_enumerates_breaking_changes_with_adr_and_migration_xref() {
    let changelog = read_repo_file("CHANGELOG.md");
    let lines: Vec<&str> = changelog.lines().collect();

    // The historical "## [1.0.0]" daemon-only release section.
    let v1_open_re = Regex::new(r"^## \[1\.0\.0\]( |$)").unwrap();
    let v1_idx = first_line_idx(&lines, 0, |l| v1_open_re.is_match(l))
        .expect("no '## [1.0.0]' header found in CHANGELOG.md");

    // Capture the 1.0.0 section: its header line, then every following line up
    // to (but excluding) the next "## " header.
    let next_header_re = Regex::new(r"^## ").unwrap();
    let mut section_lines: Vec<&str> = vec![lines[v1_idx]];
    for &line in &lines[v1_idx + 1..] {
        if next_header_re.is_match(line) {
            break;
        }
        section_lines.push(line);
    }
    let v1_section = section_lines.join("\n");

    // A "(breaking)" group must be enumerated.
    let breaking_re = Regex::new(r"(?m)^### .*[Bb]reaking").unwrap();
    assert!(
        breaking_re.is_match(&v1_section),
        "'## [1.0.0]' section missing a '(breaking)' group"
    );

    // Cross-reference to ADR 0015 (the binding decision).
    assert!(
        v1_section.contains("0015-daemon-only-clean-break.md"),
        "'## [1.0.0]' section does not cross-reference ADR 0015"
    );

    // Each breaking change called out for the daemon-only cut.
    for kw in [
        "manifestVersion",
        "bash CLI",
        "per-VM systemd",
        "Host singletons",
        "Polkit",
    ] {
        assert!(
            v1_section.contains(kw),
            "'## [1.0.0]' section missing required keyword: {kw}"
        );
    }

    // Cross-reference the operator-facing v0 -> v1 migration guide.
    assert!(
        v1_section.contains("docs/how-to/migrate-d2b-v0-to-v1.md"),
        "'## [1.0.0]' section does not cross-reference the v0->v1 migration guide"
    );
}

// ---------------------------------------------------------------------------
// tests/release-tag-eval.sh
//
// Asserts the `refs/tags/v1.1` release tag is:
//   (a) ANNOTATED (`git cat-file -t` returns `tag`, not `commit`),
//   (b) resolvable to a real commit,
//   (c) carries the bare version token (`v1.1`) as the first non-empty,
//       trailing-trimmed line of its tag message.
// ---------------------------------------------------------------------------

/// Run `git <args>` in the repo root, returning trimmed stdout on success and
/// `None` on a non-zero exit (matching the bash `|| true` fallthroughs).
fn git_stdout(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("failed to spawn git");
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

#[test]
fn release_v1_1_tag_is_annotated_and_names_the_release() {
    let tag_ref = "refs/tags/v1.1";
    let version = "v1.1";

    // (a) annotated check: `git cat-file -t` must be "tag", not "commit".
    let object_type = git_stdout(&["cat-file", "-t", tag_ref]);
    assert_eq!(
        object_type.as_deref(),
        Some("tag"),
        "{tag_ref} is not annotated (got: {}; lightweight tags rejected)",
        object_type.as_deref().unwrap_or("missing"),
    );

    // (b) resolvable to a commit.
    let commit = git_stdout(&["rev-parse", "--verify", &format!("{tag_ref}^{{commit}}")]);
    assert!(
        commit.as_deref().is_some_and(|c| !c.is_empty()),
        "{tag_ref} does not resolve to a commit"
    );

    // (c) tag message names the release: first non-empty, trailing-trimmed line
    // equals the version token.
    let message = git_stdout(&["tag", "-l", "--format=%(contents)", version]).unwrap_or_default();
    let first_line = message
        .lines()
        .map(|l| l.trim_end())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    assert_eq!(
        first_line, version,
        "{tag_ref} message first line {first_line:?} does not name the release {version:?}\n  tag message:\n{message}"
    );
}

// ---------------------------------------------------------------------------
// tests/vfsd-watchdog-retired-eval.sh
//
// Asserts the `d2b-vfsd-watchdog@.{service,timer}` definitions (and per-VM
// enable units) are absent from `nixos-modules/store.nix`. The wedge-detection
// logic moved into the broker's Virtiofsd `SpawnRunner` role supervisor per
// ADR 0018. Patterns match unit DECLARATIONS, not comments mentioning the name.
// ---------------------------------------------------------------------------

#[test]
fn vfsd_watchdog_units_retired_from_store_module() {
    let store_module = read_repo_file("nixos-modules/store.nix");

    // (a) Productive service template absent (string-key literal in attrs scope).
    let service_decl = Regex::new(r#"(?m)^\s*"d2b-vfsd-watchdog@"\s*="#).unwrap();
    assert!(
        !service_decl.is_match(&store_module),
        r#""d2b-vfsd-watchdog@" service template still declared in nixos-modules/store.nix"#
    );

    // (b) Productive timer template absent.
    let timer_decl = Regex::new(r#"systemd\.timers\."d2b-vfsd-watchdog@"\s*="#).unwrap();
    assert!(
        !timer_decl.is_match(&store_module),
        r#"systemd.timers."d2b-vfsd-watchdog@" still declared in nixos-modules/store.nix"#
    );

    // (c) Per-VM enabling units absent (literal "${name}" key).
    assert!(
        !store_module.contains(r#""d2b-vfsd-watchdog-${name}-enable""#),
        r#"per-VM enabling unit "d2b-vfsd-watchdog-${{name}}-enable" still declared in nixos-modules/store.nix"#
    );
}

// ---------------------------------------------------------------------------
// tests/microvm-nix-absent-eval.sh
//
// Asserts `flake.nix` does not declare `inputs.microvm`. The original bash gate
// SKIP-ed at v1.1-rc1 (the input drop was the last phase of the substrate
// replacement). The substrate replacement has since landed (ADR 0018 removed
// the microvm.nix flake input), so this test asserts the invariant strictly —
// see the "Spec correction" note in the migration commit body.
// ---------------------------------------------------------------------------

#[test]
fn microvm_nix_input_absent_from_flake() {
    let flake = read_repo_file("flake.nix");

    let microvm_attr = Regex::new(r"(?m)^\s*microvm\s*=\s*\{").unwrap();
    assert!(
        !microvm_attr.is_match(&flake),
        "flake.nix still declares a 'microvm = {{' input (ADR 0018 removed it)"
    );

    let microvm_ref = Regex::new(r"inputs\.microvm").unwrap();
    assert!(
        !microvm_ref.is_match(&flake),
        "flake.nix still references 'inputs.microvm' (ADR 0018 removed it)"
    );
}
