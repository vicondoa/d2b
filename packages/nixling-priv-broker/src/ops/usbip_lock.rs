//! Per-busid USBIP lock helper.
//!
//! Per `nixling_core::host::UsbipBusidLock`, every USBIP-capable VM
//! claims its busid via a daemon-owned exclusivity lock at
//! `/run/nixling/locks/usbip/<bus_id>`. The broker is the single
//! writer of that file; the daemon may read it for status display
//! but cannot mutate it. The lock file body records the owning VM
//! name so post-restart reconciliation can verify ownership without
//! re-running the bind.
//!
//! Lock semantics:
//!
//! - Acquire: `O_CREAT | O_EXCL | O_WRONLY`, file body is the owning
//!   VM name + newline. Returns `LockAlreadyHeld` if a *different* VM
//!   already owns the busid; the daemon must surface a typed refusal
//!   to the operator (USBIP single-owner is a v0.4.0 invariant).
//!   **Idempotent for the same VM**: if the lock already exists and
//!   the recorded owner matches the requesting VM, the acquire
//!   succeeds without modification. This covers VM restarts where
//!   `nixling down` does not release USBIP locks.
//! - Release: read the file, verify the recorded owner matches the
//!   expected vm name (defence-in-depth against a stale unbind),
//!   then unlink. Missing file is treated as success (idempotent
//!   unbind).

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum UsbipLockError {
    /// Lock already held; surfaced when another VM owns the busid.
    /// The body of the existing lock file is included so the broker
    /// audit row can name the conflicting owner.
    LockAlreadyHeld {
        path: PathBuf,
        existing_owner: String,
    },
    /// Release-time owner mismatch (stale unbind protection).
    OwnerMismatch {
        path: PathBuf,
        expected: String,
        observed: String,
    },
    /// Underlying I/O error (e.g. parent dir missing).
    Io { path: PathBuf, detail: String },
}

impl std::fmt::Display for UsbipLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LockAlreadyHeld {
                path,
                existing_owner,
            } => write!(
                f,
                "usbip busid lock {} already held by {}",
                path.display(),
                existing_owner
            ),
            Self::OwnerMismatch {
                path,
                expected,
                observed,
            } => write!(
                f,
                "usbip busid lock {} owner mismatch: expected {} but saw {}",
                path.display(),
                expected,
                observed
            ),
            Self::Io { path, detail } => write!(f, "usbip lock io {}: {}", path.display(), detail),
        }
    }
}

impl std::error::Error for UsbipLockError {}

/// Create the parent dir for a busid lock file.
pub fn ensure_lock_root(parent: &Path) -> Result<OwnedFd, UsbipLockError> {
    use rustix::fs::OFlags;

    let full_parent = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(parent))
            .map_err(|e| UsbipLockError::Io {
                path: parent.to_path_buf(),
                detail: e.to_string(),
            })?
    };
    let current_uid = nix::unistd::Uid::current().as_raw();
    let current_gid = nix::unistd::Gid::current().as_raw();
    let mut current_fd =
        crate::sys::path_safe::open_dir_path_safe(Path::new("/")).map_err(|e| {
            UsbipLockError::Io {
                path: PathBuf::from("/"),
                detail: e.to_string(),
            }
        })?;
    let mut saw_component = false;
    let mut components = full_parent.components().peekable();
    while let Some(component) = components.next() {
        match component {
            std::path::Component::RootDir | std::path::Component::CurDir => continue,
            std::path::Component::ParentDir => {
                return Err(UsbipLockError::Io {
                    path: full_parent.clone(),
                    detail: format!(
                        "path-safety-violation: lock root must not contain ..: {}",
                        full_parent.display()
                    ),
                });
            }
            std::path::Component::Normal(part) => {
                saw_component = true;
                let name = part.to_str().ok_or_else(|| UsbipLockError::Io {
                    path: full_parent.clone(),
                    detail: format!(
                        "lock root component is not valid UTF-8: {}",
                        full_parent.display()
                    ),
                })?;
                let is_final = components.peek().is_none();
                current_fd = match crate::sys::path_safe::open_at(
                    current_fd.as_fd(),
                    Path::new(name),
                    OFlags::RDONLY | OFlags::DIRECTORY,
                ) {
                    Ok(fd) => {
                        if is_final {
                            crate::sys::path_safe::fchmod(fd.as_fd(), 0o755).map_err(|e| {
                                UsbipLockError::Io {
                                    path: full_parent.clone(),
                                    detail: e.to_string(),
                                }
                            })?;
                        }
                        fd
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        crate::sys::path_safe::ensure_dir_path_safe(
                            &current_fd,
                            name,
                            0o755,
                            current_uid,
                            current_gid,
                        )
                        .map_err(|e| UsbipLockError::Io {
                            path: full_parent.clone(),
                            detail: e.to_string(),
                        })?
                    }
                    Err(err) => {
                        return Err(UsbipLockError::Io {
                            path: full_parent.clone(),
                            detail: err.to_string(),
                        });
                    }
                };
            }
            std::path::Component::Prefix(_) => unreachable!("unix paths never contain prefixes"),
        }
    }
    if !saw_component {
        return Err(UsbipLockError::Io {
            path: full_parent,
            detail: "path-safety-violation: lock root has no components".to_owned(),
        });
    }
    Ok(current_fd)
}

