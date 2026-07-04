//! CLI-contract integration tests for the `d2b usb security-key` command family.
//!
//! Follows the `usb_contract.rs` pattern: spawns the real `d2b` binary and asserts
//! the security-key CLI surface stays byte-stable against committed goldens under
//! `tests/golden/cli-output/`:
//!
//! * `usb security-key --help` → `usb-security-key-help.txt`
//! * `usb security-key cancel --current --dry-run` →
//!   `usb-security-key-cancel-current-dry-run.txt`
//! * `usb security-key test corp-vm --dry-run` →
//!   `usb-security-key-test-dry-run.txt`
//!
//! The live (non-dry-run) paths — `status`, `sessions`, `cancel --apply`,
//! `test <vm>` without `--dry-run` — require a daemon handler that has not
//! landed yet. Those paths are tested for exit code 78 + structured
//! `not-yet-implemented` envelope (JSON mode) without a golden file comparison,
//! so they remain stable once the handler ships and the envelope is replaced by
//! real data.
//!
//! Requires `D2B_FIXTURES` (the fixture-smoke output dir), delivered by the
//! dedicated CLI-contract step in `tests/tools/rust-workspace-checks.sh`. When
//! unset (plain `cargo test --workspace`) the test skips; the gate step always
//! sets `D2B_FIXTURES`, so the contract cannot be silently disabled there.

use std::process::Command;

use serde_json::Value;

/// The fixture-smoke output dir, or `None` when `D2B_FIXTURES` is unset.
fn fixtures_dir() -> Option<String> {
    std::env::var("D2B_FIXTURES").ok()
}

/// Read a committed golden under `tests/golden/cli-output/`. `CARGO_MANIFEST_DIR`
/// for the `d2b` crate is `packages/d2b`, so the repo root is two levels up.
fn golden(name: &str) -> String {
    let path = format!(
        "{}/../../tests/golden/cli-output/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read golden {path}: {err}"))
}

fn run_sk(fixtures: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(args)
        .env("D2B_MANIFEST_PATH", format!("{fixtures}/manifest.json"))
        .env("D2B_BUNDLE_PATH", format!("{fixtures}/bundle.json"))
        // Point sockets at non-existent paths so dry-run/help paths don't
        // accidentally connect to the operator's live daemon.
        .env(
            "D2B_PUBLIC_SOCKET",
            format!("{fixtures}/__missing_public.sock"),
        )
        .env(
            "D2B_BROKER_SOCKET",
            format!("{fixtures}/__missing_priv.sock"),
        )
        .output()
        .unwrap_or_else(|err| panic!("spawn d2b {}: {err}", args.join(" ")))
}

fn assert_success(out: &std::process::Output, what: &str) {
    assert!(
        out.status.success(),
        "`d2b {what}` exited {:?}; stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn assert_matches_golden(out: &std::process::Output, golden_name: &str, what: &str) {
    assert_success(out, what);
    let actual = String::from_utf8_lossy(&out.stdout);
    let expected = golden(golden_name);
    assert_eq!(
        actual, expected,
        "`d2b {what}` drifted from tests/golden/cli-output/{golden_name}:\n\
         --- expected ---\n{expected}\n--- actual ---\n{actual}"
    );
}

// ---- help golden ----

#[test]
fn usb_security_key_help_matches_golden() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let out = run_sk(&fixtures, &["usb", "security-key", "--help"]);
    assert_matches_golden(&out, "usb-security-key-help.txt", "usb security-key --help");
}

// ---- dry-run golden tests ----

#[test]
fn usb_security_key_cancel_current_dry_run_matches_golden() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let out = run_sk(
        &fixtures,
        &["usb", "security-key", "cancel", "--current", "--dry-run"],
    );
    assert_matches_golden(
        &out,
        "usb-security-key-cancel-current-dry-run.txt",
        "usb security-key cancel --current --dry-run",
    );
}

#[test]
fn usb_security_key_test_dry_run_matches_golden() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let out = run_sk(
        &fixtures,
        &["usb", "security-key", "test", "corp-vm", "--dry-run"],
    );
    assert_matches_golden(
        &out,
        "usb-security-key-test-dry-run.txt",
        "usb security-key test corp-vm --dry-run",
    );
}

// ---- not-yet-implemented envelope tests ----
//
// Live paths exit 78 with a `not-yet-implemented` JSON envelope until the
// daemon handler ships. These tests pin only the exit code and envelope shape
// (not the full text) so the golden doesn't need to change when the handler
// ships.

fn assert_not_yet_implemented_json(out: &std::process::Output, what: &str) {
    assert_eq!(
        out.status.code(),
        Some(78),
        "`d2b {what}` must exit 78 (not-yet-implemented)"
    );
    assert!(
        out.stderr.is_empty(),
        "`d2b {what}` JSON mode must write envelope to stdout, not stderr"
    );
    let envelope: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "`d2b {what}` envelope parse error: {e}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_eq!(
        envelope["code"], "not-yet-implemented",
        "`d2b {what}` envelope must have code=not-yet-implemented"
    );
    assert_eq!(
        envelope["exitCode"], 78,
        "`d2b {what}` envelope must have exitCode=78"
    );
}

#[test]
fn usb_security_key_status_not_yet_implemented() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let out = run_sk(&fixtures, &["usb", "security-key", "status", "--json"]);
    assert_not_yet_implemented_json(&out, "usb security-key status --json");
}

#[test]
fn usb_security_key_sessions_not_yet_implemented() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let out = run_sk(&fixtures, &["usb", "security-key", "sessions", "--json"]);
    assert_not_yet_implemented_json(&out, "usb security-key sessions --json");
}

#[test]
fn usb_security_key_test_apply_not_yet_implemented() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    // `test <vm>` without --dry-run should also exit 78.
    let out = run_sk(
        &fixtures,
        &["usb", "security-key", "test", "corp-vm", "--json"],
    );
    assert_not_yet_implemented_json(&out, "usb security-key test corp-vm --json");
}

#[test]
fn usb_security_key_cancel_apply_not_yet_implemented() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let out = run_sk(
        &fixtures,
        &[
            "usb",
            "security-key",
            "cancel",
            "--current",
            "--apply",
            "--json",
        ],
    );
    assert_not_yet_implemented_json(&out, "usb security-key cancel --current --apply --json");
}
