//! Filesystem-posture coverage for the `d2b-activation-helper` binary,
//! migrated from `tests/activation-helper-eval.sh`. The helper replaces the
//! previous shell `[ -L ]` / `[ -f ]` / `find -type f` activation patterns that
//! had TOCTOU windows; these tests prove the typed exit codes and the
//! openat2 + RESOLVE_NO_SYMLINKS refusals for every verb.
//!
//! Layer 1: no NixOS module evaluation, no root. Each case drives the real
//! binary via `CARGO_BIN_EXE_d2b-activation-helper` against an isolated
//! `tempdir()`. The existing `activation_helper_build_farm.rs` covers the
//! `build-store-view{,-farm}` verbs; this file covers `ensure-regular-file`,
//! `enforce-dir-posture`, `setfacl-on-path`, `clear-acl-on-path`, and
//! `chown-if-orphan`.
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

fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

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

#[test]
fn ensure_regular_file_happy_path_and_re_assert() {
    let dir = tempdir().unwrap();
    let (uid, gid) = (uid(), gid());

    // Happy path: create a 1 MiB file with mode 0600.
    let target = dir.path().join("happy.img");
    let rc = run(&[
        "ensure-regular-file",
        "--path",
        target.to_str().unwrap(),
        "--uid",
        &uid,
        "--gid",
        &gid,
        "--mode",
        "0600",
        "--size-mib",
        "1",
    ]);
    assert_eq!(rc, Some(0), "ensure-regular-file happy path must exit 0");
    assert_eq!(
        fs::metadata(&target).unwrap().len(),
        1024 * 1024,
        "must create a 1 MiB file"
    );
    assert_eq!(mode_of(&target), 0o600, "must apply mode 0600");

    // Existing-file re-assert (size-mib=0): re-applies mode, preserves content.
    let exist = dir.path().join("exist.img");
    fs::write(&exist, b"existing content").unwrap();
    fs::set_permissions(&exist, fs::Permissions::from_mode(0o644)).unwrap();
    let rc = run(&[
        "ensure-regular-file",
        "--path",
        exist.to_str().unwrap(),
        "--uid",
        &uid,
        "--gid",
        &gid,
        "--mode",
        "0600",
        "--size-mib",
        "0",
    ]);
    assert_eq!(rc, Some(0), "re-assert must exit 0");
    assert_eq!(mode_of(&exist), 0o600, "re-assert must re-apply mode 0600");
    assert_eq!(
        fs::read(&exist).unwrap(),
        b"existing content",
        "re-assert must not modify content"
    );
}

#[test]
fn ensure_regular_file_refuses_wrong_types_with_exit_2() {
    let dir = tempdir().unwrap();
    let (uid, gid) = (uid(), gid());
    let ensure = |path: &Path| {
        run(&[
            "ensure-regular-file",
            "--path",
            path.to_str().unwrap(),
            "--uid",
            &uid,
            "--gid",
            &gid,
            "--mode",
            "0600",
            "--size-mib",
            "1",
        ])
    };

    // Symlink refusal (the critical TOCTOU fix).
    let evil = dir.path().join("evil.img");
    symlink("/etc/shadow", &evil).unwrap();
    assert_eq!(ensure(&evil), Some(2), "must refuse symlink with exit 2");

    // Directory refusal.
    let as_dir = dir.path().join("dir.img");
    fs::create_dir(&as_dir).unwrap();
    assert_eq!(
        ensure(&as_dir),
        Some(2),
        "must refuse directory with exit 2"
    );

    // Intermediate-symlink refusal (RESOLVE_NO_SYMLINKS rejects symlinks at any
    // component, not just the final segment).
    let inner = dir.path().join("inner-dir");
    fs::create_dir(&inner).unwrap();
    let inner_link = dir.path().join("inner-link");
    symlink(&inner, &inner_link).unwrap();
    assert_eq!(
        ensure(&inner_link.join("test.img")),
        Some(2),
        "must refuse intermediate-symlink with exit 2"
    );

    // FIFO refusal with NO hang (the O_NONBLOCK fix): a 5s ceiling proves the
    // open does not wedge, and the typed exit 2 is returned rather than a
    // signal-kill (None).
    let fifo = dir.path().join("fifo.img");
    mkfifo(&fifo);
    let rc = run_with_timeout(
        &[
            "ensure-regular-file",
            "--path",
            fifo.to_str().unwrap(),
            "--uid",
            &uid,
            "--gid",
            &gid,
            "--mode",
            "0600",
            "--size-mib",
            "1",
        ],
        std::time::Duration::from_secs(5),
    );
    assert_eq!(rc, Some(2), "must refuse FIFO with exit 2 and not hang");
}

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

