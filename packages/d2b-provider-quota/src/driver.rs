//! The Quota resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `Quota` rows.
//!
//! `Quota` is a scarce-resource scope claim: the driver converges it as
//! metadata once its desired state is admitted. The Host-global authority
//! index that arbitrates the claim's class beside every other scarce class the
//! session admits is controller-session machinery and stays in
//! `d2b-core-controller`.
//!
//! The conversion is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.
//!
//! # The type's one decision
//!
//! A `Quota` row is not only a declaration: its ceilings are the Zone's
//! limits, and they are decided in [`crate::quota`] before any row is
//! persisted. [`quota_policy_of_spec`] is the driver's read path into that
//! decision - the committed desired-state bytes of a `Quota` row in, the
//! accepted [`QuotaPolicy`](crate::quota::QuotaPolicy) out - so the plane
//! admits against the same value the driver would reconcile from, and no
//! caller has to re-read the row's bytes with a second decoder.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

use crate::quota::{QuotaError, QuotaPolicy};

/// The `Quota` type's driver declaration.
pub fn quota_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::QUOTA)
}

/// Read one committed `Quota` row's desired state as the Zone's limits.
///
/// The bytes are the canonical desired-state object the manager stored, and
/// the decode is the one in [`crate::quota`]: a row whose ceilings this crate
/// cannot read is refused rather than admitted as a Zone without limits, so
/// the failure is visible at admission instead of at reconcile.
pub fn quota_policy_of_spec(desired_state: &[u8]) -> Result<QuotaPolicy, QuotaError> {
    QuotaPolicy::decode(desired_state)
}
