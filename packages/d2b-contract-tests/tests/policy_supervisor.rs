//! Supervisor-removal policy gate.
//!
//! Migrated gate:
//!   * tests/supervisor-option-absent-eval.sh -> supervisor_option_absent

use d2b_contract_tests::{read_repo_file, repo_path_exists};
fn any_line_contains(content: &str, needle: &str) -> bool {
    content.lines().any(|line| line.contains(needle))
}

// ---------------------------------------------------------------------------
// Migrated from tests/supervisor-option-absent-eval.sh.
//
// v1.1 invariant gate asserting (a) the productive `supervisor = lib.mkOption`
// declaration is gone from `nixos-modules/options-vms.nix`, and (b) the
// top-level defense-in-depth assertion in `nixos-modules/assertions.nix` fires
// on any per-VM `d2b.vms.<vm>.supervisor` definition with the friendly
// ADR-0015 (daemon-only clean break) message. The per-submodule
// `mkRemovedOptionModule` shim approach is incompatible with `attrsOf
// submodule` semantics (no `assertions` option at the per-VM layer), so the
// top-level fallback assertion is the sole supervisor-removal guard.
// ---------------------------------------------------------------------------
#[test]
fn supervisor_option_absent() {
    // (a) productive declaration gone from options-vms.nix.
    let options_rel = "nixos-modules/options-vms.nix";
    assert!(
        repo_path_exists(options_rel),
        "supervisor-option-absent-eval: {options_rel} missing"
    );
    let options = read_repo_file(options_rel);
    assert!(
        !any_line_contains(&options, "supervisor = lib.mkOption"),
        "supervisor-option-absent-eval: productive `supervisor = lib.mkOption` still present \
         in {options_rel}"
    );

    // (b) assertions.nix must exist and carry the top-level fallback assertion.
    let assertions_rel = "nixos-modules/assertions.nix";
    assert!(
        repo_path_exists(assertions_rel),
        "supervisor-option-absent-eval: assertions.nix missing"
    );
    let assertions = read_repo_file(assertions_rel);
    assert!(
        any_line_contains(&assertions, "vm ? supervisor")
            || any_line_contains(&assertions, "vms.${name}.supervisor"),
        "supervisor-option-absent-eval: supervisor-fallback assertion missing from \
         {assertions_rel}"
    );

    // (b) friendly ADR-0015 message text present.
    assert!(
        any_line_contains(&assertions, "removed in v1.1")
            && any_line_contains(&assertions, "ADR 0015"),
        "supervisor-option-absent-eval: ADR-0015 friendly message text missing from \
         {assertions_rel}"
    );
}
