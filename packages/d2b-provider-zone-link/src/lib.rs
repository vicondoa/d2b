//! The ZoneLink provider crate: the ZoneLink resource type's driver declaration.
//!
//! The crate owns the ZoneLink type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `ZoneLink` is the link between two zones: the driver converges it as
//! metadata once its desired state is admitted, and the crate carries the
//! link's crash-safe enrollment-and-session state machine (`zone_links`) and
//! its durable cursor adoption (`zonelink`).

mod driver;

pub mod zone_links;
pub mod zonelink;

pub use driver::zone_link_descriptor;

pub use zone_links::*;
pub use zonelink::*;
