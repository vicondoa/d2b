//! Host-global KVM Device admission.

use std::collections::BTreeMap;

use d2b_contracts::v3::ResourceRef;

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

/// A retained Host-global authority reservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityReservation {
    key: [u8; 32],
    owner_ref: ResourceRef,
}

impl AuthorityReservation {
    /// Borrow the owner of this reservation.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }
}

/// Single-owner Host-global authority index.
#[derive(Debug, Default)]
pub struct HostGlobalAuthorityIndex {
    owners: BTreeMap<[u8; 32], ResourceRef>,
}

impl HostGlobalAuthorityIndex {
    /// Construct an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve a key before an asynchronous effect starts.
    pub fn reserve(
        &mut self,
        key: [u8; 32],
        owner_ref: ResourceRef,
    ) -> Result<AuthorityReservation, DeviceAdmissionError> {
        if let Some(existing) = self.owners.get(&key) {
            if existing != &owner_ref {
                return Err(DeviceAdmissionError::WrongOwner);
            }
            return Err(DeviceAdmissionError::WrongOwner);
        }
        self.owners.insert(key, owner_ref.clone());
        Ok(AuthorityReservation { key, owner_ref })
    }

    /// Release a reservation only for its original owner.
    pub fn release(
        &mut self,
        reservation: AuthorityReservation,
    ) -> Result<(), DeviceAdmissionError> {
        match self.owners.get(&reservation.key) {
            Some(owner) if owner == &reservation.owner_ref => {
                self.owners.remove(&reservation.key);
                Ok(())
            }
            _ => Err(DeviceAdmissionError::WrongOwner),
        }
    }
}
