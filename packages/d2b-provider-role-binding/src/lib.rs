//! The RoleBinding provider crate: the RoleBinding resource type's driver, its spec decoder,
//! and its driver declaration.
//!
//! The crate owns the RoleBinding type's complete resource knowledge: the driver's
//! validate, recover, reconcile, finalize, and delete verbs and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! `RoleBinding` binds a role to its declared subjects. The driver converges it as metadata once its desired state is admitted; the row carries the role reference and the typed subject, resource, zone, and execution scopes the authorization path resolves.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    ROLE_BINDING_TYPE_NAME, RoleBindingDriver, RoleBindingDriverFactory, role_binding_descriptor,
    role_binding_spec_decoder,
};
