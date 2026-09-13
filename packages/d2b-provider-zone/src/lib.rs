//! The Zone provider crate: the Zone resource type's driver, its spec decoder,
//! and its driver declaration.
//!
//! The crate owns the Zone type's complete resource knowledge: the driver's
//! validate, recover, reconcile, finalize, and delete verbs and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! `Zone` is the zone's own row. The driver converges it as metadata once its desired state is admitted, and the crate carries the production Zone status projection (`zone_status`), which always emits the mandatory system-core handler pair.


mod driver;

pub mod zone_status;

pub use driver::{
    ZONE_TYPE_NAME, ZoneDriver, ZoneDriverFactory, zone_descriptor,
    zone_spec_decoder,
};

pub use zone_status::*;
