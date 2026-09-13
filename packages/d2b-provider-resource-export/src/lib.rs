//! The ResourceExport provider crate: the ResourceExport resource type's driver, its spec decoder,
//! and its driver declaration.
//!
//! The crate owns the ResourceExport type's complete resource knowledge: the driver's
//! validate, recover, reconcile, finalize, and delete verbs and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! `ResourceExport` declares that a resource is exported to another zone. The driver converges it as metadata once its desired state is admitted; the export's own realization belongs to the exported resource's family.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    RESOURCE_EXPORT_TYPE_NAME, ResourceExportDriver, ResourceExportDriverFactory, resource_export_descriptor,
    resource_export_spec_decoder,
};
