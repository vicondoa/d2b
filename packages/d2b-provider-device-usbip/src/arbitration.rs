//! Device claim arbitration for USBIP.

use core::fmt;
use d2b_contracts_resource::v3::{ResourceUid, device::DeviceArbitration};

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
    ///
    /// # Errors
    ///
    /// Returns [`UsbipClaimError::ArbitrationViolation`] when the claim
    /// ceiling is outside `1..=16` or an exclusive arbitration requests a
    /// ceiling other than one.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).unwrap()
    }

    fn backing() -> PhysicalUsbBackingToken {
        PhysicalUsbBackingToken::from_core([7; 32])
    }

    #[test]
    fn constructor_rejects_ceilings_outside_one_to_sixteen() {
        let rejected = [
            (DeviceArbitration::Shared, 0u32),
            (DeviceArbitration::Shared, 17),
            (DeviceArbitration::Exclusive, 0),
            (DeviceArbitration::Exclusive, 2),
        ];
        for (arbitration, max_claims) in rejected {
            assert_eq!(
                UsbipArbitrator::new(arbitration, max_claims, backing()).unwrap_err(),
                UsbipClaimError::ArbitrationViolation,
                "ceiling {max_claims} under {arbitration:?} must be rejected",
            );
        }
        for (arbitration, max_claims) in [
            (DeviceArbitration::Shared, 1u32),
            (DeviceArbitration::Shared, 16),
            (DeviceArbitration::Exclusive, 1),
        ] {
            assert!(
                UsbipArbitrator::new(arbitration, max_claims, backing()).is_ok(),
                "ceiling {max_claims} under {arbitration:?} must be accepted",
            );
        }
    }

    #[test]
    fn claim_paths_follow_the_arbitration_mode_and_ceiling() {
        let table = [
            (
                DeviceArbitration::Exclusive,
                1u32,
                Ok(()),
                Err(UsbipClaimError::ClaimConflict),
                Err(UsbipClaimError::ClaimConflict),
            ),
            (
                DeviceArbitration::Shared,
                1u32,
                Ok(()),
                Err(UsbipClaimError::MaxClaimsExceeded),
                Err(UsbipClaimError::MaxClaimsExceeded),
            ),
            (
                DeviceArbitration::Shared,
                2u32,
                Ok(()),
                Ok(()),
                Err(UsbipClaimError::MaxClaimsExceeded),
            ),
        ];
        for (arbitration, ceiling, first, second, third) in table {
            let mut arbiter = UsbipArbitrator::new(arbitration, ceiling, backing()).unwrap();
            assert_eq!(arbiter.claim(uid("123e4567-e89b-42d3-a456-426614174000"), backing()), first);
            assert_eq!(arbiter.claim(uid("223e4567-e89b-42d3-a456-426614174001"), backing()), second);
            assert_eq!(arbiter.claim(uid("323e4567-e89b-42d3-a456-426614174002"), backing()), third);
        }
    }

    #[test]
    fn same_holder_reclaim_is_idempotent() {
        let mut arbiter = UsbipArbitrator::new(DeviceArbitration::Shared, 2, backing()).unwrap();
        let holder = uid("123e4567-e89b-42d3-a456-426614174000");
        assert_eq!(arbiter.claim(holder.clone(), backing()), Ok(()));
        assert_eq!(arbiter.claim(holder, backing()), Ok(()));
        assert_eq!(arbiter.claim_count(), 1);
    }

    #[test]
    fn release_removes_exactly_the_named_claimant_and_frees_the_slot() {
        let mut arbiter = UsbipArbitrator::new(DeviceArbitration::Shared, 2, backing()).unwrap();
        let first = uid("123e4567-e89b-42d3-a456-426614174000");
        let second = uid("223e4567-e89b-42d3-a456-426614174001");
        arbiter.claim(first.clone(), backing()).unwrap();
        arbiter.claim(second.clone(), backing()).unwrap();
        assert!(arbiter.release(&first));
        assert_eq!(arbiter.claim_count(), 1);
        assert!(!arbiter.release(&first));
        assert_eq!(arbiter.claim(first, backing()), Ok(()));
        assert_eq!(arbiter.claim_count(), 2);
    }

    #[test]
    fn claim_rejects_a_mismatched_backing_token() {
        let mut arbiter = UsbipArbitrator::new(DeviceArbitration::Shared, 1, backing()).unwrap();
        assert_eq!(
            arbiter.claim(
                uid("123e4567-e89b-42d3-a456-426614174000"),
                PhysicalUsbBackingToken::from_core([9; 32]),
            ),
            Err(UsbipClaimError::PhysicalBackingConflict)
        );
        assert_eq!(arbiter.claim_count(), 0);
    }
}
