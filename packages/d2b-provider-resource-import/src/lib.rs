//! The ResourceImport provider crate: the ResourceImport resource type's driver declaration.
//!
//! The crate owns the ResourceImport type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `ResourceImport` declares that a remote resource is imported into this
//! zone: the driver converges it as metadata once its desired state is
//! admitted, and the imported resource's own family realizes it.

#![deny(missing_docs)]

mod driver;

pub use driver::resource_import_descriptor;
