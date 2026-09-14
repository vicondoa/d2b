//! The RoleBinding provider crate: the RoleBinding resource type's driver declaration.
//!
//! The crate owns the RoleBinding type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `RoleBinding` binds a role to its declared subjects: the driver converges
//! it as metadata once its desired state is admitted, and the row carries the
//! role reference and the typed subject, resource, zone, and execution scopes
//! the authorization path resolves.

#![deny(missing_docs)]

mod driver;

pub use driver::role_binding_descriptor;
