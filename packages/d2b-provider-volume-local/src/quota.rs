//! Quota admission and bounded usage decisions.
//!
//! The actual filesystem capability probe is performed by the injected source
//! effect adapter.  These pure decisions keep hard admission, usage reporting,
//! and write rejection independent of a filesystem or broker.

use d2b_contracts::v3::volume::{QuotaEnforcement, QuotaSpec};

use crate::error::VolumeLocalError;
use crate::port::QuotaCapability;

/// Bounded current usage reported by the effect adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaUsage {
    /// Charged bytes.
    pub used_bytes: u64,
    /// Charged inode count.
    pub inode_count: u64,
}

/// Result of checking one usage sample against a declared quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaUsageDecision {
    /// The write or current state is within the declared limits.
    Allowed,
    /// The write must be refused without changing the Volume.
    Rejected,
}

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

/// Check current bytes and inodes against the declared ceilings.
pub fn check_usage(
    quota: Option<&QuotaSpec>,
    usage: QuotaUsage,
) -> Result<QuotaUsageDecision, VolumeLocalError> {
    let Some(quota) = quota else {
        return Ok(QuotaUsageDecision::Allowed);
    };
    if quota.enforcement() == QuotaEnforcement::None {
        return Ok(QuotaUsageDecision::Allowed);
    }
    if quota
        .max_bytes()
        .is_some_and(|limit| usage.used_bytes > limit)
        || quota
            .max_inodes()
            .is_some_and(|limit| usage.inode_count > limit)
    {
        return Ok(QuotaUsageDecision::Rejected);
    }
    Ok(QuotaUsageDecision::Allowed)
}

/// Return the stable write-rejection error for a rejected usage sample.
pub const fn usage_error(decision: QuotaUsageDecision) -> Option<VolumeLocalError> {
    match decision {
        QuotaUsageDecision::Allowed => None,
        QuotaUsageDecision::Rejected => Some(VolumeLocalError::QuotaExceeded),
    }
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

    #[test]
    fn usage_checks_both_byte_and_inode_limits() {
        let quota = QuotaSpec::new(Some(1024), Some(8), QuotaEnforcement::Hard).unwrap();
        assert_eq!(
            check_usage(
                Some(&quota),
                QuotaUsage {
                    used_bytes: 1025,
                    inode_count: 1,
                }
            )
            .unwrap(),
            QuotaUsageDecision::Rejected
        );
        assert_eq!(
            check_usage(
                Some(&quota),
                QuotaUsage {
                    used_bytes: 1,
                    inode_count: 9,
                }
            )
            .unwrap(),
            QuotaUsageDecision::Rejected
        );
    }

    #[test]
    fn informational_limits_do_not_reject_writes() {
        let quota = QuotaSpec::new(Some(1), Some(1), QuotaEnforcement::None).unwrap();
        assert_eq!(
            check_usage(
                Some(&quota),
                QuotaUsage {
                    used_bytes: 2,
                    inode_count: 2,
                }
            )
            .unwrap(),
            QuotaUsageDecision::Allowed
        );
    }
}
