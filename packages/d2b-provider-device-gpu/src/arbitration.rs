//! Exclusive and shared GPU claim admission.

use core::fmt;

use d2b_contracts::v3::{ResourceUid, device::DeviceArbitration};

use crate::authority::GpuBackingToken;

/// Closed GPU claim failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuClaimError {
    /// The requested arbitration is incompatible with the selected mode.
    ArbitrationViolation,
    /// A full-device claim is already held.
    ClaimConflict,
    /// The signed shared-holder ceiling was reached.
    MaxClaimsExceeded,
    /// The Core-derived backing identity did not match.
    PhysicalBackingConflict,
}

impl GpuClaimError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ArbitrationViolation => "gpu-arbitration-violation",
            Self::ClaimConflict => "device-claim-conflict",
            Self::MaxClaimsExceeded => "device-claim-max-exceeded",
            Self::PhysicalBackingConflict => "gpu-physical-backing-conflict",
        }
    }
}

impl fmt::Display for GpuClaimError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for GpuClaimError {}

/// One admitted GPU claimant.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuClaim {
    holder: ResourceUid,
    backing: GpuBackingToken,
}

impl GpuClaim {
    /// Borrow the opaque claimant identity.
    pub const fn holder(&self) -> &ResourceUid {
        &self.holder
    }

    /// Borrow the Core-derived backing token.
    pub const fn backing(&self) -> &GpuBackingToken {
        &self.backing
    }
}

impl fmt::Debug for GpuClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GpuClaim(<redacted>)")
    }
}

/// Pure claim arbiter used before any broker effect.
pub struct GpuArbitrator {
    arbitration: DeviceArbitration,
    max_claims: u32,
    render_node_only: bool,
    backing: GpuBackingToken,
    claims: Vec<GpuClaim>,
}

impl GpuArbitrator {
    /// Construct an arbiter from the signed Device base and Provider settings.
    pub fn new(
        arbitration: DeviceArbitration,
        max_claims: u32,
        render_node_only: bool,
        backing: GpuBackingToken,
    ) -> Result<Self, GpuClaimError> {
        if !(1..=16).contains(&max_claims)
            || (arbitration == DeviceArbitration::Exclusive && max_claims != 1)
            || (arbitration == DeviceArbitration::Shared && !render_node_only)
        {
            return Err(GpuClaimError::ArbitrationViolation);
        }
        Ok(Self {
            arbitration,
            max_claims,
            render_node_only,
            backing,
            claims: Vec::new(),
        })
    }

    /// Admit a claimant before opening a device or starting a worker.
    pub fn claim(
        &mut self,
        holder: ResourceUid,
        backing: GpuBackingToken,
    ) -> Result<(), GpuClaimError> {
        if backing != self.backing {
            return Err(GpuClaimError::PhysicalBackingConflict);
        }
        if self.claims.iter().any(|claim| claim.holder == holder) {
            return Ok(());
        }
        if self.arbitration == DeviceArbitration::Exclusive && !self.claims.is_empty() {
            return Err(GpuClaimError::ClaimConflict);
        }
        if self.claims.len() >= self.max_claims as usize {
            return Err(GpuClaimError::MaxClaimsExceeded);
        }
        self.claims.push(GpuClaim { holder, backing });
        Ok(())
    }

    /// Release one exact claimant after its worker has closed.
    pub fn release(&mut self, holder: &ResourceUid) -> bool {
        let before = self.claims.len();
        self.claims.retain(|claim| &claim.holder != holder);
        before != self.claims.len()
    }

    /// Return the current claimant count.
    pub const fn claim_count(&self) -> usize {
        self.claims.len()
    }

    /// Return the signed holder ceiling.
    pub const fn max_claims(&self) -> u32 {
        self.max_claims
    }

    /// Whether this arbiter is render-node-only.
    pub const fn render_node_only(&self) -> bool {
        self.render_node_only
    }

    /// Borrow the bounded claimant list.
    pub fn claims(&self) -> &[GpuClaim] {
        &self.claims
    }
}

impl fmt::Debug for GpuArbitrator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuArbitrator")
            .field("arbitration", &self.arbitration)
            .field("max_claims", &self.max_claims)
            .field("render_node_only", &self.render_node_only)
            .field("claim_count", &self.claims.len())
            .finish()
    }
}
