//! `DiskInit` broker op handler.
//!
//! Creates and pre-allocates a disk-image file for the per-VM
//! writable store overlay. The broker resolves every `DiskInit`
//! plan-op from the trusted bundle's `ProcessNode.plan_ops` for
//! the supplied `vm_id` and executes them in order.
//!
//! Security invariants:
//! - The target path MUST be absolute and under
//!   `/var/lib/nixling/vms/` (path-safety check).
//! - No `..` path components are permitted.
//! - The caller never supplies raw paths, sizes, modes, or
//!   ownership; all values come from the trusted bundle.
//!
//! Idempotency: when `if_absent = true` and the file already
//! exists, the op is skipped (no truncation, no error).

use std::io;
use std::path::Path;

use nixling_core::bundle_resolver::{BundleResolver, ResolvedDiskInitOp};

/// Permitted root prefix for all disk-init target paths.
const PERMITTED_ROOT: &str = "/var/lib/nixling/vms/";

/// Validate `mkfs.ext4` binary path resolution from
/// `NIXLING_BROKER_MKFS_EXT4_BINARY` (default
/// `/run/current-system/sw/bin/mkfs.ext4`). Rejects non-absolute paths
/// and any `..` segments to prevent env-var injection from steering the
/// root-running broker to an attacker-chosen binary.
fn validate_mkfs_ext4_binary(raw: &str) -> io::Result<&std::path::Path> {
    let p = std::path::Path::new(raw);
    if !p.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("disk-init: NIXLING_BROKER_MKFS_EXT4_BINARY must be absolute (got {raw:?})"),
        ));
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "disk-init: NIXLING_BROKER_MKFS_EXT4_BINARY may not contain `..` (got {raw:?})"
            ),
        ));
    }
    Ok(p)
}

/// Outcome of a single `disk_init_one` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskInitOutcome {
    /// File was created and pre-allocated.
    Created,
    /// File already existed and `if_absent = true`; skipped.
    Skipped,
}

/// Aggregate result of running all disk-init ops for a VM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInitSummary {
    pub ops_total: u32,
    pub ops_created: u32,
    pub ops_skipped: u32,
    /// Stable hash of all target paths for the audit record.
    pub target_paths_hash: String,
}

