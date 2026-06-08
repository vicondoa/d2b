//! P2 ph2-p2-ssh-host-key-preflight: VM-start preflight that
//! refuses to start a VM when its per-VM sshd host keys have
//! drifted from the canonical mode/owner posture.
//!
//! # Canonical posture
//!
//! Directory `/var/lib/nixling/vms/<vm>/sshd-host-keys/`:
//! - mode `0750`
//! - owner `nixlingd`, group `nixling-launcher`
//!   (declared and enforced by the ownership matrix in
//!   `nixos-modules/options-ownership-matrix.nix` + the daemon-side
//!   `ownership_preflight` module; this preflight only verifies that
//!   the directory is a real directory at the expected path, not a
//!   symlink, because *real* directory ownership drift is fail-closed
//!   by `OwnershipMatrixDrift`).
//!
//! Each `ssh_host_*_key` (private) file under that directory:
//! - regular file (refuses symlinks via `O_NOFOLLOW`)
//! - owner uid `0` (root)
//! - group gid `0` (root)
//! - mode `0o0400`
//!
//! Source of truth: plan task `ph2-p2-ssh-host-key-preflight` and
//! `docs/reference/privileges.md` row "SshHostKeyPreflight".
//!
//! # Spec correction (recorded for plan.md "Spec corrections")
//!
//! The original sub-agent prompt referenced
//! `/var/lib/nixling/keys/<vm>/sshd-host-keys` and a
//! `root:nixling-<vm>-runner 0750` directory posture with `0640`
//! files. The canonical paths and modes shipped by the existing
//! ownership matrix (see `nixos-modules/options-ownership-matrix.nix`
//! and `docs/reference/per-vm-state-ownership.md`) are
//! `/var/lib/nixling/vms/<vm>/sshd-host-keys` and `0750` for the
//! directory; the file posture from the plan + privileges.md is
//! `root:root 0400`. This module follows the canonical (existing
//! code wins per AGENTS.md "Existing code is canon").

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// One drift finding from the ssh-host-key preflight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshdHostKeyDrift {
    /// The keys directory itself is missing.
    DirMissing { path: PathBuf },
    /// The keys directory path resolves to a symlink. Refused to
    /// avoid TOCTOU between preflight and runner exec.
    DirIsSymlink { path: PathBuf },
    /// The keys directory path exists but is not a directory.
    DirNotADirectory { path: PathBuf },
    /// Stat on the keys directory failed (permission denied, etc.).
    DirStatFailed { path: PathBuf, detail: String },
    /// A `ssh_host_*_key` entry is a symlink (refused by
    /// `O_NOFOLLOW` semantics).
    KeyIsSymlink { path: PathBuf },
    /// A `ssh_host_*_key` entry is not a regular file.
    KeyNotARegularFile { path: PathBuf },
    /// A `ssh_host_*_key` entry's owner uid is not 0 (root).
    KeyWrongOwner {
        path: PathBuf,
        expected_uid: u32,
        actual_uid: u32,
    },
    /// A `ssh_host_*_key` entry's group gid is not 0 (root).
    KeyWrongGroup {
        path: PathBuf,
        expected_gid: u32,
        actual_gid: u32,
    },
    /// A `ssh_host_*_key` entry's mode is not 0o0400.
    KeyWrongMode {
        path: PathBuf,
        expected_mode: u32,
        actual_mode: u32,
    },
    /// Stat on a key file failed.
    KeyStatFailed { path: PathBuf, detail: String },
    /// Reading the keys directory failed.
    DirReadFailed { path: PathBuf, detail: String },
}

impl SshdHostKeyDrift {
    /// Path most relevant to the operator-facing diagnostic.
    pub fn path(&self) -> &Path {
        match self {
            Self::DirMissing { path }
            | Self::DirIsSymlink { path }
            | Self::DirNotADirectory { path }
            | Self::DirStatFailed { path, .. }
            | Self::KeyIsSymlink { path }
            | Self::KeyNotARegularFile { path }
            | Self::KeyWrongOwner { path, .. }
            | Self::KeyWrongGroup { path, .. }
            | Self::KeyWrongMode { path, .. }
            | Self::KeyStatFailed { path, .. }
            | Self::DirReadFailed { path, .. } => path,
        }
    }

