use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use nixling_priv_broker::sys::path_safe;
use tempfile::TempDir;

const OPEN_DIR_XDEV_HELPER_ENV: &str = "NIXLING_OPEN_DIR_XDEV_HELPER";

fn scratch_dir() -> TempDir {
    let base = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("kernel-surface-tests")
        });
    fs::create_dir_all(&base).expect("create scratch base");
    TempDir::new_in(base).expect("tempdir")
}

#[test]
fn open_dir_path_safe_follows_real_mount_but_refuses_symlink() {
    if std::env::var_os(OPEN_DIR_XDEV_HELPER_ENV).is_some() {
        open_dir_path_safe_mount_contract_helper();
        return;
    }

    let exe = std::env::current_exe().expect("current test binary");
    let output = match Command::new("unshare")
        .args(["--user", "--map-root-user", "--mount"])
        .arg(exe)
        .args([
            "--exact",
            "open_dir_path_safe_follows_real_mount_but_refuses_symlink",
            "--nocapture",
        ])
        .env(OPEN_DIR_XDEV_HELPER_ENV, "1")
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            eprintln!("skipping mount-contract test: unshare unavailable");
            return;
        }
        Err(err) => panic!("failed to execute unshare: {err}"),
    };

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        && stderr.contains("Operation not permitted")
        && stderr.contains("uid_map")
    {
        eprintln!("skipping mount-contract test: user namespace uid_map denied");
        return;
    }

    assert!(
        output.status.success(),
        "helper failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        stderr,
    );
}

/// Documents the path-safety contract after the cross-mount fix: a
/// real, pre-existing mount on the walk is FOLLOWED (broker paths
/// legitimately span `/run` tmpfs, `/dev` devtmpfs, `/sys` sysfs, ...,
/// and planting a host-NS mount requires `CAP_SYS_ADMIN` = root, which
/// is out of the broker's threat model), while a SYMLINK component is
/// still refused — INCLUDING after a followed mount crossing — and a
/// path crossing TWO mounts still resolves (NO_XDEV is re-applied per
/// component, so the relax is scoped to exactly the crossing component).
fn open_dir_path_safe_mount_contract_helper() {
    let tmp = scratch_dir();
    let safe_root = tmp.path().join("safe");
    let foreign_root = tmp.path().join("foreign");
    let foreign2_root = tmp.path().join("foreign2");
    let mountpoint = safe_root.join("mnt");
    fs::create_dir_all(&mountpoint).expect("safe mountpoint");
    fs::create_dir_all(foreign_root.join("inner")).expect("foreign inner");
    fs::create_dir_all(foreign_root.join("inner2")).expect("foreign nested mountpoint");
    fs::create_dir_all(foreign2_root.join("deep")).expect("foreign2 deep");
    // A symlink that lives INSIDE the first mount (visible as
    // `mnt/post-lnk` once bound). It must be refused even though the walk
    // already followed the `mnt` crossing.
    std::os::unix::fs::symlink("inner", foreign_root.join("post-lnk"))
        .expect("create post-crossing symlink");

    let bind = |src: &std::path::Path, dst: &std::path::Path| {
        let out = Command::new("mount")
            .args(["--bind"])
            .arg(src)
            .arg(dst)
            .output()
            .expect("bind mount command");
        assert!(
            out.status.success(),
            "mount {src:?} -> {dst:?} failed\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr),
        );
    };
    // First crossing: mnt. Second (nested) crossing: mnt/inner2.
    bind(&foreign_root, &mountpoint);
    bind(&foreign2_root, &mountpoint.join("inner2"));

    // All probes capture only the outcome and DROP any fd before umount
    // (an open fd inside a mount makes umount fail with EBUSY).
    let outcome = |p: std::path::PathBuf| -> (bool, Option<i32>) {
        let r = path_safe::open_dir_path_safe(&p);
        let ok = r.is_ok();
        let errno = r.err().and_then(|e| e.raw_os_error());
        (ok, errno)
    };

    // (1) A real mount crossing is FOLLOWED.
    let followed = outcome(mountpoint.join("inner"));
    // (2) A path crossing TWO mounts still resolves.
    let two_crossings = outcome(mountpoint.join("inner2").join("deep"));
    // (3) A symlink AFTER a followed crossing is still refused (proves
    // NO_SYMLINKS is re-applied beneath the crossing, not just before it).
    let post_crossing_symlink = outcome(mountpoint.join("post-lnk"));
    // (4) A symlink BEFORE any crossing is refused.
    let symlink = safe_root.join("lnk");
    std::os::unix::fs::symlink(&foreign_root, &symlink).expect("create symlink");
    let pre_crossing_symlink = outcome(symlink);

    // Tear down nested mount first, then the outer one.
    for target in [mountpoint.join("inner2"), mountpoint.clone()] {
        let out = Command::new("umount")
            .arg(&target)
            .output()
            .expect("umount command");
        assert!(
            out.status.success(),
            "umount {target:?} failed\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr),
        );
    }

    assert!(
        followed.0,
        "a real mount crossing must be followed: {followed:?}"
    );
    assert!(
        two_crossings.0,
        "a path crossing two mounts must resolve: {two_crossings:?}"
    );
    assert_eq!(
        post_crossing_symlink.1,
        Some(nix::libc::ELOOP),
        "a symlink AFTER a followed mount crossing must be refused with ELOOP"
    );
    assert_eq!(
        pre_crossing_symlink.1,
        Some(nix::libc::ELOOP),
        "a symlink component must be refused with ELOOP"
    );
}

#[test]
fn atomic_replace_fd_installs_into_empty_target() {
    let tmp = scratch_dir();
    let target = tmp.path().join("state");
    let dir_fd = path_safe::open_dir_path_safe(tmp.path()).expect("open safe dir");

    path_safe::atomic_replace_fd(&dir_fd, "state", b"first\n", 0o640)
        .expect("install into empty target");

    assert_eq!(fs::read(&target).expect("target contents"), b"first\n");
    let mode = fs::symlink_metadata(&target)
        .expect("target metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o640);
    assert_eq!(fs::read_dir(tmp.path()).expect("dir entries").count(), 1);
}

#[test]
fn atomic_replace_fd_exchanges_existing_target() {
    let tmp = scratch_dir();
    let target = tmp.path().join("state");
    fs::write(&target, b"old\n").expect("seed target");
    let dir_fd = path_safe::open_dir_path_safe(tmp.path()).expect("open safe dir");

    path_safe::atomic_replace_fd(&dir_fd, "state", b"new\n", 0o600)
        .expect("exchange existing target");

    assert_eq!(fs::read(&target).expect("target contents"), b"new\n");
    let mode = fs::symlink_metadata(&target)
        .expect("target metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(fs::read_dir(tmp.path()).expect("dir entries").count(), 1);
}