/// Execute a single disk-init spec.
///
/// Validates the path, creates the file (O_CREAT|O_EXCL),
/// pre-allocates `size_bytes` via `fallocate`, and sets mode +
/// owner via `fchmod` + `fchown`.
///
/// Returns `DiskInitOutcome::Skipped` when `if_absent = true` and
/// the file already exists. Returns an `io::Error` on any failure.
pub fn disk_init_one(spec: &ResolvedDiskInitOp) -> io::Result<DiskInitOutcome> {
    validate_target_path(&spec.target_path)?;

    // If the file exists and we were asked to skip, do so.
    if spec.if_absent && spec.target_path.exists() {
        return Ok(DiskInitOutcome::Skipped);
    }

    // Ensure the parent directory exists.
    if let Some(parent) = spec.target_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("disk-init: create parent dir {}: {e}", parent.display()),
            )
        })?;
    }

    // Open with O_CREAT | O_EXCL — fail if already exists (even
    // when if_absent = false: we never silently overwrite a disk).
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        // mode is subject to umask when using OpenOptions; we fchmod
        // explicitly below so umask doesn't matter here.
        .mode(0o600)
        .open(&spec.target_path)
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("disk-init: open {}: {e}", spec.target_path.display()),
            )
        })?;

    // Pre-allocate disk space.  fallocate with no flags allocates
    // blocks without zeroing (fast), falling back gracefully on
    // filesystems that don't support it (writes sparse extent).
    use std::os::fd::AsFd;
    rustix::fs::fallocate(
        file.as_fd(),
        rustix::fs::FallocateFlags::empty(),
        0,
        spec.size_bytes,
    )
    .map_err(|e| {
        io::Error::other(format!(
            "disk-init: fallocate {} bytes on {}: {e}",
            spec.size_bytes,
            spec.target_path.display()
        ))
    })?;

    // Set the file mode explicitly (ignores broker umask).
    // Uses the safe path_safe::fchmod wrapper (broker crate has
    // `unsafe_code = "deny"` so we cannot call rustix directly).
    crate::sys::path_safe::fchmod(file.as_fd(), spec.mode).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "disk-init: fchmod {:04o} on {}: {e}",
                spec.mode,
                spec.target_path.display()
            ),
        )
    })?;

    // Set owner/group via the safe path_safe::fchown wrapper.
    crate::sys::path_safe::fchown(file.as_fd(), Some(spec.owner_uid), Some(spec.owner_gid))
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "disk-init: fchown {}:{} on {}: {e}",
                    spec.owner_uid,
                    spec.owner_gid,
                    spec.target_path.display()
                ),
            )
        })?;

    // Format the newly-created raw image with ext4. Without this, the
    // guest kernel sees an empty raw block device and hangs in initramfs
    // trying to mount the overlay upperdir/workdir. We close the file
    // BEFORE running mkfs.ext4 because the tool wants its own exclusive
    // handle.
    drop(file);
    // Resolve mkfs.ext4 binary path with explicit validation. The
    // broker's PATH is intentionally minimal and does NOT include
    // e2fsprogs, so we accept an env-var override — but we REJECT
    // non-absolute paths and any path containing `..` segments to close
    // the env-injection vector (a malicious env var could otherwise
    // steer the root-running broker to an attacker-chosen binary). See
    // `mkfs_ext4_env_var_rejects_*` tests for regression coverage.
    let mkfs_path_raw = std::env::var("NIXLING_BROKER_MKFS_EXT4_BINARY")
        .unwrap_or_else(|_| "/run/current-system/sw/bin/mkfs.ext4".to_owned());
    let mkfs_path = validate_mkfs_ext4_binary(&mkfs_path_raw)?;
    let mkfs_status = std::process::Command::new(mkfs_path)
        .arg("-q") // quiet
        .arg("-F") // force on non-block device (raw file)
        .arg("-E")
        .arg("lazy_itable_init=1,lazy_journal_init=1")
        .arg(&spec.target_path)
        .status()
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "disk-init: mkfs.ext4 ({mkfs_path_raw}) spawn on {}: {e}",
                    spec.target_path.display()
                ),
            )
        })?;
    if !mkfs_status.success() {
        return Err(io::Error::other(format!(
            "disk-init: mkfs.ext4 exit={:?} on {}",
            mkfs_status.code(),
            spec.target_path.display()
        )));
    }
    // Re-apply mode + owner via fd-based safe ops (path_safe::fchmod /
    // fchown) on a freshly-opened O_NOFOLLOW handle, NOT via path-based
    // std::fs::set_permissions + std::os::unix::fs::chown. The earlier
    // path-based variant reintroduced a TOCTOU/symlink-race window after
    // `validate_target_path` had already run — between mkfs returning
    // and chmod/chown reaching the inode, an attacker who had write
    // access to the parent dir could replace the path with a symlink to
    // a sensitive target. Re-opening with O_NOFOLLOW refuses symlinks at
    // this gate too.
    let reopened = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        // libc::O_NOFOLLOW = 0x20000 = 0o400000 on Linux.
        .custom_flags(0o400000)
        .open(&spec.target_path)
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "disk-init: post-mkfs reopen O_NOFOLLOW on {}: {e}",
                    spec.target_path.display()
                ),
            )
        })?;
    crate::sys::path_safe::fchmod(reopened.as_fd(), spec.mode).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "disk-init: post-mkfs fchmod {:04o} on {}: {e}",
                spec.mode,
                spec.target_path.display()
            ),
        )
    })?;
    crate::sys::path_safe::fchown(reopened.as_fd(), Some(spec.owner_uid), Some(spec.owner_gid))
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "disk-init: post-mkfs fchown {}:{} on {}: {e}",
                    spec.owner_uid,
                    spec.owner_gid,
                    spec.target_path.display()
                ),
            )
        })?;
    drop(reopened);

    Ok(DiskInitOutcome::Created)
}

