//! Shared USBIP wire validation helpers.
//!
//! Bus IDs cross the CLI, daemon, broker, and guest-control boundary. Keep the
//! shape check here so every layer rejects the same traversal-/shell-unsafe
//! strings before they reach a subprocess argv.

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
