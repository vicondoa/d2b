//! The Quota provider crate: the Quota resource type's driver.
//!
//! The crate owns the Quota type's identity, the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by, and the conversion that makes the type mean
//! something: the driver in [`serving`] reads a committed ceiling row, counts
//! the Zone's committed usage, and publishes the resulting policy where the
//! manager-boundary admission reads it.
//!
//! `Quota` is a scarce-resource scope claim. What its ceilings refuse, and the
//! arithmetic that decides it, are in [`quota`] and are decided here rather
//! than in the daemon composition. The one daemon-owned fact a ceiling is
//! measured against - the Zone's committed usage - is declared as the
//! per-Zone runtime in [`facets`].

#![deny(missing_docs)]

mod facets;
mod serving;

/// The Quota ResourceType spec and status shapes owned by this crate, and the
/// admission decision its ceilings make (U40, R8/R36).
pub mod quota;

pub use facets::{UsageSource, ZoneQuotaRuntime, install, remove, runtime};
pub use serving::{
    QUOTA_RESOURCE_TYPE, QuotaDriver, QuotaDriverFactory, QuotaRowError, quota_descriptor,
    quota_policy_of_spec, quota_spec_decoder,
};