#[test]
fn chown_if_orphan_known_owner_noop_and_symlink_refusal() {
    let dir = tempdir().unwrap();

    // Current process owns the file; uid maps to a real user -> no-op.
    let owned = dir.path().join("owned.txt");
    fs::write(&owned, b"").unwrap();
    let rc = run(&[
        "chown-if-orphan",
        "--path",
        owned.to_str().unwrap(),
        "--uid",
        "0",
        "--gid",
        "0",
    ]);
    assert_eq!(rc, Some(0), "chown-if-orphan known-owner must exit 0");
    let post = fs::metadata(&owned).unwrap();
    assert_eq!(
        std::os::unix::fs::MetadataExt::uid(&post).to_string(),
        uid(),
        "chown-if-orphan must NOT chown a known-owner file"
    );

    // Symlink refusal.
    let evil = dir.path().join("evil-chown.txt");
    symlink("/etc/shadow", &evil).unwrap();
    assert_eq!(
        run(&[
            "chown-if-orphan",
            "--path",
            evil.to_str().unwrap(),
            "--uid",
            "0",
            "--gid",
            "0",
        ]),
        Some(2),
        "chown-if-orphan must refuse symlink with exit 2"
    );
}

#[test]
fn setfacl_and_clear_acl_refusals() {
    let setfacl = match which("setfacl") {
        Some(p) => p,
        None => {
            eprintln!("setfacl not on PATH; skipping setfacl-on-path coverage");
            return;
        }
    };
    let dir = tempdir().unwrap();
    let uid = uid();

    // Happy path: apply u:UID:r via /proc/self/fd.
    let aclfile = dir.path().join("aclfile.txt");
    fs::write(&aclfile, b"").unwrap();
    fs::set_permissions(&aclfile, fs::Permissions::from_mode(0o644)).unwrap();
    let rc = run(&[
        "setfacl-on-path",
        "--path",
        aclfile.to_str().unwrap(),
        "--acl-spec",
        &format!("u:{uid}:r"),
        "--setfacl-bin",
        &setfacl,
    ]);
    assert_eq!(rc, Some(0), "setfacl-on-path happy path must exit 0");

    // Symlink refusal.
    let evil = dir.path().join("evil-acl.txt");
    symlink("/etc/shadow", &evil).unwrap();
    assert_eq!(
        run(&[
            "setfacl-on-path",
            "--path",
            evil.to_str().unwrap(),
            "--acl-spec",
            &format!("u:{uid}:r"),
            "--setfacl-bin",
            &setfacl,
        ]),
        Some(2),
        "setfacl-on-path must refuse symlink with exit 2"
    );

    // clear-acl-on-path symlink refusal.
    assert_eq!(
        run(&[
            "clear-acl-on-path",
            "--path",
            evil.to_str().unwrap(),
            "--setfacl-bin",
            &setfacl,
        ]),
        Some(2),
        "clear-acl-on-path must refuse symlink with exit 2"
    );

    // --require-kind regular refuses a directory.
    let as_dir = dir.path().join("aclrequire-dir");
    fs::create_dir(&as_dir).unwrap();
    assert_eq!(
        run(&[
            "setfacl-on-path",
            "--path",
            as_dir.to_str().unwrap(),
            "--acl-spec",
            &format!("u:{uid}:r"),
            "--require-kind",
            "regular",
            "--setfacl-bin",
            &setfacl,
        ]),
        Some(2),
        "setfacl-on-path --require-kind regular must refuse a directory with exit 2"
    );

    // FIFO refusal with no hang.
    let fifo = dir.path().join("aclfifo");
    mkfifo(&fifo);
    let rc = run_with_timeout(
        &[
            "setfacl-on-path",
            "--path",
            fifo.to_str().unwrap(),
            "--acl-spec",
            &format!("u:{uid}:r"),
            "--require-kind",
            "regular",
            "--setfacl-bin",
            &setfacl,
        ],
        std::time::Duration::from_secs(5),
    );
    assert_eq!(
        rc,
        Some(2),
        "setfacl-on-path must refuse FIFO with exit 2 and not hang"
    );
}

// --- helpers ---------------------------------------------------------------

fn mkfifo(path: &Path) {
    nix::unistd::mkfifo(path, nix::sys::stat::Mode::from_bits_truncate(0o644))
        .unwrap_or_else(|e| panic!("mkfifo({}) failed: {e}", path.display()));
}

/// Run the helper with a wall-clock ceiling, returning the exit code. Returns
/// `None` if the process had to be killed (i.e. it hung past the ceiling),
/// which the no-hang assertions treat as a failure distinct from exit 2.
fn run_with_timeout(args: &[&str], ceiling: std::time::Duration) -> Option<i32> {
    let mut child = Command::new(HELPER)
        .args(args)
        .spawn()
        .expect("spawn helper (timeout)");
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status.code();
        }
        if start.elapsed() > ceiling {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn which(bin: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|candidate| candidate.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}
