//! The Operation resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `Operation` rows.
//!
//! `Operation` is a broker operation row: it declares the payload schema, the
//! authority profile, the audit facet, and the handler reference the generic
//! envelope dispatches. The type is declared here and its rows materialize in
//! the policy-rows unit.
//!
//! The conversion is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The `Operation` type's driver declaration.
pub fn operation_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::OPERATION)
}
