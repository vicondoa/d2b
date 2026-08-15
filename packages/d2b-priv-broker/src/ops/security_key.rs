//! Broker op: `OpenHidrawSecurityKey`.
//!
//! Resolves a configured FIDO security-key stable selector, opens the
//! physical `hidraw` node, validates it is a character device with a
//! readable FIDO report descriptor, and returns an
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
//! - The selector is an opaque, bundle-resolved device token. The broker
//!   opens exactly that hidraw entry and never scans for the first matching
//!   device or grants blanket hidraw access.

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};

use d2b_core::host::SecurityKeySelector;
use nix::sys::stat::{SFlag, fstat};

use super::OpError;

/// FIDO HID usage page (0xF1D0), little-endian, as it appears inside a
/// HID report descriptor's usage-page item payload.
const FIDO_USAGE_PAGE_LE: &[u8] = &[0xD0, 0xF1];

/// Device-class label recorded in the audit trail and response body.
pub const DEVICE_CLASS_HIDRAW_FIDO: &str = "hidraw-fido";

/// A resolved stable selector → device-path mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSecurityKeySelector {
    /// Opaque selector label (no raw path) for audit/response.
    pub selector_label: String,
    /// Resolved absolute `/dev` path to the hidraw node. The path is derived
    /// from the trusted sysfs entry name and is never supplied by the caller.
    pub hidraw_path: PathBuf,
    /// True when the HID report descriptor was readable and explicitly matched
    /// the FIDO usage page.
    pub descriptor_verified: bool,
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
    selectors: &[SecurityKeySelector],
    _audit_log: &crate::audit::AuditLog,
) -> Result<LiveOpenHidrawSecurityKeyOutcome, OpError> {
    validate_device_authority(req)?;
    let resolved = resolve_selector(
        req.selector_id.as_str(),
        selectors,
        Path::new("/sys/class/hidraw"),
    )?;
    let fd = open_and_validate_hidraw(&resolved.hidraw_path, resolved.descriptor_verified)?;
    Ok(LiveOpenHidrawSecurityKeyOutcome {
        fd,
        selector_label: resolved.selector_label,
        device_class: DEVICE_CLASS_HIDRAW_FIDO.to_owned(),
    })
}

/// Require Core's exact Device and Host-backing proof before any node lookup.
pub(crate) fn validate_device_authority(
    req: &d2b_contracts::broker_wire::OpenHidrawSecurityKeyRequest,
) -> Result<(), OpError> {
    if req
        .device_ref
        .as_ref()
        .is_none_or(|reference| reference.resource_type().as_str() != "Device")
        || req
            .authority_key
            .as_deref()
            .is_none_or(|key| key.is_empty() || key.len() > 128)
    {
        return Err(OpError::Refused {
            operation: "OpenHidrawSecurityKey",
            reason: "device-authority-proof-required".to_owned(),
        });
    }
    Ok(())
}

