//! Broker op: `OpenHidrawSecurityKey`.
//!
//! Resolves a configured FIDO security-key stable selector, opens the
//! physical `hidraw` node, validates it is a character device owned by
//! an acceptable group (`plugdev`/`input`/`fido`), and returns an
//! `OwnedFd` to be passed to `d2bd` via `SCM_RIGHTS`. Long-lived
//! CTAPHID session state (CID isolation, lease serialization, relay)
//! lives in `d2bd::security_key`, not here. This module only opens the
//! device and hands off the fd.
//!
//! Security notes:
//! - Raw hidraw paths never cross the broker wire; the daemon supplies
//!   only an opaque `selector_id`.
//! - The broker opens the node `O_RDWR | O_NONBLOCK | O_NOFOLLOW` so
//!   no symlink can be substituted after the path safety check, and a
//!   post-open `fstat` re-confirms the character-device type.
//! - In this initial implementation the selector is resolved by
//!   scanning `/sys/class/hidraw/` for the first FIDO-class device
//!   (report descriptor contains the 0xF1D0 usage page, with a
//!   group-ownership fallback for kernels that restrict `rdesc`
//!   reads). When the contracts/manifest workstream lands per-host
//!   security-key bundle entries, this function will look up
//!   `selector_id` against the resolver's registry instead.

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};

use nix::sys::stat::{SFlag, fstat};

use super::OpError;

/// FIDO HID usage page (0xF1D0), little-endian, as it appears inside a
/// HID report descriptor's usage-page item payload.
const FIDO_USAGE_PAGE_LE: &[u8] = &[0xD0, 0xF1];

/// Groups that may own a FIDO hidraw node. `plugdev` is the typical
/// libfido2 udev rule target; `input`/`fido` cover other distros.
const ALLOWED_GROUPS: &[&str] = &["plugdev", "input", "fido"];

/// Device-class label recorded in the audit trail and response body.
pub const DEVICE_CLASS_HIDRAW_FIDO: &str = "hidraw-fido";

/// A resolved stable selector → device-path mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSecurityKeySelector {
    /// Opaque selector label (no raw path) for audit/response.
    pub selector_label: String,
    /// Resolved absolute path to the hidraw node.
    pub hidraw_path: PathBuf,
}

/// Outcome of a live `OpenHidrawSecurityKey` op.
#[derive(Debug)]
pub struct LiveOpenHidrawSecurityKeyOutcome {
    pub fd: OwnedFd,
    pub selector_label: String,
    pub device_class: String,
}

/// Resolve `req.selector_id`, open the physical hidraw node, and
/// validate it. Returns the fd plus scrubbed metadata for the audit
/// record and wire response.
pub fn live_open_hidraw_security_key(
    req: &d2b_contracts::broker_wire::OpenHidrawSecurityKeyRequest,
    _audit_log: &crate::audit::AuditLog,
) -> Result<LiveOpenHidrawSecurityKeyOutcome, OpError> {
    let resolved = resolve_selector(req.selector_id.as_str())?;
    let fd = open_and_validate_hidraw(&resolved.hidraw_path)?;
    Ok(LiveOpenHidrawSecurityKeyOutcome {
        fd,
        selector_label: resolved.selector_label,
        device_class: DEVICE_CLASS_HIDRAW_FIDO.to_owned(),
    })
}

/// Scan `/sys/class/hidraw/` for FIDO-class devices.
///
/// Returns the first device whose report descriptor contains the FIDO
/// HID usage page, falling back to group ownership when the report
/// descriptor can't be read (some kernels restrict `rdesc` to root).
pub(crate) fn resolve_selector(
    selector_id: &str,
) -> Result<ResolvedSecurityKeySelector, OpError> {
    let sysfs_hidraw = Path::new("/sys/class/hidraw");
    let entries = std::fs::read_dir(sysfs_hidraw).map_err(|e| OpError::Io {
        path: sysfs_hidraw.to_owned(),
        detail: e.to_string(),
    })?;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("hidraw") {
            continue;
        }
        let dev_path = PathBuf::from("/dev").join(&*name_str);
        let sysfs_path = entry.path();

        if is_fido_device(&sysfs_path) {
            return Ok(ResolvedSecurityKeySelector {
                // Stable selector label combines the caller-supplied
                // opaque id with the resolved sysfs index; no raw
                // path leaks into audit/response fields.
                selector_label: format!("{selector_id}:{name_str}"),
                hidraw_path: dev_path,
            });
        }
    }

    Err(OpError::UnknownSubject {
        operation: "OpenHidrawSecurityKey",
        subject: selector_id.to_owned(),
    })
}

