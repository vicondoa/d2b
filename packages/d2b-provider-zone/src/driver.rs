//! The Zone resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `Zone` rows.
//!
//! `Zone` is the zone's own row: the driver converges it as metadata once its
//! desired state is admitted, and the crate carries the production Zone status
//! projection (`zone_status`), which always emits the mandatory system-core
//! handler pair.
//!
//! The conversion is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The `Zone` type's driver declaration.
pub fn zone_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::ZONE)
}
