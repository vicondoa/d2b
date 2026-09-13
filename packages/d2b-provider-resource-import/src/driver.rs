//! The ResourceImport resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `ResourceImport` rows.
//!
//! `ResourceImport` declares that a remote resource is imported into this
//! zone: the driver converges it as metadata once its desired state is
//! admitted, and the imported resource's own family realizes it.
//!
//! The conversion is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The `ResourceImport` type's driver declaration.
pub fn resource_import_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::RESOURCE_IMPORT)
}
