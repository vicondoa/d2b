//! The Zone provider crate: the Zone resource type's driver declaration.
//!
//! The crate owns the Zone type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `Zone` is the zone's own row: the driver converges it as metadata once its
//! desired state is admitted, and the crate carries the production Zone status
//! projection (`zone_status`), which always emits the mandatory system-core
//! handler pair.

mod driver;

pub mod zone_status;

pub use driver::zone_descriptor;

pub use zone_status::*;