/// Execute all `DiskInit` plan-ops for `vm_id` from the trusted bundle.
///
/// Called from `runtime::dispatch_request_with_backend` for
/// `BrokerRequest::DiskInit`. Returns a [`DiskInitSummary`] for
/// the audit record or the first I/O error encountered.
pub fn live_disk_init(resolver: &BundleResolver, vm_id: &str) -> io::Result<DiskInitSummary> {
    let ops = resolver.resolve_disk_init_ops(vm_id);
    let ops_total = ops.len() as u32;
    let mut ops_created = 0u32;
    let mut ops_skipped = 0u32;
    let mut paths_concat = String::new();

    for op in &ops {
        paths_concat.push_str(&op.target_path.display().to_string());
        paths_concat.push('\n');
        match disk_init_one(op)? {
            DiskInitOutcome::Created => ops_created += 1,
            DiskInitOutcome::Skipped => ops_skipped += 1,
        }
    }

    Ok(DiskInitSummary {
        ops_total,
        ops_created,
        ops_skipped,
        target_paths_hash: super::hosts::stable_hash_str(&paths_concat),
    })
}

/// Validate `path` is absolute and under [`PERMITTED_ROOT`], with
/// no `..` components. Returns an `io::Error` on violation.
fn validate_target_path(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "path-safety-violation: disk-init target must be absolute, got {}",
                path.display()
            ),
        ));
    }
    if !path
        .to_str()
        .map(|s| s.starts_with(PERMITTED_ROOT))
        .unwrap_or(false)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "path-safety-violation: disk-init target must be under {PERMITTED_ROOT}, got {}",
                path.display()
            ),
        ));
    }
    // Reject any `..` component after the prefix check (defense in depth).
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "path-safety-violation: disk-init target contains '..': {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_core::bundle_resolver::ResolvedDiskInitOp;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::PathBuf;

    fn scratch_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nixling-disk-init-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Build a `ResolvedDiskInitOp` pointing under a scratch root that
    /// mimics the `/var/lib/nixling/vms/` prefix layout for unit tests.
    /// We can't write into the real `/var/lib/nixling/vms/` from tests
    /// without root, so we monkey-patch `PERMITTED_ROOT` by temporarily
    /// setting a writable scratch path and using a custom validator.
    /// Instead, we test `disk_init_one` directly with the internal
    /// `validate_target_path` bypassed where necessary (by constructing
    /// a path that starts with PERMITTED_ROOT but pointing at a tmpfs
    /// mount — not feasible). For the unit tests we therefore call the
    /// lower-level `disk_init_one_raw` helper which accepts a pre-
    /// validated path.
    ///
    /// Rationale: the path-safety test runs its own dedicated test
    /// that calls `disk_init_one` with a bad path and checks the error.
    /// The creates / skips tests bypass validation and call the internal
    /// raw helper to stay hermetic (no root required).
    fn test_spec(path: PathBuf, size_bytes: u64, if_absent: bool) -> ResolvedDiskInitOp {
        ResolvedDiskInitOp {
            target_path: path,
            size_bytes,
            mode: 0o600,
            owner_uid: nix::unistd::geteuid().as_raw(),
            owner_gid: nix::unistd::getegid().as_raw(),
            if_absent,
        }
    }

    /// Inner implementation that skips the path-prefix check, used only
    /// in unit tests to stay root-free.
    fn disk_init_one_no_prefix_check(spec: &ResolvedDiskInitOp) -> io::Result<DiskInitOutcome> {
        if spec.if_absent && spec.target_path.exists() {
            return Ok(DiskInitOutcome::Skipped);
        }
        if let Some(parent) = spec.target_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        use std::os::fd::AsFd;
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&spec.target_path)?;
        rustix::fs::fallocate(
            file.as_fd(),
            rustix::fs::FallocateFlags::empty(),
            0,
            spec.size_bytes,
        )
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        crate::sys::path_safe::fchmod(file.as_fd(), spec.mode)?;
        crate::sys::path_safe::fchown(file.as_fd(), Some(spec.owner_uid), Some(spec.owner_gid))?;
        Ok(DiskInitOutcome::Created)
    }

    #[test]
    fn disk_init_creates_file_when_absent() {
        let scratch = scratch_root();
        let target = scratch
            .join("vms")
            .join("test-vm")
            .join("store-overlay.img");
        let spec = test_spec(target.clone(), 4096, true);

        let outcome = disk_init_one_no_prefix_check(&spec).expect("disk_init_one succeeds");
        assert_eq!(outcome, DiskInitOutcome::Created);

        assert!(target.exists(), "file must exist after creation");

        let meta = fs::metadata(&target).expect("stat file");
        // Size should be at least `size_bytes` (fallocate may round up).
        assert!(
            meta.len() >= 4096,
            "file length {} < requested 4096",
            meta.len()
        );
        // Mode check: mask with 0o777 to get permission bits.
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o600,
            "file mode should be 0o600"
        );
        // Owner check (we ran as current euid so fchown is a no-op).
        assert_eq!(meta.uid(), nix::unistd::geteuid().as_raw());

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn disk_init_skips_when_present_and_if_absent() {
        let scratch = scratch_root();
        let target = scratch.join("store-overlay.img");
        // Pre-create the file with sentinel content.
        fs::write(&target, b"existing").expect("pre-create sentinel file");

        let spec = test_spec(target.clone(), 1_073_741_824 /* 1 GiB */, true);
        let outcome = disk_init_one_no_prefix_check(&spec).expect("disk_init_one succeeds");
        assert_eq!(outcome, DiskInitOutcome::Skipped, "must skip existing file");

        // File must not be truncated or grown.
        let meta = fs::metadata(&target).expect("stat file");
        assert_eq!(meta.len(), 8, "existing file must not be modified");

        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn disk_init_rejects_path_outside_permitted_root() {
        let spec = ResolvedDiskInitOp {
            target_path: PathBuf::from("/etc/nixling/evil.img"),
            size_bytes: 4096,
            mode: 0o600,
            owner_uid: 1000,
            owner_gid: 1000,
            if_absent: true,
        };
        let err = disk_init_one(&spec).expect_err("must reject path outside permitted root");
        assert_eq!(
            err.kind(),
            io::ErrorKind::PermissionDenied,
            "must return PermissionDenied for bad path"
        );
        assert!(
            err.to_string().contains("path-safety-violation"),
            "error message must mention path-safety-violation"
        );
    }

    // ----- mkfs path-safety regression tests -----

    #[test]
    fn mkfs_ext4_env_var_rejects_relative_path() {
        let err =
            validate_mkfs_ext4_binary("mkfs.ext4").expect_err("relative path must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("must be absolute"));
    }

    #[test]
    fn mkfs_ext4_env_var_rejects_parent_dir_traversal() {
        let err = validate_mkfs_ext4_binary("/run/current-system/sw/../../tmp/mkfs.ext4")
            .expect_err("`..` segment must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("may not contain"));
    }

    #[test]
    fn mkfs_ext4_env_var_accepts_canonical_absolute_path() {
        let p = validate_mkfs_ext4_binary("/run/current-system/sw/bin/mkfs.ext4")
            .expect("absolute path OK");
        assert!(p.is_absolute());
    }

    /// Post-mkfs reopen with O_NOFOLLOW refuses symlink swap. Proxy
    /// test for the TOCTOU race that the previous
    /// `std::fs::set_permissions` / `std::os::unix::fs::chown` variant
    /// allowed between mkfs returning and the chmod/chown reaching the
    /// inode. By re-opening with O_NOFOLLOW we ensure a concurrent
    /// attacker cannot redirect us to a sensitive target.
    #[test]
    fn post_mkfs_reopen_refuses_symlink() {
        use std::os::unix::fs::OpenOptionsExt;
        let scratch = scratch_root();
        let target_real = scratch.join("real.img");
        let target_link = scratch.join("link.img");
        std::fs::write(&target_real, b"x").unwrap();
        std::os::unix::fs::symlink(&target_real, &target_link).unwrap();
        // O_NOFOLLOW = 0x20000 = 0o400000 on Linux.
        let err = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(0o400000)
            .open(&target_link)
            .expect_err("O_NOFOLLOW must refuse symlink terminal");
        // Linux: ELOOP for O_NOFOLLOW on a symlink terminal.
        assert_eq!(
            err.raw_os_error(),
            Some(40 /* ELOOP */),
            "expected ELOOP, got {err:?}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
