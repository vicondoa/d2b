//! Filesystem-posture coverage for the `d2b-activation-helper` binary,
//! migrated from `tests/activation-helper-eval.sh`. The helper replaces the
//! previous shell `[ -L ]` / `[ -f ]` / `find -type f` activation patterns that
//! had TOCTOU windows; these tests prove the typed exit codes and the
//! openat2 + RESOLVE_NO_SYMLINKS refusals for every verb.
//!
//! Layer 1: no NixOS module evaluation, no root. Each case drives the real
//! binary via `CARGO_BIN_EXE_d2b-activation-helper` against an isolated
//! `tempdir()`. The existing `activation_helper_build_farm.rs` covers the
//! `build-store-view{,-farm}` verbs; this file covers `enforce-dir-posture`.
#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;

const HELPER: &str = env!("CARGO_BIN_EXE_d2b-activation-helper");

/// Run the helper with `args` and return its exit code (`None` if killed by a
/// signal, which the no-hang FIFO cases assert against).
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn run(args: &[&str]) -> Option<i32> {
    Command::new(HELPER)
        .args(args)
        .output()
        .expect("spawn d2b-activation-helper")
        .status
        .code()
}

fn uid() -> String {
    nix::unistd::Uid::current().as_raw().to_string()
}

fn gid() -> String {
    nix::unistd::Gid::current().as_raw().to_string()
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn help_exits_zero_and_missing_verb_exits_one() {
    let out = Command::new(HELPER)
        .arg("--help")
        .output()
        .expect("spawn helper --help");
    assert_eq!(out.status.code(), Some(0), "--help must exit 0");
    let help = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
    assert!(
        help.contains("d2b-activation-helper"),
        "--help must print usage, got: {help}"
    );

    // No verb -> exit 1.
    assert_eq!(run(&[]), Some(1), "missing verb must exit 1");
}
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn enforce_dir_posture_happy_refusals_and_idempotent_noop() {
    let dir = tempdir().unwrap();
    let (uid, gid) = (uid(), gid());
    let enforce = |path: &Path| {
        run(&[
            "enforce-dir-posture",
            "--path",
            path.to_str().unwrap(),
            "--uid",
            &uid,
            "--gid",
            &gid,
            "--mode",
            "0750",
        ])
    };

    // Happy path: set mode 0750 on a directory.
    let posture = dir.path().join("posture-dir");
    fs::create_dir(&posture).unwrap();
    assert_eq!(enforce(&posture), Some(0), "happy path must exit 0");
    assert_eq!(mode_of(&posture), 0o750, "must set mode 0750");

    // Symlink refusal.
    let dir_link = dir.path().join("dir-link");
    symlink(&posture, &dir_link).unwrap();
    assert_eq!(
        enforce(&dir_link),
        Some(2),
        "must refuse symlink with exit 2"
    );

    // Intermediate-symlink refusal.
    let inner = dir.path().join("inner-dir");
    fs::create_dir(&inner).unwrap();
    let inner_link = dir.path().join("inner-link");
    symlink(&inner, &inner_link).unwrap();
    assert_eq!(
        enforce(&inner_link),
        Some(2),
        "must refuse intermediate-symlink with exit 2"
    );

    // Missing path is an idempotent no-op (activation may run before the
    // directory exists).
    assert_eq!(
        enforce(&dir.path().join("does-not-exist")),
        Some(0),
        "missing path must be an idempotent no-op (exit 0)"
    );
}
// --- helpers ---------------------------------------------------------------
