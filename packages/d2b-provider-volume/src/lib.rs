//! The Volume provider crate: the Volume resource type's driver, its spec
//! decoder, its driver declaration, and the family's effects service.
//!
//! The crate owns the Volume type's complete resource knowledge: the
//! preserved volume-local layout leg, the deterministic `VolumeBinding`
//! children one Volume's declared attachments derive, the driver's validate,
//! recover, reconcile, finalize, and delete verbs, the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by, and the family's declared effects service
//! ([`VOLUME_EFFECTS_SERVICE`], U7).
//!
//! Everything the driver needs from outside arrives through the driver
//! effect port ([`VolumeDriverEffects`]): the layout effect over the
//! preserved volume-local controller, and the durable layout probe recover
//! reads. The production implementation is this crate's own
//! [`VolumeEffectsService`], built from the daemon-supplied declared facets
//! ([`VolumeEffectFacets`]); the daemon holds no volume effect
//! implementation and no host state.

#![deny(missing_docs)]

mod driver;
mod effects_service;
mod facets;

// The scripted `VolumeRuntime` recording double is needed both by
// external crates (which opt in via the `test-support` feature) and by this
// crate's own tests. Gating on `any(test, feature = "test-support")` makes it
// available automatically when compiling this crate's tests, so
// `cargo test -p d2b-provider-volume` works without anyone having to remember
// `--features test-support`.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use driver::{
    VOLUME_CREATIONS, VOLUME_TYPE_NAME, VolumeDriverArgs, VolumeDriverEffects, volume_descriptor,
    volume_provider_declaration, volume_spec_decoder,
};
pub use effects_service::{
    VOLUME_EFFECTS_SERVICE, VolumeEffectsService, VolumeEffectsServiceFactory,
};
pub use facets::{VolumeEffectFacets, VolumeRuntime};
