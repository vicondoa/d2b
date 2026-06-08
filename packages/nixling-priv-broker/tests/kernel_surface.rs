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
fn open_dir_path_safe_rejects_mount_crossing() {
    if std::env::var_os(OPEN_DIR_XDEV_HELPER_ENV).is_some() {
        open_dir_path_safe_rejects_mount_crossing_helper();
        return;
    }

    let exe = std::env::current_exe().expect("current test binary");
    let output = match Command::new("unshare")
        .args(["--user", "--map-root-user", "--mount"])
        .arg(exe)
        .args([
            "--exact",
            "open_dir_path_safe_rejects_mount_crossing",
            "--nocapture",
        ])
        .env(OPEN_DIR_XDEV_HELPER_ENV, "1")
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            eprintln!("skipping mount-crossing test: unshare unavailable");
            return;
        }
        Err(err) => panic!("failed to execute unshare: {err}"),
    };

    assert!(
        output.status.success(),
        "helper failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn open_dir_path_safe_rejects_mount_crossing_helper() {
    let tmp = scratch_dir();
    let safe_root = tmp.path().join("safe");
    let foreign_root = tmp.path().join("foreign");
    let mountpoint = safe_root.join("mnt");
    fs::create_dir_all(&mountpoint).expect("safe mountpoint");
    fs::create_dir_all(&foreign_root).expect("foreign dir");

    let mount_output = Command::new("mount")
        .args(["--bind"])
        .arg(&foreign_root)
        .arg(&mountpoint)
        .output()
        .expect("bind mount command");
    assert!(
        mount_output.status.success(),
        "mount failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&mount_output.stdout),
        String::from_utf8_lossy(&mount_output.stderr),
    );

    let err = path_safe::open_dir_path_safe(&mountpoint)
        .expect_err("RESOLVE_NO_XDEV must reject mount crossing");

    let umount_output = Command::new("umount")
        .arg(&mountpoint)
        .output()
        .expect("umount command");
    assert!(
        umount_output.status.success(),
        "umount failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&umount_output.stdout),
        String::from_utf8_lossy(&umount_output.stderr),
    );

    assert_eq!(err.raw_os_error(), Some(nix::libc::EXDEV));
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