/// Acquire a per-busid lock; refuses if already held.
pub fn acquire_lock(
    lock_path: &Path,
    owner_vm: &str,
    daemon_uid: u32,
    daemon_gid: u32,
) -> Result<(), UsbipLockError> {
    let full_lock_path = resolve_lock_path(lock_path).map_err(|e| UsbipLockError::Io {
        path: lock_path.to_path_buf(),
        detail: e.to_string(),
    })?;
    let parent = full_lock_path.parent().ok_or_else(|| UsbipLockError::Io {
        path: full_lock_path.clone(),
        detail: format!(
            "path-safety-violation: {} has no parent",
            full_lock_path.display()
        ),
    })?;
    let parent_fd = ensure_lock_root(parent)?;
    let lock_name = lock_basename(&full_lock_path).map_err(|e| UsbipLockError::Io {
        path: full_lock_path.clone(),
        detail: e.to_string(),
    })?;
    match crate::sys::path_safe::create_file_at_safe(
        &parent_fd,
        &lock_name,
        nix::libc::O_RDWR | nix::libc::O_CREAT | nix::libc::O_EXCL,
        0o600,
    ) {
        Ok(fd) => {
            let mut f = File::from(fd);
            crate::sys::path_safe::fchmod(f.as_fd(), 0o640).map_err(|e| UsbipLockError::Io {
                path: full_lock_path.clone(),
                detail: e.to_string(),
            })?;
            crate::sys::path_safe::fchown(f.as_fd(), Some(daemon_uid), Some(daemon_gid)).map_err(
                |e| UsbipLockError::Io {
                    path: full_lock_path.clone(),
                    detail: e.to_string(),
                },
            )?;
            f.write_all(owner_vm.as_bytes())
                .map_err(|e| UsbipLockError::Io {
                    path: full_lock_path.clone(),
                    detail: e.to_string(),
                })?;
            f.write_all(b"\n").map_err(|e| UsbipLockError::Io {
                path: full_lock_path.clone(),
                detail: e.to_string(),
            })?;
            f.sync_all().map_err(|e| UsbipLockError::Io {
                path: full_lock_path.clone(),
                detail: e.to_string(),
            })?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing =
                read_owner(&full_lock_path).unwrap_or_else(|_| "<unreadable>".to_owned());
            // Idempotent: if the same VM already owns the lock (e.g.
            // after a VM restart without an explicit detach), treat the
            // acquire as a success rather than refusing.
            if existing.trim() == owner_vm {
                return Ok(());
            }
            Err(UsbipLockError::LockAlreadyHeld {
                path: full_lock_path,
                existing_owner: existing,
            })
        }
        Err(e) => Err(UsbipLockError::Io {
            path: full_lock_path,
            detail: e.to_string(),
        }),
    }
}

