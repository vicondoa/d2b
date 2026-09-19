//! The Quota provider crate: the Quota resource type's driver declaration.
//!
//! The crate owns the Quota type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `Quota` is a scarce-resource scope claim: the driver converges it as
//! metadata once its desired state is admitted. The Host-global authority
//! index that arbitrates the claim's class beside every other scarce class the
//! session admits is controller-session machinery and stays in
//! `d2b-core-controller`.

#![deny(missing_docs)]

mod driver;

/// The Quota ResourceType spec and status shapes owned by this crate.
pub mod quota;

pub use driver::quota_descriptor;
pub use quota::*;
