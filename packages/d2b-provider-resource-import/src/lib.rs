//! The ResourceImport provider crate: the ResourceImport resource type's driver, its spec decoder,
//! and its driver declaration.
//!
//! The crate owns the ResourceImport type's complete resource knowledge: the driver's
//! validate, recover, reconcile, finalize, and delete verbs and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! `ResourceImport` declares that a remote resource is imported into this zone. The driver converges it as metadata once its desired state is admitted; the imported resource's own family realizes it.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    RESOURCE_IMPORT_TYPE_NAME, ResourceImportDriver, ResourceImportDriverFactory, resource_import_descriptor,
    resource_import_spec_decoder,
};
