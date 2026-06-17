//! Supervisor-removal + USBIP-state-machine policy / source-lint gates (the
//! "H-group"), migrated from the `tests/*-eval.sh` bash gates. Each test reads
//! the real repo files (via the `nixling_contract_tests` repo-file helpers) and
//! asserts a structural / documentation invariant. This crate runs only from
//! `tests/tools/rust-workspace-checks.sh` against the real checkout (it is excluded
//! from the hermetic Nix sandbox workspace build), so repo-file access is sound.
//!
//! Migrated gates:
//!   * tests/supervisor-option-absent-eval.sh -> supervisor_option_absent
//!   * tests/usbip-state-machine-eval.sh       -> usbip_state_machine_surface

use nixling_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;

/// Whether any single line of `content` matches `pattern`. This mirrors `grep`'s
/// per-line evaluation faithfully (so a `.*` in the pattern can never span a
/// newline boundary, as it could with a whole-file `Regex::is_match`).
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

// ---------------------------------------------------------------------------
// Migrated from tests/supervisor-option-absent-eval.sh.
//
// v1.1 invariant gate asserting (a) the productive `supervisor = lib.mkOption`
// declaration is gone from `nixos-modules/options-vms.nix`, and (b) the
// top-level defense-in-depth assertion in `nixos-modules/assertions.nix` fires
// on any per-VM `nixling.vms.<vm>.supervisor` definition with the friendly
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
        !any_line_matches(&options, r"^\s*supervisor\s*=\s*lib\.mkOption"),
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
        any_line_matches(&assertions, r"vm \? supervisor|vms\.\$\{name\}\.supervisor"),
        "supervisor-option-absent-eval: supervisor-fallback assertion missing from \
         {assertions_rel}"
    );

    // (b) friendly ADR-0015 message text present.
    assert!(
        any_line_matches(
            &assertions,
            r"removed in v1\.1.*per ADR 0015|ADR 0015.*daemon-only clean break"
        ),
        "supervisor-option-absent-eval: ADR-0015 friendly message text missing from \
         {assertions_rel}"
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/usbip-state-machine-eval.sh.
//
// Integration gate for the typed per-busid USBIP state machine. The state
// machine lives in `packages/nixlingd/src/usbip_state_machine.rs`; the
// in-process behaviour is covered by that module's own `#[test]`s. This gate is
// the repo-level source/doc lint that:
//   1. Confirms every source + doc file is present.
//   2. Confirms the module is wired into `nixlingd::lib`
//      (`pub mod usbip_state_machine;`).
//   3. Confirms `CANONICAL_STEPS` pins the canonical bring-up order
//      `modprobe -> lock -> withhold -> firewall -> backend -> bind -> proxy`.
//   4. Confirms `TypedError::UsbipStepFailed` is wired with the pinned
//      `"usbip-step-failed"` kind and exit code 67.
//   5. Confirms the reference doc names the canonical order verbatim.
//
// Read-only / eval-time-only; no live host required.
// ---------------------------------------------------------------------------
#[test]
fn usbip_state_machine_surface() {
    let src_rel = "packages/nixlingd/src/usbip_state_machine.rs";
    let lib_rel = "packages/nixlingd/src/lib.rs";
    let typed_rel = "packages/nixlingd/src/typed_error.rs";
    let doc_rel = "docs/reference/usbip-state-machine.md";

    // Every source + doc file is present.
    for (rel, what) in [
        (src_rel, "module"),
        (lib_rel, "lib.rs"),
        (typed_rel, "typed_error.rs"),
        (doc_rel, "doc"),
    ] {
        assert!(
            repo_path_exists(rel),
            "usbip-state-machine-eval: {what} missing: {rel}"
        );
    }

    let src = read_repo_file(src_rel);
    let lib = read_repo_file(lib_rel);
    let typed = read_repo_file(typed_rel);
    let doc = read_repo_file(doc_rel);

    // (1) lib.rs declares the module.
    assert!(
        lib.contains("pub mod usbip_state_machine;"),
        "usbip-state-machine-eval: lib.rs does not declare 'pub mod usbip_state_machine;'"
    );

    // (2) Canonical step ordering pinned in the CANONICAL_STEPS const. Extract
    // the seven UsbipBusidStep::* names in source order and compare against the
    // canonical pin (a faithful port of the bash awk|grep|sed pipeline).
    let got = canonical_steps_from_source(&src);
    let want = "Modprobe Lock Withhold Firewall Backend Bind Proxy";
    assert_eq!(
        got, want,
        "usbip-state-machine-eval: canonical order drift: got [{got}] want [{want}]"
    );

    // (3) Typed error wiring: variant, kind string, and exit code 67.
    assert!(
        typed.contains("UsbipStepFailed"),
        "usbip-state-machine-eval: typed_error.rs missing UsbipStepFailed variant"
    );
    assert!(
        typed.contains(r#""usbip-step-failed""#),
        "usbip-state-machine-eval: typed_error.rs missing 'usbip-step-failed' kind string"
    );
    assert!(
        any_line_matches(&typed, r"Self::UsbipStepFailed \{ \.\. \} => 67"),
        "usbip-state-machine-eval: typed_error.rs UsbipStepFailed exit code is not 67"
    );

    // (4) Doc cross-check: must name the canonical order verbatim so prose can't
    // drift from the code. The marker is the canonical-order string itself, NOT
    // this gate's retired `.sh` filename (which the doc also references at a
    // separate line) — see the self-referential-doc note in the migration record.
    assert!(
        doc.contains("modprobe → lock → withhold → firewall → backend → bind → proxy"),
        "usbip-state-machine-eval: doc does not name the canonical order verbatim"
    );
}

/// Faithful port of the bash gate's
/// `awk '/pub const CANONICAL_STEPS/,/\];/' | grep -oE 'UsbipBusidStep::[A-Za-z]+'
/// | sed 's/UsbipBusidStep:://' | tr '\n' ' '` pipeline: walk the
/// `CANONICAL_STEPS` const body and join the bare step names with a single space.
fn canonical_steps_from_source(src: &str) -> String {
    let start = Regex::new(r"pub const CANONICAL_STEPS").expect("valid start regex");
    let end = Regex::new(r"\];").expect("valid end regex");
    let step = Regex::new(r"UsbipBusidStep::([A-Za-z]+)").expect("valid step regex");

    let mut in_range = false;
    let mut names: Vec<String> = Vec::new();
    for line in src.lines() {
        if !in_range && start.is_match(line) {
            in_range = true;
        }
        if in_range {
            for caps in step.captures_iter(line) {
                names.push(caps[1].to_string());
            }
            if end.is_match(line) {
                in_range = false;
            }
        }
    }
    names.join(" ")
}
