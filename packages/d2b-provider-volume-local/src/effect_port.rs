//! Provider-side opaque bindings for Volume state effects.
//!
//! No type in this module carries a host path, numeric identity, descriptor,
//! command, or broker operation. The core adapter resolves opaque IDs and
//! routes named view descriptors out-of-band to the target supervisor.

use std::fmt;

use d2b_contracts_resource::v3::execution_policy::BoundedToken;

/// Neutral core/broker Volume effect boundary.
pub use d2b_contracts::v3::effect_port::{
    AccessClass, CleanupTrigger, EffectError, LayoutEntryId, ProvisionOutcome, QuotaCapacityStatus,
    QuotaUsage, RepairOutcome, RotateSealingKeyDisposition, RotateSealingKeyRequest,
    RotateSealingKeyResult, SealingPolicyId, SourcePolicyId, StoreSyncOutcome, UserId, ViewId,
    VolumeEffectIdError, VolumeEffectPort, VolumeId, VolumeMountToken,
};

/// The execution domain in which a volume-local controller runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionDomain {
    /// One Host domain.
    Host(BoundedToken),
    /// One Guest-local domain.
    Guest(BoundedToken),
}

/// Closed error set for the Provider-side effect seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeEffectError {
    /// A request violated a closed semantic bound.
    InvalidRequest,
    /// A Guest-local controller was asked to observe another domain.
    DomainMismatch,
    /// The adapter could not reserve the declared quota.
    QuotaInsufficient,
    /// The Volume is already at or above its soft quota.
    QuotaExceeded,
    /// Marker verification failed closed.
    MarkerFailed,
    /// The core adapter failed without exposing backend detail.
    BackendUnavailable,
}

impl VolumeEffectError {
    /// Return the stable redacted code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "volume-effect-request-invalid",
            Self::DomainMismatch => "volume-domain-mismatch",
            Self::QuotaInsufficient => "quota-insufficient",
            Self::QuotaExceeded => "volume-quota-exceeded",
            Self::MarkerFailed => "volume-marker-verification-failed",
            Self::BackendUnavailable => "volume-effect-backend-unavailable",
        }
    }
}

impl fmt::Display for VolumeEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for VolumeEffectError {}

/// Ensure this Provider instance can act on the source execution domain.
pub fn validate_domain(
    controller: &ExecutionDomain,
    source: &ExecutionDomain,
) -> Result<(), VolumeEffectError> {
    if controller == source {
        Ok(())
    } else {
        Err(VolumeEffectError::DomainMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(value: &str) -> BoundedToken {
        BoundedToken::parse(value).unwrap()
    }

    #[test]
    fn guest_local_domain_rejects_host_and_other_guest_sources() {
        let local = ExecutionDomain::Guest(token("work-vm"));
        assert!(validate_domain(&local, &ExecutionDomain::Guest(token("work-vm"))).is_ok());
        assert_eq!(
            validate_domain(&local, &ExecutionDomain::Host(token("host-system"))),
            Err(VolumeEffectError::DomainMismatch)
        );
        assert_eq!(
            validate_domain(&local, &ExecutionDomain::Guest(token("personal-vm"))),
            Err(VolumeEffectError::DomainMismatch)
        );
    }
}