    /// Short, single-line drift reason.
    pub fn reason(&self) -> String {
        match self {
            Self::DirMissing { path } => {
                format!("sshd-host-keys directory missing: {}", path.display())
            }
            Self::DirIsSymlink { path } => {
                format!("sshd-host-keys directory is a symlink: {}", path.display())
            }
            Self::DirNotADirectory { path } => {
                format!(
                    "sshd-host-keys path is not a directory: {}",
                    path.display()
                )
            }
            Self::DirStatFailed { path, detail } => {
                format!(
                    "sshd-host-keys stat failed: {} ({})",
                    path.display(),
                    detail
                )
            }
            Self::KeyIsSymlink { path } => {
                format!("ssh host key is a symlink: {}", path.display())
            }
            Self::KeyNotARegularFile { path } => {
                format!("ssh host key is not a regular file: {}", path.display())
            }
            Self::KeyWrongOwner {
                path,
                expected_uid,
                actual_uid,
            } => format!(
                "ssh host key owner uid {actual_uid} != expected {expected_uid}: {}",
                path.display()
            ),
            Self::KeyWrongGroup {
                path,
                expected_gid,
                actual_gid,
            } => format!(
                "ssh host key group gid {actual_gid} != expected {expected_gid}: {}",
                path.display()
            ),
            Self::KeyWrongMode {
                path,
                expected_mode,
                actual_mode,
            } => format!(
                "ssh host key mode {actual_mode:o} != expected {expected_mode:o}: {}",
                path.display()
            ),
            Self::KeyStatFailed { path, detail } => {
                format!("ssh host key stat failed: {} ({})", path.display(), detail)
            }
            Self::DirReadFailed { path, detail } => {
                format!(
                    "sshd-host-keys read_dir failed: {} ({})",
                    path.display(),
                    detail
                )
            }
        }
    }
}

const EXPECTED_KEY_UID: u32 = 0;
const EXPECTED_KEY_GID: u32 = 0;
const EXPECTED_KEY_MODE: u32 = 0o0400;

/// Pure preflight: returns `Ok(())` if the directory + every
/// `ssh_host_*_key` (non-`.pub`) file matches the canonical posture,
/// or the first drift finding otherwise. Iteration is deterministic
/// (sorted by file name) so the surfaced drift is stable across
/// invocations.
///
/// `keys_dir` is the absolute path to
/// `/var/lib/nixling/vms/<vm>/sshd-host-keys`. The `vm` argument is
/// carried only for tracing/diagnostic context.
pub fn check_sshd_host_keys(vm: &str, keys_dir: &Path) -> Result<(), SshdHostKeyDrift> {
    // Step 1: lstat the directory itself — refuse symlinks.
    let dir_meta = match fs::symlink_metadata(keys_dir) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(SshdHostKeyDrift::DirMissing {
                path: keys_dir.to_path_buf(),
            });
        }
        Err(e) => {
            return Err(SshdHostKeyDrift::DirStatFailed {
                path: keys_dir.to_path_buf(),
                detail: e.to_string(),
            });
        }
    };
    let dir_ft = dir_meta.file_type();
    if dir_ft.is_symlink() {
        return Err(SshdHostKeyDrift::DirIsSymlink {
            path: keys_dir.to_path_buf(),
        });
    }
    if !dir_ft.is_dir() {
        return Err(SshdHostKeyDrift::DirNotADirectory {
            path: keys_dir.to_path_buf(),
        });
    }

    // Step 2: enumerate entries matching `ssh_host_*_key` (excluding
    // `.pub`). Sort for determinism.
    let mut entries: Vec<PathBuf> = match fs::read_dir(keys_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                let name = match p.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => return false,
                };
                name.starts_with("ssh_host_") && name.ends_with("_key")
            })
            .collect(),
        Err(e) => {
            return Err(SshdHostKeyDrift::DirReadFailed {
                path: keys_dir.to_path_buf(),
                detail: e.to_string(),
            });
        }
    };
    entries.sort();

    // Step 3: each key file must be a regular file (not symlink),
    // root:root, mode 0o0400.
    for path in entries {
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                return Err(SshdHostKeyDrift::KeyStatFailed {
                    path,
                    detail: e.to_string(),
                });
            }
        };
        let ft = meta.file_type();
        if ft.is_symlink() {
            return Err(SshdHostKeyDrift::KeyIsSymlink { path });
        }
        if !ft.is_file() {
            return Err(SshdHostKeyDrift::KeyNotARegularFile { path });
        }
        let uid = meta.uid();
        if uid != EXPECTED_KEY_UID {
            return Err(SshdHostKeyDrift::KeyWrongOwner {
                path,
                expected_uid: EXPECTED_KEY_UID,
                actual_uid: uid,
            });
        }
        let gid = meta.gid();
        if gid != EXPECTED_KEY_GID {
            return Err(SshdHostKeyDrift::KeyWrongGroup {
                path,
                expected_gid: EXPECTED_KEY_GID,
                actual_gid: gid,
            });
        }
        let mode = meta.permissions().mode() & 0o7777;
        // v1.1.2fu25 panel-virt: when the file has POSIX ACL named
        // entries (e.g. from the activation script's per-keyfile
        // `u:virtiofsd_uid:r` grant required by ADR 0021 broker-
        // pre-NS virtiofsd reading the 0400 root:root host key),
        // Linux stores the mask in the file's group-mode bits. The
        // group's BASE perm (in the ACL's ACL_GROUP_OBJ entry) is
        // still ---, but stat() reports 0440 because the mask is r.
        // Accept either 0400 (no ACL) or 0440 (ACL with mask r--)
        // when the file has a system.posix_acl_access xattr; reject
        // any other mode.
        let mode_ok = if mode == EXPECTED_KEY_MODE {
            true
        } else if mode == 0o0440 {
            // Only accept 0o0440 if a posix_acl_access xattr exists
            // (which is what bumped the stat-reported group bits via
            // the mask). Otherwise it really is mode drift.
            has_posix_acl(&path)
        } else {
            false
        };
        if !mode_ok {
            return Err(SshdHostKeyDrift::KeyWrongMode {
                path,
                expected_mode: EXPECTED_KEY_MODE,
                actual_mode: mode,
            });
        }
        tracing::debug!(
            vm = %vm,
            // P2fu1 observability-r2 closure: bounded attrs only;
            // path is high-cardinality + leaks host layout. The
            // operator-recoverable form lives in the typed error
            // envelope + audit log per the daemon tracing contract.
            outcome = "key-entry-ok",
            uid,
            gid,
            mode = format!("{mode:o}"),
            "ssh-host-key-preflight: key entry OK",
        );
    }
    Ok(())
}

