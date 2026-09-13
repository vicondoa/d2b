//! The Volume provider crate: the Volume resource type's driver, its spec
//! decoder, and its driver declaration.
//!
//! The crate owns the Volume type's complete resource knowledge: the
//! preserved volume-local layout leg, the deterministic `VolumeBinding`
//! children one Volume's declared attachments derive, the driver's validate,
//! recover, reconcile, finalize, and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! Everything the driver needs from outside arrives through the driver
//! effect port ([`VolumeDriverEffects`]): the layout effect the daemon
//! realizes over the preserved volume-local controller, and the durable
//! layout probe recover reads. The production implementation lives in the
//! daemon behind that port, so the family carries no effect implementation
//! and no host state.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    VOLUME_CREATIONS, VOLUME_TYPE_NAME, VolumeDriverArgs, VolumeDriverEffects, volume_descriptor,
    volume_provider_declaration, volume_spec_decoder,
};
