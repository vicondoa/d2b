//! The ResourceExport provider crate: the ResourceExport resource type's driver declaration.
//!
//! The crate owns the ResourceExport type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `ResourceExport` declares that a resource is exported to another zone: the
//! driver converges it as metadata once its desired state is admitted, and the
//! export's own realization belongs to the exported resource's family.

#![deny(missing_docs)]

mod driver;

pub use driver::resource_export_descriptor;