/// Check whether a sysfs hidraw entry is a FIDO-class device.
fn is_fido_device(sysfs_entry: &Path) -> bool {
    let rdesc_path = sysfs_entry.join("device/report_descriptor");
    if let Ok(rdesc) = std::fs::read(&rdesc_path)
        && rdesc
            .windows(FIDO_USAGE_PAGE_LE.len())
            .any(|w| w == FIDO_USAGE_PAGE_LE)
    {
        return true;
    }
    // Fallback: accept if the /dev node is owned by an allowed group.
    let dev_name = sysfs_entry.file_name().unwrap_or_default();
    let dev_path = PathBuf::from("/dev").join(dev_name);
    let Ok(meta) = std::fs::metadata(&dev_path) else {
        return false;
    };
    use std::os::unix::fs::MetadataExt;
    let gid = meta.gid();
    match nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid)) {
        Ok(Some(group)) => ALLOWED_GROUPS.contains(&group.name.as_str()),
        _ => false,
    }
}

/// Open the hidraw node with pre- and post-open safety checks.
pub(crate) fn open_and_validate_hidraw(path: &Path) -> Result<OwnedFd, OpError> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};

    // Pre-open check (defence-in-depth; O_NOFOLLOW below prevents a
    // symlink swap between this stat and the actual open).
    let meta = std::fs::symlink_metadata(path).map_err(|e| OpError::Io {
        path: path.to_owned(),
        detail: e.to_string(),
    })?;
    if !meta.file_type().is_char_device() {
        return Err(OpError::Refused {
            operation: "OpenHidrawSecurityKey",
            reason: format!("{}: resolved path is not a character device", path.display()),
        });
    }
    let gid = meta.gid();
    let group_name = nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid))
        .ok()
        .flatten()
        .map(|g| g.name)
        .unwrap_or_else(|| gid.to_string());
    if !ALLOWED_GROUPS.contains(&group_name.as_str()) {
        return Err(OpError::Refused {
            operation: "OpenHidrawSecurityKey",
            reason: format!(
                "{}: device group {group_name:?} not in allowed set",
                path.display()
            ),
        });
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_NONBLOCK | nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| OpError::Io {
            path: path.to_owned(),
            detail: e.to_string(),
        })?;
    let fd = OwnedFd::from(file);

    // Post-open re-check: confirm we really opened a character device
    // (guards against a raced symlink swap between the pre-open stat
    // and the open() call above).
    let stat = fstat(fd.as_raw_fd()).map_err(|e| OpError::Io {
        path: path.to_owned(),
        detail: e.to_string(),
    })?;
    if !SFlag::from_bits_truncate(stat.st_mode).contains(SFlag::S_IFCHR) {
        return Err(OpError::Refused {
            operation: "OpenHidrawSecurityKey",
            reason: format!("{}: post-open stat is not a character device", path.display()),
        });
    }

    Ok(fd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_not_found_or_sysfs_absent_is_safe() {
        // In a hermetic test environment /sys/class/hidraw may be
        // absent or contain no FIDO devices; the resolver must return
        // a typed error, never panic.
        match resolve_selector("test-selector") {
            Err(OpError::UnknownSubject { subject, .. }) => {
                assert_eq!(subject, "test-selector");
            }
            Err(OpError::Io { .. }) => {
                // /sys/class/hidraw absent in this sandbox: acceptable.
            }
            Ok(_) => {
                // A real FIDO device happened to be present; fine too.
            }
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn is_fido_device_rejects_nonexistent_path() {
        assert!(!is_fido_device(Path::new("/nonexistent/hidraw-path")));
    }

    #[test]
    fn open_and_validate_hidraw_dev_null_fails_group_or_type_validation() {
        // /dev/null is a character device but is never owned by a FIDO
        // group, so it must be refused, not silently opened.
        match open_and_validate_hidraw(Path::new("/dev/null")) {
            Err(OpError::Refused { .. }) => {}
            Err(OpError::Io { .. }) => {
                // Sandboxed environments without /dev/null access also
                // fail closed acceptably.
            }
            Ok(_) => panic!("/dev/null must not pass FIDO hidraw validation"),
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn open_and_validate_hidraw_missing_path_is_io_error() {
        match open_and_validate_hidraw(Path::new("/nonexistent/hidraw-path")) {
            Err(OpError::Io { .. }) => {}
            other => panic!("expected Io error for missing path, got {other:?}"),
        }
    }
}
