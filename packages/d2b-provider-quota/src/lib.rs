//! The Quota provider crate: the `Quota` resource type's driver, its spec
//! decoder, and its driver declaration.
//!
//! The crate owns the `Quota` type's complete resource knowledge: the driver's
//! validate, recover, reconcile, finalize, and delete verbs and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! `Quota` is a scarce-resource scope claim: the driver converges it as
//! metadata once its desired state is admitted. The Host-global authority
//! index that arbitrates the claim's class (beside every other scarce class
//! the session admits) is controller-session machinery, not this type's, and
//! stays in `d2b-core-controller`.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    QUOTA_TYPE_NAME, QuotaDriver, QuotaDriverFactory, quota_descriptor, quota_spec_decoder,
};
