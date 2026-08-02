//! Device claim arbitration for USBIP.

use core::fmt;
use d2b_contracts::v3::{ResourceUid, device::DeviceArbitration};

use crate::busid::PhysicalUsbBackingToken;

/// Closed USBIP Device claim failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipClaimError {
    /// The backing authority did not match the expected Device.
    PhysicalBackingConflict,
    /// An exclusive Device already has a claimant.
    ClaimConflict,
    /// The configured claim ceiling was reached.
    MaxClaimsExceeded,
    /// The requested arbitration and claim mode disagree.
    ArbitrationViolation,
}

impl UsbipClaimError {
    /// Return the stable Device error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::PhysicalBackingConflict => "physical-usb-backing-conflict",
            Self::ClaimConflict => "device-claim-conflict",
            Self::MaxClaimsExceeded => "device-claim-max-exceeded",
            Self::ArbitrationViolation => "device-arbitration-violation",
        }
    }
}

impl fmt::Display for UsbipClaimError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for UsbipClaimError {}

/// A bounded USBIP claimant record.
#[derive(Clone, PartialEq, Eq)]
pub struct UsbipClaim {
    holder: ResourceUid,
    backing: PhysicalUsbBackingToken,
}

impl UsbipClaim {
    /// Borrow the opaque holder identity.
    pub const fn holder(&self) -> &ResourceUid {
        &self.holder
    }

    /// Borrow the Core-derived backing token.
    pub const fn backing(&self) -> &PhysicalUsbBackingToken {
        &self.backing
    }
}

impl fmt::Debug for UsbipClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UsbipClaim(<redacted>)")
    }
}

/// Exclusive/shared Device claim arbiter.
pub struct UsbipArbitrator {
    arbitration: DeviceArbitration,
    max_claims: u32,
    backing: PhysicalUsbBackingToken,
    claims: Vec<UsbipClaim>,
}

impl UsbipArbitrator {
    /// Construct an arbiter after validating the Device claim ceiling.
    pub fn new(
        arbitration: DeviceArbitration,
        max_claims: u32,
        backing: PhysicalUsbBackingToken,
    ) -> Result<Self, UsbipClaimError> {
        if !(1..=16).contains(&max_claims)
            || (arbitration == DeviceArbitration::Exclusive && max_claims != 1)
        {
            return Err(UsbipClaimError::ArbitrationViolation);
        }
        Ok(Self {
            arbitration,
            max_claims,
            backing,
            claims: Vec::new(),
        })
    }

    /// Admit one claimant before any bind, module, firewall, or relay effect.
    pub fn claim(
        &mut self,
        holder: ResourceUid,
        backing: PhysicalUsbBackingToken,
    ) -> Result<(), UsbipClaimError> {
        if backing != self.backing {
            return Err(UsbipClaimError::PhysicalBackingConflict);
        }
        if self.claims.iter().any(|claim| claim.holder == holder) {
            return Ok(());
        }
        if self.arbitration == DeviceArbitration::Exclusive && !self.claims.is_empty() {
            return Err(UsbipClaimError::ClaimConflict);
        }
        if self.claims.len() >= self.max_claims as usize {
            return Err(UsbipClaimError::MaxClaimsExceeded);
        }
        self.claims.push(UsbipClaim { holder, backing });
        Ok(())
    }

    /// Release one exact claimant.
    pub fn release(&mut self, holder: &ResourceUid) -> bool {
        let before = self.claims.len();
        self.claims.retain(|claim| &claim.holder != holder);
        before != self.claims.len()
    }

    /// Return the current claimant count.
    pub const fn claim_count(&self) -> usize {
        self.claims.len()
    }

    /// Borrow the bounded claimant list.
    pub fn claims(&self) -> &[UsbipClaim] {
        &self.claims
    }
}

impl fmt::Debug for UsbipArbitrator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UsbipArbitrator")
            .field("arbitration", &self.arbitration)
            .field("max_claims", &self.max_claims)
            .field("claim_count", &self.claims.len())
            .finish()
    }
}