/// Resolve a configured stable selector.
///
/// Raw `hidraw-N` identifiers are deliberately rejected. Resolution is
/// limited to the trusted vendor/product/serial selector registry.
pub(crate) fn resolve_selector(
    selector_id: &str,
    selectors: &[SecurityKeySelector],
    sysfs_root: &Path,
) -> Result<ResolvedSecurityKeySelector, OpError> {
    if selector_id.is_empty()
        || selector_id.len() > 63
        || !selector_id.bytes().enumerate().all(|(index, byte)| {
            (index == 0 && byte.is_ascii_lowercase())
                || (index > 0
                    && (byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
        })
        || selector_id.starts_with("hidraw-")
    {
        return Err(OpError::UnknownSubject {
            operation: "OpenHidrawSecurityKey",
            subject: selector_id.to_owned(),
        });
    }
    let Some(selector) = selectors
        .iter()
        .find(|selector| selector.selector_id == selector_id)
    else {
        return Err(OpError::UnknownSubject {
            operation: "OpenHidrawSecurityKey",
            subject: selector_id.to_owned(),
        });
    };

    let mut matches = Vec::new();
    let entries = std::fs::read_dir(sysfs_root).map_err(|error| OpError::Io {
        path: sysfs_root.to_owned(),
        detail: error.to_string(),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| OpError::Io {
            path: sysfs_root.to_owned(),
            detail: error.to_string(),
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("hidraw") {
            continue;
        }
        let path = entry.path();
        let Some((vendor_id, product_id, serial)) = hidraw_identity(&path) else {
            continue;
        };
        if vendor_id == selector.vendor_id
            && product_id == selector.product_id
            && selector
                .serial
                .as_deref()
                .is_none_or(|expected| serial.as_deref() == Some(expected))
            && fido_device_match(&path).is_some()
        {
            matches.push(PathBuf::from("/dev").join(name.as_ref()));
        }
    }

    let [hidraw_path] = matches.as_slice() else {
        return Err(OpError::UnknownSubject {
            operation: "OpenHidrawSecurityKey",
            subject: selector_id.to_owned(),
        });
    };
    Ok(ResolvedSecurityKeySelector {
        selector_label: selector.label.clone(),
        hidraw_path: hidraw_path.clone(),
        descriptor_verified: true,
    })
}

fn hidraw_identity(path: &Path) -> Option<(u16, u16, Option<String>)> {
    let mut current = std::fs::canonicalize(path.join("device")).ok()?;
    for _ in 0..8 {
        let vendor = read_hex_attr(&current.join("idVendor"));
        let product = read_hex_attr(&current.join("idProduct"));
        if let (Some(vendor), Some(product)) = (vendor, product) {
            let serial = std::fs::read_to_string(current.join("serial"))
                .ok()
                .map(|value| value.trim_end_matches(['\r', '\n']).to_owned());
            return Some((vendor, product, serial));
        }
        current = current.parent()?.to_owned();
    }
    None
}

fn read_hex_attr(path: &Path) -> Option<u16> {
    let value = std::fs::read_to_string(path).ok()?;
    let value = value.trim();
    u16::from_str_radix(value.strip_prefix("0x").unwrap_or(value), 16).ok()
}

/// Check whether a sysfs hidraw entry is a FIDO-class device.
#[cfg(test)]
fn is_fido_device(sysfs_entry: &Path) -> bool {
    fido_device_match(sysfs_entry).is_some()
}

fn fido_device_match(sysfs_entry: &Path) -> Option<bool> {
    let rdesc_path = sysfs_entry.join("device/report_descriptor");
    match std::fs::read(&rdesc_path) {
        Ok(rdesc) => rdesc
            .windows(FIDO_USAGE_PAGE_LE.len())
            .any(|w| w == FIDO_USAGE_PAGE_LE)
            .then_some(true),
        Err(_) => None,
    }
}

/// Open the hidraw node with pre- and post-open safety checks.
pub(crate) fn open_and_validate_hidraw(
    path: &Path,
    descriptor_verified: bool,
) -> Result<OwnedFd, OpError> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};

    // Pre-open check (defence-in-depth; O_NOFOLLOW below prevents a
    // symlink swap between this stat and the actual open).
    let meta = std::fs::symlink_metadata(path).map_err(|e| OpError::Io {
        path: path.to_owned(),
        detail: e.to_string(),
    })?;
    if !meta.file_type().is_char_device() {
        return Err(OpError::Refused {
            operation: "OpenHidrawSecurityKey",
            reason: format!(
                "{}: resolved path is not a character device",
                path.display()
            ),
        });
    }
    if !descriptor_verified {
        return Err(OpError::Refused {
            operation: "OpenHidrawSecurityKey",
            reason: "fido-report-descriptor-required".to_owned(),
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
            reason: format!(
                "{}: post-open stat is not a character device",
                path.display()
            ),
        });
    }

    Ok(fd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_selector_is_rejected_before_sysfs_lookup() {
        assert!(matches!(
            resolve_selector("test-selector", &[], Path::new("/sys/class/hidraw")),
            Err(OpError::UnknownSubject { subject, .. }) if subject == "test-selector"
        ));
    }

    #[test]
    fn configured_selector_reports_sysfs_io_errors() {
        let selector = SecurityKeySelector {
            selector_id: "test-selector".to_owned(),
            label: "test-selector".to_owned(),
            vendor_id: 0x1050,
            product_id: 0x0407,
            serial: None,
        };
        assert!(matches!(
            resolve_selector(
                "test-selector",
                std::slice::from_ref(&selector),
                Path::new("/does-not-exist"),
            ),
            Err(OpError::Io { .. })
        ));
    }

    #[test]
    fn raw_hidraw_selector_is_rejected() {
        assert!(matches!(
            resolve_selector("hidraw-0", &[], Path::new("/sys/class/hidraw")),
            Err(OpError::UnknownSubject { .. })
        ));
    }

    #[test]
    fn trusted_selector_resolves_one_fido_hidraw_and_refuses_ambiguity() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("hidraw");
        std::fs::create_dir_all(root.join("hidraw0/device")).expect("hidraw0");
        std::fs::write(root.join("hidraw0/device/idVendor"), "1050\n").expect("vendor");
        std::fs::write(root.join("hidraw0/device/idProduct"), "0407\n").expect("product");
        std::fs::write(
            root.join("hidraw0/device/report_descriptor"),
            [0x06, 0xD0, 0xF1],
        )
        .expect("descriptor");
        let selector = SecurityKeySelector {
            selector_id: "primary".to_owned(),
            label: "primary".to_owned(),
            vendor_id: 0x1050,
            product_id: 0x0407,
            serial: None,
        };
        let resolved =
            resolve_selector("primary", std::slice::from_ref(&selector), &root).expect("resolve");
        assert_eq!(resolved.selector_label, "primary");
        assert!(resolved.descriptor_verified);

        std::fs::create_dir_all(root.join("hidraw1/device")).expect("hidraw1");
        std::fs::write(root.join("hidraw1/device/idVendor"), "1050\n").expect("vendor");
        std::fs::write(root.join("hidraw1/device/idProduct"), "0407\n").expect("product");
        std::fs::write(
            root.join("hidraw1/device/report_descriptor"),
            [0x06, 0xD0, 0xF1],
        )
        .expect("descriptor");
        assert!(matches!(
            resolve_selector("primary", &[selector], &root),
            Err(OpError::UnknownSubject { .. })
        ));
    }

    #[test]
    fn missing_core_device_authority_refuses_before_lookup() {
        let request = d2b_contracts::broker_wire::OpenHidrawSecurityKeyRequest {
            vm_id: d2b_contracts::types::VmId::new("work-vm"),
            selector_id: "hidraw-0".to_owned(),
            device_ref: None,
            authority_key: None,
            tracing_span_id: None,
        };
        assert!(matches!(
            validate_device_authority(&request),
            Err(OpError::Refused { reason, .. }) if reason == "device-authority-proof-required"
        ));
    }

    #[test]
    fn is_fido_device_rejects_nonexistent_path() {
        assert!(!is_fido_device(Path::new("/nonexistent/hidraw-path")));
    }

    #[test]
    fn is_fido_device_rejects_readable_non_fido_descriptor_without_group_fallback() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let device_dir = tmp.path().join("hidraw0/device");
        std::fs::create_dir_all(&device_dir).expect("device dir");
        std::fs::write(
            device_dir.join("report_descriptor"),
            [0x00, 0x01, 0x02, 0x03],
        )
        .expect("write descriptor");

        assert!(
            !is_fido_device(&tmp.path().join("hidraw0")),
            "readable non-FIDO report descriptors must fail closed instead of falling through to group fallback"
        );
    }

    #[test]
    fn is_fido_device_accepts_readable_fido_descriptor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let device_dir = tmp.path().join("hidraw0/device");
        std::fs::create_dir_all(&device_dir).expect("device dir");
        std::fs::write(
            device_dir.join("report_descriptor"),
            [0x06, 0xD0, 0xF1, 0x09, 0x01],
        )
        .expect("write descriptor");

        assert!(is_fido_device(&tmp.path().join("hidraw0")));
        assert_eq!(fido_device_match(&tmp.path().join("hidraw0")), Some(true));
    }

    #[test]
    fn open_and_validate_hidraw_dev_null_fails_group_or_type_validation() {
        // /dev/null is a character device but is never owned by a FIDO
        // group, so it must be refused, not silently opened.
        match open_and_validate_hidraw(Path::new("/dev/null"), false) {
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
        match open_and_validate_hidraw(Path::new("/nonexistent/hidraw-path"), false) {
            Err(OpError::Io { .. }) => {}
            other => panic!("expected Io error for missing path, got {other:?}"),
        }
    }
}
