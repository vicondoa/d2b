//! Quota admission.
//!
//! The actual filesystem capability probe is performed by the injected source
//! effect adapter.  This pure decision keeps hard admission independent of a
//! filesystem or broker.

use d2b_contracts_resource::v3::volume::{QuotaEnforcement, QuotaSpec};

use crate::error::VolumeLocalError;
use crate::port::QuotaCapability;

/// Admit hard quota mode only when the backing adapter can enforce it.
pub fn admit_quota(
    quota: Option<&QuotaSpec>,
    capability: QuotaCapability,
) -> Result<(), VolumeLocalError> {
    let Some(quota) = quota else {
        return Ok(());
    };
    if quota.enforcement() == QuotaEnforcement::Hard && capability == QuotaCapability::Unenforceable
    {
        return Err(VolumeLocalError::QuotaUnenforceable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_quota_requires_an_enforceable_filesystem() {
        let quota = QuotaSpec::new(Some(1024), Some(8), QuotaEnforcement::Hard).unwrap();
        assert_eq!(
            admit_quota(Some(&quota), QuotaCapability::Unenforceable),
            Err(VolumeLocalError::QuotaUnenforceable)
        );
        assert!(admit_quota(Some(&quota), QuotaCapability::Enforceable).is_ok());
    }
}
