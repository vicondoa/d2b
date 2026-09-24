//! The Role provider crate: the Role resource type's driver declaration.
//!
//! The crate owns the Role type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `Role` is an authority role: the driver converges it as metadata once
//! its desired state is admitted, and the crate carries the revision-bound
//! positive authorization decision cache (`rbac`).

mod driver;

pub mod rbac;

pub use driver::role_descriptor;
