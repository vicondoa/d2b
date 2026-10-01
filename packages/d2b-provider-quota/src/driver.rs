//! Read-side helpers for one committed `Quota` row.
//!
//! The row's own conversion - the driver that publishes its ceilings - is in
//! [`crate::serving`]. What stays here is the decode boundary, so the driver
//! and the manager-boundary admission read the same committed bytes through
//! one function rather than through two that could drift.

use crate::quota::{QuotaError, QuotaPolicy};

/// Read one committed `Quota` row's desired state as the Zone's limits.
///
/// The bytes are the canonical desired-state object the manager stored, and
/// the decode is the one in [`crate::quota`]: a row whose ceilings this crate
/// cannot read is refused rather than admitted as a Zone without limits, so
/// the failure is visible at admission instead of at reconcile.
///
/// This is the contract's own error, not a provider-local string, so a caller
/// names the same reason a status or a refusal would.
pub fn quota_policy_of_spec(desired_state: &[u8]) -> Result<QuotaPolicy, QuotaError> {
    QuotaPolicy::decode(desired_state)
}
