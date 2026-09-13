//! The Role provider crate: the Role resource type's driver, its spec decoder,
//! and its driver declaration.
//!
//! The crate owns the Role type's complete resource knowledge: the driver's
//! validate, recover, reconcile, finalize, and delete verbs and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! `Role` is an authority role. The driver converges it as metadata once its desired state is admitted; the revision-bound positive decision cache (`rbac`) that the resource API consults lives in this crate.


mod driver;

pub mod rbac;

pub use driver::{
    ROLE_TYPE_NAME, RoleDriver, RoleDriverFactory, role_descriptor,
    role_spec_decoder,
};

pub use rbac::*;
