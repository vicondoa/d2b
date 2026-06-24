//! Shared USBIP wire validation helpers and Phase 4 explicit-attach DTOs.
//!
//! Bus IDs cross the CLI, daemon, broker, and guest-control boundary. Keep the
//! shape check here so every layer rejects the same traversal-/shell-unsafe
//! strings before they reach a subprocess argv.
//!
//! # Phase 4 explicit-attach model
//!
//! `nixling usb attach <vm> <present-busid> --apply` allows any present USB
//! device to a USB-capable VM without requiring static busid/vendor allowlists
//! in the bundle. The claim source distinguishes this explicit path from the
//! legacy declared path so the daemon can apply the correct broker ops and the
//! broker can record the correct audit shape.
//!
//! The `UsbipDaemonClaimRecord` is the in-process DTO the daemon builds and
//! passes to broker dispatch. It does NOT cross the public wire; it is an
//! internal handoff from the daemon's USB handler to the broker op selector.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusIdError {
    Empty,
    Invalid,
    TooLong { max: usize },
}

/// SYSFS_BUS_ID_SIZE per `include/linux/mod_devicetable.h` and
/// `tools/usb/usbip/libsrc/usbip_common.h`: 32 bytes including the trailing NUL,
/// so the printable busid is at most 31 chars.
pub const SYSFS_BUS_ID_MAX: usize = 31;

/// Normalize a USB `idVendor` / `idProduct` string for redacted status or audit
/// projections. Only exact four-hex descriptors survive; malformed or
/// device-private strings collapse to `None`.
pub fn sanitize_usb_hex_id(value: Option<&str>) -> Option<String> {
    let value = value?;
    if value.len() == 4 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(value.to_ascii_lowercase())
    } else {
        None
    }
}

/// Render a kernel-read USB vendor/product integer as the canonical four-hex
/// descriptor used in privileged audit records.
pub fn format_usb_hex_id(value: u16) -> String {
    format!("{value:04x}")
}

/// Validate a USB bus id shape. Accepted forms:
///
/// - `B` (root hub bus, rare): digits, no leading zeros except `0`.
/// - `B-P` (port on root hub): digits-dash-digits.
/// - `B-P.S[.S...]` (port on chained hub): digits-dash-digits.dots.
///
/// ASCII digits only, no empty segments, no leading zeros, and no metacharacters.
pub fn validate_bus_id(bus_id: &str) -> Result<(), BusIdError> {
    if bus_id.is_empty() {
        return Err(BusIdError::Empty);
    }
    if bus_id.len() > SYSFS_BUS_ID_MAX {
        return Err(BusIdError::TooLong {
            max: SYSFS_BUS_ID_MAX,
        });
    }

    fn segment_ok(segment: &str) -> bool {
        !segment.is_empty()
            && segment.chars().all(|c| c.is_ascii_digit())
            && !(segment.len() > 1 && segment.starts_with('0'))
    }

    match bus_id.split_once('-') {
        None if segment_ok(bus_id) => Ok(()),
        None => Err(BusIdError::Invalid),
        Some((bus, port_chain)) if segment_ok(bus) && !port_chain.is_empty() => {
            if port_chain.split('.').all(segment_ok) {
                Ok(())
            } else {
                Err(BusIdError::Invalid)
            }
        }
        Some(_) => Err(BusIdError::Invalid),
    }
}

// ---- Phase 4 explicit-attach claim DTOs ----

/// Whether an active daemon USB claim originated from a static bundle declaration
/// or from an explicit `nixling usb attach <vm> <busid> --apply` invocation.
///
/// `Declared` claims have bundle-resolved firewall and bind intent refs that the
/// broker validates against the trusted bundle. `Explicit` claims carry only a
/// daemon-validated sysfs busid and a USB-capable VM name; the broker uses
/// `UsbipExplicitBind` / `UsbipExplicitFirewallRule` ops instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "source")]
pub enum UsbipClaimSource {
    /// Originated from a static bundle declaration: both a firewall intent ref
    /// and a bind intent ref exist in the trusted bundle.
    Declared {
        /// Opaque bundle firewall intent id, e.g. `usbip-fw-work-1-2`.
        firewall_ref: String,
        /// Opaque bundle bind intent id, e.g. `usbip-bind-work-corp-vm-1-2`.
        bind_ref: String,
    },
    /// Originated from an explicit `nixling usb attach <vm> <busid> --apply`
    /// that passed the sysfs-presence check, the USB-capable gate, and the
    /// active-claim exclusivity check. No static bundle allowlist is required.
    Explicit,
}

impl UsbipClaimSource {
    /// `true` if this is an explicitly requested attach (no bundle allowlist).
    pub fn is_explicit(&self) -> bool {
        matches!(self, Self::Explicit)
    }

    /// `true` if this claim has bundle-resolved firewall and bind intent refs.
    pub fn is_declared(&self) -> bool {
        matches!(self, Self::Declared { .. })
    }
}

/// Daemon-internal handoff record for one active USB claim.
///
/// Built inside `dispatch_broker_usbip_bind` after all admission checks pass;
/// passed to broker op selectors that differ between the declared and explicit
/// paths. Never serialized to the public wire; the broker audit layer sees only
/// opaque refs or explicit busid+vm+env fields per the op.
///
/// The `lock_path` is the OFD-lockable file under
/// `/run/nixling/locks/usbip/<busid>` that the broker opens/acquires for the
/// exclusive USB claim. The daemon passes the OFD lock fd via `SCM_RIGHTS`
/// when dispatching broker USB runner/firewall ops so the kernel enforces
/// mutual exclusion without a daemon-global in-memory table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipDaemonClaimRecord {
    /// Validated sysfs busid, e.g. `1-2` or `2-1.4.5`.
    pub busid: String,
    /// Target VM name (USB-capable, manifest-present).
    pub vm: String,
    /// Env the VM belongs to (from manifest entry).
    pub env: String,
    /// Per-env proxy listen port for the USBIP proxy (default 3240).
    pub proxy_port: u16,
    /// Source of this claim: declared from bundle or explicit operator request.
    pub source: UsbipClaimSource,
    /// Absolute path to the per-busid OFD lock file.
    pub lock_path: String,
}

impl UsbipDaemonClaimRecord {
    /// Canonical lock-file path for a validated busid.
    ///
    /// The busid must have already passed [`validate_bus_id`] before calling
    /// this, since the result is used as a filesystem path.
    pub fn lock_path_for_busid(busid: &str) -> String {
        format!("/run/nixling/locks/usbip/{busid}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_bus_ids() {
        for bus_id in ["0", "1", "1-2", "10-3.2", "2-1.4.5"] {
            assert_eq!(validate_bus_id(bus_id), Ok(()), "{bus_id} should pass");
        }
    }

    #[test]
    fn rejects_non_canonical_bus_ids() {
        for bus_id in ["", "01", "1-", "1-.2", "1-02", "1-2.", "1-2/a", "1-٢"] {
            assert!(validate_bus_id(bus_id).is_err(), "{bus_id:?} should fail");
        }
    }

    #[test]
    fn sanitizes_usb_hex_ids_for_projection() {
        assert_eq!(sanitize_usb_hex_id(Some("1050")), Some("1050".to_owned()));
        assert_eq!(sanitize_usb_hex_id(Some("ABCD")), Some("abcd".to_owned()));
        assert_eq!(format_usb_hex_id(0x0407), "0407");

        for rejected in [
            None,
            Some(""),
            Some("1050\n"),
            Some("serial"),
            Some("12345"),
        ] {
            assert_eq!(sanitize_usb_hex_id(rejected), None);
        }
    }
}
