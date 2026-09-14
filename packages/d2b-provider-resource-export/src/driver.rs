//! The ResourceExport resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `ResourceExport` rows.
//!
//! `ResourceExport` declares that a resource is exported to another zone: the
//! driver converges it as metadata once its desired state is admitted, and the
//! export's own realization belongs to the exported resource's family.
//!
//! The conversion is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The `ResourceExport` type's driver declaration.
pub fn resource_export_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::RESOURCE_EXPORT)
}
