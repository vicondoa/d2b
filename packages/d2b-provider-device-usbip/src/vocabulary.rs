//! The USBIP family's declared vocabulary.
//!
//! Every fact here is owned by this crate: the host device-matrix class the
//! USBIP role claims, the inventory `busClass` the family binds, the
//! per-busid lock directory, the kernel-module names the host capability
//! probe reads under `/sys/module`, the runner-role spelling the bind
//! extension keys on, and the host device-node facts (major/minor pair and
//! owner account) the privileged open validates. Shared consumers read these
//! constants instead of restating the spellings.

/// The host device-matrix class the USBIP role claims.
pub const USBIP_DEVICE_CLASS: &str = "usbip-host";

/// The inventory `busClass` the family binds: only USB devices are admitted.
pub const USBIP_BUS_CLASS: &str = "usb";

/// The OFD-lock directory one bind's lock file lives under
/// (`/run/d2b/locks/usbip/<busid>`).
pub const USBIP_LOCK_DIR: &str = "/run/d2b/locks/usbip";

/// The kernel module the USBIP core stack loads as (`/sys/module/usbip_core`).
pub const USBIP_CORE_MODULE: &str = "usbip_core";

/// The kernel module the USBIP host driver loads as (`/sys/module/usbip_host`).
pub const USBIP_HOST_MODULE: &str = "usbip_host";

/// The runner-role spelling the USBIP backend bind extension keys on.
pub const USBIP_BIND_ROLE: &str = "usbip";

/// The USBIP device-node major number the privileged open validates.
pub const USBIP_DEVICE_MAJOR: u64 = 251;

/// The USBIP device-node minor number the privileged open validates.
pub const USBIP_DEVICE_MINOR: u64 = 0;

/// The owner account the USBIP device node is validated against.
pub const USBIP_DEVICE_OWNER: &str = "root";

/// Refusal of a bind whose device's bus class the family does not declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipBindClassError {
    /// The device's inventory `busClass` is not the class this family binds.
    ClassNotDeclared,
}

impl UsbipBindClassError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ClassNotDeclared => "usbip-bind-class-not-declared",
        }
    }
}

impl core::fmt::Display for UsbipBindClassError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for UsbipBindClassError {}

/// Refuse a bind whose device's bus class the family does not declare.
///
/// The family binds physical USB devices only; a bind for a device selected
/// under any other `busClass` (hidraw, drm, pci, tpm) is refused closed
/// rather than admitted by default.
pub fn admit_bind_bus_class(bus_class: &str) -> Result<(), UsbipBindClassError> {
    if bus_class == USBIP_BUS_CLASS {
        Ok(())
    } else {
        Err(UsbipBindClassError::ClassNotDeclared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_admits_the_declared_usb_class() {
        assert_eq!(admit_bind_bus_class(USBIP_BUS_CLASS), Ok(()));
        assert_eq!(admit_bind_bus_class("usb"), Ok(()));
    }

    #[test]
    fn bind_refuses_a_device_outside_the_declared_class() {
        for outside in ["hidraw", "drm", "pci", "tpm"] {
            assert_eq!(
                admit_bind_bus_class(outside),
                Err(UsbipBindClassError::ClassNotDeclared),
                "a {outside} device must be refused"
            );
        }
        assert_eq!(
            UsbipBindClassError::ClassNotDeclared.code(),
            "usbip-bind-class-not-declared"
        );
    }
}