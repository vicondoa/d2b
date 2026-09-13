//! The ZoneLink provider crate: the ZoneLink resource type's driver, its spec decoder,
//! and its driver declaration.
//!
//! The crate owns the ZoneLink type's complete resource knowledge: the driver's
//! validate, recover, reconcile, finalize, and delete verbs and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! `ZoneLink` is the link between two zones. The driver converges it as metadata once its desired state is admitted; the link's crash-safe enrollment-and-session state machine (`zone_links`) and its durable cursor adoption (`zonelink`) live in this crate and plan effects without performing transport work.


mod driver;

pub mod zone_links;
pub mod zonelink;

pub use driver::{
    ZONE_LINK_TYPE_NAME, ZoneLinkDriver, ZoneLinkDriverFactory, zone_link_descriptor,
    zone_link_spec_decoder,
};

pub use zone_links::*;

pub use zonelink::*;