/// Release a per-busid lock; missing file is treated as success.
/// Owner mismatch is a typed error (defence-in-depth against a
/// stale unbind happening after a different VM rebound the same
/// busid).
pub fn release_lock(lock_path: &Path, expected_owner: &str) -> Result<(), UsbipLockError> {
    let full_lock_path = resolve_lock_path(lock_path).map_err(|e| UsbipLockError::Io {
        path: lock_path.to_path_buf(),
        detail: e.to_string(),
    })?;
    let (parent_fd, lock_name) =
        parent_fd_and_name(&full_lock_path).map_err(|e| UsbipLockError::Io {
            path: full_lock_path.clone(),
            detail: e.to_string(),
        })?;
    let observed = match read_owner_at(&parent_fd, &lock_name) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(UsbipLockError::Io {
                path: full_lock_path.clone(),
                detail: e.to_string(),
            });
        }
    };
    if observed != expected_owner {
        return Err(UsbipLockError::OwnerMismatch {
            path: full_lock_path,
            expected: expected_owner.to_owned(),
            observed,
        });
    }
    crate::sys::path_safe::remove_path_safe(&parent_fd, &lock_name)
        .or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(e)
            }
        })
        .map_err(|e| UsbipLockError::Io {
            path: full_lock_path.clone(),
            detail: e.to_string(),
        })
}

fn resolve_lock_path(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir().map(|cwd| cwd.join(path))
    }
}

fn parent_fd_and_name(path: &Path) -> std::io::Result<(std::os::fd::OwnedFd, String)> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path-safety-violation: {} has no parent", path.display()),
        )
    })?;
    let name = lock_basename(path)?;
    let parent_fd = open_existing_lock_parent(parent)?;
    Ok((parent_fd, name))
}

fn lock_basename(path: &Path) -> std::io::Result<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "path-safety-violation: {} has no valid basename",
                    path.display()
                ),
            )
        })
        .map(ToOwned::to_owned)
}

fn open_existing_lock_parent(parent: &Path) -> std::io::Result<OwnedFd> {
    let full_parent = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        std::env::current_dir()?.join(parent)
    };
    let mut current_fd = crate::sys::path_safe::open_dir_path_safe(Path::new("/"))?;
    let mut saw_component = false;
    for component in full_parent.components() {
        match component {
            std::path::Component::RootDir | std::path::Component::CurDir => continue,
            std::path::Component::ParentDir => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "path-safety-violation: lock root must not contain ..: {}",
                        full_parent.display()
                    ),
                ));
            }
            std::path::Component::Normal(part) => {
                saw_component = true;
                let name = part.to_str().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!(
                            "lock root component is not valid UTF-8: {}",
                            full_parent.display()
                        ),
                    )
                })?;
                current_fd = crate::sys::path_safe::open_at(
                    current_fd.as_fd(),
                    Path::new(name),
                    rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY,
                )?;
            }
            std::path::Component::Prefix(_) => unreachable!("unix paths never contain prefixes"),
        }
    }
    if !saw_component {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path-safety-violation: lock root has no components",
        ));
    }
    Ok(current_fd)
}

fn read_owner_at(parent_fd: &std::os::fd::OwnedFd, lock_name: &str) -> std::io::Result<String> {
    let fd = crate::sys::path_safe::open_at(
        parent_fd.as_fd(),
        Path::new(lock_name),
        rustix::fs::OFlags::RDONLY,
    )?;
    let mut raw = String::new();
    let mut file = File::from(fd);
    file.read_to_string(&mut raw)?;
    Ok(raw.trim().to_owned())
}

fn read_owner(path: &Path) -> std::io::Result<String> {
    let full_path = resolve_lock_path(path)?;
    let (parent_fd, lock_name) = parent_fd_and_name(&full_path)?;
    read_owner_at(&parent_fd, &lock_name)
}