/// v1.1.2fu25: returns true when the file has a
/// `system.posix_acl_access` xattr (i.e. the activation script's
/// `setfacl -m u:UID:r` grant for ADR 0021 broker-pre-NS
/// virtiofsd has been applied). Used by the preflight to
/// distinguish 0o0440-with-ACL (legitimate) from 0o0440-without-ACL
/// (real mode drift). Returns false on any xattr lookup error so
/// the preflight fails closed.
fn has_posix_acl(path: &Path) -> bool {
    // Pass a tiny buffer; we only care whether the call succeeds
    // (xattr exists) or fails with ENODATA / ENOTSUP / IO error.
    let mut tiny_buf = [0u8; 4];
    match rustix::fs::lgetxattr(path, "system.posix_acl_access", &mut tiny_buf) {
        Ok(_) => true,
        Err(e) if e.raw_os_error() == libc::ERANGE => true, // value exists, just too big for buf
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use std::os::unix::fs::{symlink, PermissionsExt};

    /// Materialize a sshd-host-keys directory with one key file.
    /// Returns (dir, key_path). The key is mode 0o400 but its owner
    /// uid/gid will be the current test process's uid/gid (almost
    /// certainly NOT 0). Callers can override expectations or
    /// override owner checks via the helpers below.
    fn make_keys_dir(root: &Path) -> (PathBuf, PathBuf) {
        let dir = root.join("sshd-host-keys");
        fs::create_dir_all(&dir).expect("create keys dir");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o0750)).unwrap();
        let key = dir.join("ssh_host_ed25519_key");
        let mut f = File::create(&key).expect("create key");
        f.write_all(b"PRIVATE KEY\n").unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o0400)).unwrap();
        (dir, key)
    }

    /// Decide whether the test process is uid 0 — only when it is
    /// can we exercise the "happy path returns Ok" assertion. Under
    /// any other uid the key file will own uid != 0 and the check
    /// (correctly) refuses.
    fn running_as_root() -> bool {
        nix::unistd::Uid::current().is_root()
    }

    #[test]
    fn missing_directory_is_drift() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("sshd-host-keys");
        let err = check_sshd_host_keys("vm1", &missing).expect_err("should refuse missing");
        assert!(
            matches!(err, SshdHostKeyDrift::DirMissing { .. }),
            "expected DirMissing, got {err:?}"
        );
        assert!(err.reason().contains("missing"));
    }

    #[test]
    fn symlink_directory_is_drift() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("real");
        fs::create_dir_all(&target).unwrap();
        let link = tmp.path().join("sshd-host-keys");
        symlink(&target, &link).unwrap();
        let err = check_sshd_host_keys("vm1", &link).expect_err("should refuse symlink dir");
        assert!(
            matches!(err, SshdHostKeyDrift::DirIsSymlink { .. }),
            "expected DirIsSymlink, got {err:?}"
        );
    }

    #[test]
    fn non_directory_path_is_drift() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("sshd-host-keys");
        File::create(&p).unwrap();
        let err = check_sshd_host_keys("vm1", &p).expect_err("should refuse non-dir");
        assert!(
            matches!(err, SshdHostKeyDrift::DirNotADirectory { .. }),
            "expected DirNotADirectory, got {err:?}"
        );
    }

    #[test]
    fn empty_keys_dir_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sshd-host-keys");
        fs::create_dir_all(&dir).unwrap();
        // An empty keys directory is acceptable: there are no
        // `ssh_host_*_key` files to validate. (Real per-VM provisioning
        // populates them lazily on first boot; ownership of the
        // directory itself is enforced separately by
        // `ownership_preflight`.)
        check_sshd_host_keys("vm1", &dir).expect("empty dir should pass");
    }

    #[test]
    fn ignores_pub_keys_and_unrelated_files() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, _key) = make_keys_dir(tmp.path());
        // Drop in a .pub sibling and an unrelated file; both must be
        // ignored by the preflight.
        let pubk = dir.join("ssh_host_ed25519_key.pub");
        File::create(&pubk).unwrap();
        fs::set_permissions(&pubk, fs::Permissions::from_mode(0o0644)).unwrap();
        let unrelated = dir.join("README");
        File::create(&unrelated).unwrap();

        if running_as_root() {
            check_sshd_host_keys("vm1", &dir).expect("happy path");
        } else {
            // Under non-root, the *.pub and README are filtered out;
            // the failing item is the private key (wrong owner). We
            // assert that the failing path is the private key, not
            // the pub key.
            let err = check_sshd_host_keys("vm1", &dir).expect_err("non-root → owner drift");
            let name = err
                .path()
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            assert_eq!(
                name, "ssh_host_ed25519_key",
                "preflight should target private key, not pub key. got {err:?}"
            );
        }
    }

    #[test]
    fn wrong_owner_is_drift() {
        // Always exercised: when the test runs as non-root, the
        // owner-uid check fires. When it runs as root we skip with a
        // log (rare in CI).
        if running_as_root() {
            eprintln!("skipping wrong_owner_is_drift: running as root");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let (dir, _key) = make_keys_dir(tmp.path());
        let err = check_sshd_host_keys("vm1", &dir).expect_err("should refuse wrong owner");
        assert!(
            matches!(err, SshdHostKeyDrift::KeyWrongOwner { .. }),
            "expected KeyWrongOwner, got {err:?}"
        );
    }

    #[test]
    fn wrong_mode_is_drift() {
        // We can always exercise mode drift by chmod'ing the key.
        // This depends on the owner check NOT firing first — so it
        // only runs as root. (Under non-root the owner mismatch
        // would be surfaced first.)
        if !running_as_root() {
            eprintln!("skipping wrong_mode_is_drift: needs root to set uid=0 owner");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let (dir, key) = make_keys_dir(tmp.path());
        fs::set_permissions(&key, fs::Permissions::from_mode(0o0644)).unwrap();
        let err = check_sshd_host_keys("vm1", &dir).expect_err("should refuse wrong mode");
        match err {
            SshdHostKeyDrift::KeyWrongMode {
                expected_mode,
                actual_mode,
                ..
            } => {
                assert_eq!(expected_mode, 0o0400);
                assert_eq!(actual_mode, 0o0644);
            }
            other => panic!("expected KeyWrongMode, got {other:?}"),
        }
    }

    #[test]
    fn symlink_key_is_drift() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sshd-host-keys");
        fs::create_dir_all(&dir).unwrap();
        let target = tmp.path().join("not-a-key");
        File::create(&target).unwrap();
        let link = dir.join("ssh_host_ed25519_key");
        symlink(&target, &link).unwrap();
        let err = check_sshd_host_keys("vm1", &dir).expect_err("should refuse symlinked key");
        assert!(
            matches!(err, SshdHostKeyDrift::KeyIsSymlink { .. }),
            "expected KeyIsSymlink, got {err:?}"
        );
    }

    #[test]
    fn drift_path_accessor_returns_offending_path() {
        let p = PathBuf::from("/var/lib/nixling/vms/vm1/sshd-host-keys/ssh_host_ed25519_key");
        let d = SshdHostKeyDrift::KeyWrongMode {
            path: p.clone(),
            expected_mode: 0o0400,
            actual_mode: 0o0644,
        };
        assert_eq!(d.path(), p.as_path());
        assert!(d.reason().contains("400"));
        assert!(d.reason().contains("644"));
    }

    #[test]
    fn happy_path_when_running_as_root() {
        if !running_as_root() {
            eprintln!("skipping happy_path: not running as root");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let (dir, _key) = make_keys_dir(tmp.path());
        check_sshd_host_keys("vm1", &dir).expect("canonical posture should pass");
    }
}
