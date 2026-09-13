//! Host-global KVM Device admission.

use d2b_contracts_resource::v3::ResourceRef;

/// Observed Device phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePhase {
    /// Device has not reached Ready.
    Pending,
    /// Device is ready.
    Ready,
    /// Device failed closed.
    Failed,
    /// Device is degraded.
    Degraded,
}

/// Host platform class required by the QEMU media runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformClass {
    /// Supported x86_64 Linux platform.
    X86_64Linux,
    /// Unsupported platform.
    Other,
}

/// Core-derived Device observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceObservation {
    /// Device reference.
    pub device_ref: ResourceRef,
    /// Device phase.
    pub phase: DevicePhase,
    /// Current owner proof.
    pub owner_ref: Option<ResourceRef>,
    /// Host platform.
    pub platform: PlatformClass,
    /// Opaque Host-global authority key.
    pub authority_key: [u8; 32],
    /// Verified process identity binding.
    pub process_identity: Option<String>,
    /// Signed media contract identifier.
    pub media_contract: String,
}

/// Device admission failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAdmissionError {
    /// Device is not ready.
    NotReady,
    /// Device is owned by another resource.
    WrongOwner,
    /// Platform is not supported.
    UnsupportedPlatform,
    /// Process identity proof is missing or wrong.
    ProcessIdentityMismatch,
    /// Media contract is not the required version.
    MediaContractMismatch,
    /// Device is not a KVM Device.
    WrongDevice,
}

impl DeviceAdmissionError {
    /// Return a stable redacted error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotReady => "kvm-device-unavailable",
            Self::WrongOwner => "kvm-device-owner-mismatch",
            Self::UnsupportedPlatform => "kvm-platform-unsupported",
            Self::ProcessIdentityMismatch => "kvm-process-identity-mismatch",
            Self::MediaContractMismatch => "qemu-media-contract-mismatch",
            Self::WrongDevice => "kvm-device-ref-invalid",
        }
    }
}

/// Validate a KVM Device before a runner effect begins.
pub struct DeviceAdmission;

impl DeviceAdmission {
    /// Check owner, platform, process identity, and media contract.
    pub fn validate(
        guest_ref: &ResourceRef,
        observation: &DeviceObservation,
        expected_process_identity: &str,
        expected_contract: &str,
    ) -> Result<(), DeviceAdmissionError> {
        if observation.device_ref.to_canonical_string() != "Device/host-kvm" {
            return Err(DeviceAdmissionError::WrongDevice);
        }
        if observation.phase != DevicePhase::Ready {
            return Err(DeviceAdmissionError::NotReady);
        }
        if observation
            .owner_ref
            .as_ref()
            .is_some_and(|owner| owner != guest_ref)
        {
            return Err(DeviceAdmissionError::WrongOwner);
        }
        if observation.platform != PlatformClass::X86_64Linux {
            return Err(DeviceAdmissionError::UnsupportedPlatform);
        }
        if observation.process_identity.as_deref() != Some(expected_process_identity) {
            return Err(DeviceAdmissionError::ProcessIdentityMismatch);
        }
        if observation.media_contract != expected_contract {
            return Err(DeviceAdmissionError::MediaContractMismatch);
        }
        Ok(())
    }
}