/// Read the current owner of a busid lock without modifying it.
/// Used by reconcile / proxy-reconcile to verify expected ownership.
pub fn peek_owner(lock_path: &Path) -> Option<String> {
    read_owner(lock_path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use tempfile::TempDir;

    fn temp_lock_dir() -> TempDir {
        let base = std::env::var_os("CARGO_TARGET_TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
        fs::create_dir_all(&base).expect("create temp base");
        TempDir::new_in(base).expect("tempdir")
    }

    #[test]
    fn acquire_creates_lock_with_owner_body() {
        let tmp = temp_lock_dir();
        let lock = tmp.path().join("1-2");
        let uid = nix::unistd::Uid::current().as_raw();
        let gid = nix::unistd::Gid::current().as_raw();
        acquire_lock(&lock, "work-vm", uid, gid).unwrap();
        assert!(lock.exists());
        assert_eq!(peek_owner(&lock).unwrap(), "work-vm");
        let metadata = fs::symlink_metadata(&lock).expect("lock metadata");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        assert_eq!(metadata.uid(), uid);
        assert_eq!(metadata.gid(), gid);
    }

    #[test]
    fn acquire_refuses_when_lock_already_held() {
        let tmp = temp_lock_dir();
        let lock = tmp.path().join("2-3");
        let uid = nix::unistd::Uid::current().as_raw();
        let gid = nix::unistd::Gid::current().as_raw();
        acquire_lock(&lock, "vm-a", uid, gid).unwrap();
        let err = acquire_lock(&lock, "vm-b", uid, gid).unwrap_err();
        match err {
            UsbipLockError::LockAlreadyHeld { existing_owner, .. } => {
                assert_eq!(existing_owner, "vm-a")
            }
            other => panic!("expected LockAlreadyHeld, got {other:?}"),
        }
    }

    #[test]
    fn release_removes_lock_on_matching_owner() {
        let tmp = temp_lock_dir();
        let lock = tmp.path().join("3-1");
        let uid = nix::unistd::Uid::current().as_raw();
        let gid = nix::unistd::Gid::current().as_raw();
        acquire_lock(&lock, "vm-c", uid, gid).unwrap();
        release_lock(&lock, "vm-c").unwrap();
        assert!(!lock.exists());
    }

    #[test]
    fn release_refuses_on_owner_mismatch() {
        let tmp = temp_lock_dir();
        let lock = tmp.path().join("4-2");
        let uid = nix::unistd::Uid::current().as_raw();
        let gid = nix::unistd::Gid::current().as_raw();
        acquire_lock(&lock, "vm-d", uid, gid).unwrap();
        let err = release_lock(&lock, "vm-other").unwrap_err();
        assert!(matches!(err, UsbipLockError::OwnerMismatch { .. }));
        // Lock is preserved on mismatch.
        assert!(lock.exists());
    }

    #[test]
    fn acquire_refuses_symlink_parent() {
        let tmp = temp_lock_dir();
        let real = tmp.path().join("real");
        fs::create_dir_all(&real).expect("real dir");
        let link = tmp.path().join("link");
        symlink(&real, &link).expect("symlink parent");
        let uid = nix::unistd::Uid::current().as_raw();
        let gid = nix::unistd::Gid::current().as_raw();
        let err = acquire_lock(&link.join("5-5"), "vm-z", uid, gid).unwrap_err();
        assert!(matches!(err, UsbipLockError::Io { .. }));
    }

    #[test]
    fn acquire_idempotent_when_same_vm_owns_lock() {
        let tmp = temp_lock_dir();
        let lock = tmp.path().join("6-1");
        let uid = nix::unistd::Uid::current().as_raw();
        let gid = nix::unistd::Gid::current().as_raw();
        acquire_lock(&lock, "work-aad", uid, gid).unwrap();
        // Re-acquire by the same VM succeeds (e.g. after VM restart).
        acquire_lock(&lock, "work-aad", uid, gid).unwrap();
        assert_eq!(peek_owner(&lock).unwrap(), "work-aad");
    }

    #[test]
    fn release_idempotent_when_missing() {
        let tmp = temp_lock_dir();
        let lock = tmp.path().join("never-existed");
        release_lock(&lock, "vm-x").expect("missing lock release is no-op");
    }
}
