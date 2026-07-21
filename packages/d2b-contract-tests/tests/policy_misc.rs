//! Miscellaneous policy / source / canary lints (the "H-group"), migrated from
//! the `tests/*.sh` bash gates. Each test reads the real repo files (via the
//! `d2b_contract_tests` repo-file helpers) and asserts a structural /
//! source invariant, or reproduces a deterministic logic canary. This crate
//! runs only from `tests/tools/rust-workspace-checks.sh` against the real checkout
//! (it is excluded from the hermetic Nix sandbox workspace build), so repo-file
//! access is sound here.
//!
//! Migrated gates:
//!   * tests/vm-submodule-eval.sh -> workload_composition_shape
//!   * tests/minijail-version-check.sh -> minijail_version_comparison_canary,
//!     minijail_version_parser_canary
//!   * tests/unit/nix/cases/index.nix
//!     "index/companion-builders-do-not-read-the-module-fixpoint" ->
//!     index_companion_builders_avoid_module_fixpoint (relocated from a Nix
//!     source-text scan to this Rust policy lint, since the invariant is a
//!     repo-wide source-shape rule, not an evaluable module value)

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;

/// Whether any single line of `content` matches `pattern`. This mirrors `grep`'s
/// per-line evaluation faithfully (so a `\s*` in the pattern can never span a
/// newline boundary, as it could with a whole-file `Regex::is_match`).
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

// ---------------------------------------------------------------------------
// Migrated from tests/vm-submodule-eval.sh.
//
// Asserts the legacy evaluator split is gone and the workload evaluator owns
// the canonical composition entry point.
// ---------------------------------------------------------------------------
#[test]
fn workload_composition_shape() {
    assert!(
        !repo_path_exists("nixos-modules/vm-submodule.nix"),
        "workload composition must not restore vm-submodule.nix"
    );
    assert!(
        !repo_path_exists("nixos-modules/vm-options.nix"),
        "workload composition must not restore vm-options.nix"
    );
    let rel = "nixos-modules/vm-evaluator.nix";
    let evaluator = read_repo_file(rel);
    assert!(
        any_line_matches(&evaluator, r"_composeWorkload\s*="),
        "workload composition entry point not found in {rel}"
    );
}

// ---------------------------------------------------------------------------
// Relocated from tests/unit/nix/cases/index.nix
// "index/companion-builders-do-not-read-the-module-fixpoint".
//
// The three index companion builders (index-realms.nix, index-workloads.nix,
// index-resources.nix) are recursion-safe precisely because they only ever
// consume the raw `d2b.realms` attrs and each other's outputs — never the
// fully-merged `config.d2b._index` module fixpoint they themselves feed. That
// invariant is a source-shape rule (no reference to the module system's own
// output may appear in the files that produce it), not a value nix-unit can
// evaluate, so it belongs here as a source scan rather than as a Nix eval
// case.
// ---------------------------------------------------------------------------
#[test]
fn index_companion_builders_avoid_module_fixpoint() {
    for rel in [
        "nixos-modules/index-realms.nix",
        "nixos-modules/index-workloads.nix",
        "nixos-modules/index-resources.nix",
    ] {
        let source = read_repo_file(rel);
        assert!(
            !source.contains("config.d2b._index"),
            "{rel}: index companion builders must not read the module fixpoint \
             (config.d2b._index) they themselves feed"
        );
    }
}

// ---------------------------------------------------------------------------
// Migrated from tests/minijail-version-check.sh.
//
// A deterministic canary (not a source grep): it reproduces the
// version-comparison logic the host check uses when it refuses to start with an
// older minijail (Tier-0 pin: Nix-built minijail v17; refuse any version < 17),
// plus the parser-shape canary that extracts the integer revision from a
// `minijail0`-shaped version banner. No repo files / nix eval / fixtures are
// required, so the canary ports faithfully as a hermetic Rust unit test.
// ---------------------------------------------------------------------------

/// Tier-0 pinned minimum minijail revision (Nix-built v17).
const MINIJAIL_MIN_VERSION: u32 = 17;

/// Deterministic version comparison: returns "ok" when `observed >= required`,
/// else "too-old". Faithful port of the bash gate's `cmp_version`.
fn minijail_cmp_version(observed: u32, required: u32) -> &'static str {
    if observed >= required {
        "ok"
    } else {
        "too-old"
    }
}

#[test]
fn minijail_version_comparison_canary() {
    // (label, observed, required, expected) — verbatim from the bash gate's
    // assert_cmp cases.
    let cases = [
        (
            "pinned-current",
            MINIJAIL_MIN_VERSION,
            MINIJAIL_MIN_VERSION,
            "ok",
        ),
        (
            "newer-accepted",
            MINIJAIL_MIN_VERSION + 3,
            MINIJAIL_MIN_VERSION,
            "ok",
        ),
        (
            "older-refused",
            MINIJAIL_MIN_VERSION - 1,
            MINIJAIL_MIN_VERSION,
            "too-old",
        ),
        ("ancient-refused", 10, MINIJAIL_MIN_VERSION, "too-old"),
    ];
    for (label, observed, required, expected) in cases {
        let actual = minijail_cmp_version(observed, required);
        assert_eq!(
            actual, expected,
            "minijail-version-check[{label}]: observed={observed} required={required}: \
             expected {expected}, got {actual}"
        );
    }
}

/// Extract the integer minijail revision from a `minijail0`-shaped version
/// banner. Faithful port of the bash gate's awk:
///   /revision[ \t]+[0-9]+/{ for (i=1;i<=NF;i++) if ($i=="revision") { print $(i+1); exit } }
fn parse_minijail_revision(output: &str) -> Option<u32> {
    let line_re = Regex::new(r"revision[ \t]+[0-9]+").expect("valid revision regex");
    for line in output.lines() {
        if !line_re.is_match(line) {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        for (i, field) in fields.iter().enumerate() {
            if *field == "revision" {
                return fields.get(i + 1).and_then(|f| f.parse::<u32>().ok());
            }
        }
    }
    None
}

#[test]
fn minijail_version_parser_canary() {
    // Simulate `minijail0 --version` output and assert we extract the integer
    // revision correctly (the bash gate's `fake_output` + awk parser).
    let fake_output = "minijail0\ngoogle/minijail revision 17 abc1234\n";
    let parsed = parse_minijail_revision(fake_output);
    assert_eq!(
        parsed,
        Some(MINIJAIL_MIN_VERSION),
        "minijail-version-check: version parser extracted {parsed:?} instead of \
         {MINIJAIL_MIN_VERSION}"
    );
}
