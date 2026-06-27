//! W3 CLI-contract integration test, migrated from
//! tests/cli-rust-native-usb.sh.
//!
//! Spawns the real `d2b` binary and asserts the USBIP CLI surface stays
//! byte-stable against the committed goldens under tests/golden/cli-output/:
//!   * `usb --help` matches usb-help.txt;
//!   * `usb attach corp-vm 1-2 --dry-run` matches usb-attach-dry-run.txt;
//!   * `usb detach corp-vm 1-2 --dry-run` matches usb-detach-dry-run.txt.
//!
//! The dry-run subcommands need the fixture-smoke bundle (D2B_FIXTURES) so
//! corp-vm resolves in the manifest. The dry-run output is host-independent
//! deterministic text (no daemon mutation), so no system-state / daemon-state
//! sandbox is required here — only that corp-vm exists in the manifest.
//!
//! Requires D2B_FIXTURES (the fixture-smoke output dir), delivered by the
//! dedicated CLI-contract step in tests/tools/rust-workspace-checks.sh. When unset
//! (plain `cargo test --workspace`) the test skips; the gate step always sets
//! D2B_FIXTURES, so the contract cannot be silently disabled there.

use std::process::Command;

/// The fixture-smoke output dir, or `None` when D2B_FIXTURES is unset.
fn fixtures_dir() -> Option<String> {
    std::env::var("D2B_FIXTURES").ok()
}

/// Read a committed golden under tests/golden/cli-output/. CARGO_MANIFEST_DIR
/// for the d2b crate is `packages/d2b`, so the repo root is two levels
/// up.
fn golden(name: &str) -> String {
    let path = format!(
        "{}/../../tests/golden/cli-output/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read golden {path}: {err}"))
}

fn run_usb(fixtures: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(args)
        .env("D2B_MANIFEST_PATH", format!("{fixtures}/manifest.json"))
        .env("D2B_BUNDLE_PATH", format!("{fixtures}/bundle.json"))
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
        "`d2b {what}` drifted from tests/golden/cli-output/{golden_name}:\n--- expected ---\n{expected}\n--- actual ---\n{actual}"
    );
}

#[test]
fn usb_help_matches_golden() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let out = run_usb(&fixtures, &["usb", "--help"]);
    assert_matches_golden(&out, "usb-help.txt", "usb --help");
}

#[test]
fn usb_attach_dry_run_matches_golden() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let out = run_usb(&fixtures, &["usb", "attach", "corp-vm", "1-2", "--dry-run"]);
    assert_matches_golden(
        &out,
        "usb-attach-dry-run.txt",
        "usb attach corp-vm 1-2 --dry-run",
    );
}

#[test]
fn usb_detach_dry_run_matches_golden() {
    let Some(fixtures) = fixtures_dir() else {
        eprintln!("SKIP: D2B_FIXTURES unset (not the gated CLI-contract step)");
        return;
    };
    let out = run_usb(&fixtures, &["usb", "detach", "corp-vm", "1-2", "--dry-run"]);
    assert_matches_golden(
        &out,
        "usb-detach-dry-run.txt",
        "usb detach corp-vm 1-2 --dry-run",
    );
}
