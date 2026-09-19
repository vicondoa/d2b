//! The Device family's declared open vocabulary.
//!
//! The `Device` ResourceType is served by four hardware Providers, and each
//! family declares the host device-matrix classes its rows claim. This
//! module aggregates those declarations into the closed class set the
//! family's opens admit: an open for a class no family declares fails
//! closed rather than being admitted by default.

use d2b_provider_device_gpu::vocabulary::GPU_DEVICE_CLASSES;
use d2b_provider_device_security_key::vocabulary::SECURITY_KEY_DEVICE_CLASS;
use d2b_provider_device_tpm::vocabulary::TPM_DEVICE_CLASS;
use d2b_provider_device_usbip::vocabulary::USBIP_DEVICE_CLASS;

/// Refusal of an open whose device class no Device family declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceOpenAdmissionError {
    /// The requested class is not in the families' declared class set.
    ClassNotDeclared,
}

impl DeviceOpenAdmissionError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ClassNotDeclared => "device-open-class-not-declared",
        }
    }
}

impl core::fmt::Display for DeviceOpenAdmissionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for DeviceOpenAdmissionError {}

/// Refuse an open whose device class no Device family declares.
///
/// The class set is the union of the four families' declarations; a class
/// the families do not declare (a network, storage, or media class, or a
/// class that has not moved into a declaration yet) is refused closed.
pub fn admit_open_class(class: &str) -> Result<(), DeviceOpenAdmissionError> {
    let declared = class == TPM_DEVICE_CLASS
        || class == USBIP_DEVICE_CLASS
        || class == SECURITY_KEY_DEVICE_CLASS
        || GPU_DEVICE_CLASSES.contains(&class);
    if declared {
        Ok(())
    } else {
        Err(DeviceOpenAdmissionError::ClassNotDeclared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_admits_every_family_declared_class() {
        for class in [
            TPM_DEVICE_CLASS,
            USBIP_DEVICE_CLASS,
            SECURITY_KEY_DEVICE_CLASS,
            "kvm",
            "dri",
            "nvidia-ctl",
            "nvidia-uvm",
            "nvidia-render",
            "udmabuf",
        ] {
            assert_eq!(admit_open_class(class), Ok(()), "{class} is declared");
        }
    }

    #[test]
    fn open_fails_closed_for_a_class_no_family_declares() {
        for outside in ["net-tun", "vhost-net", "fuse", "vfio", "pipewire-socket"] {
            assert_eq!(
                admit_open_class(outside),
                Err(DeviceOpenAdmissionError::ClassNotDeclared),
                "an open for {outside} must fail closed"
            );
        }
        assert_eq!(
            DeviceOpenAdmissionError::ClassNotDeclared.code(),
            "device-open-class-not-declared"
        );
    }
}