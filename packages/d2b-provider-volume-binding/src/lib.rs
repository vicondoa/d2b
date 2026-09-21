//! The VolumeBinding provider crate: the VolumeBinding resource type's
//! driver, its spec decoder, its read-side row helpers, its driver
//! declaration, and the implementation of the family's driver effects.
//!
//! The crate owns the binding type's complete resource knowledge: the
//! derived virtiofsd worker plan the frozen `volume-virtiofs` contract
//! admits, the binding-owned worker Process and Endpoint children one
//! binding mints, the fenced readiness projection its actor publishes, the
//! driver's validate, recover, reconcile, finalize, and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! The family's driver effects (U6) are implemented by this crate itself
//! ([`crate::effects_service`]): the serving-socket probe, the socket
//! removal, and the guest-mount observation the daemon realizes cross the
//! provider boundary as the declared [`crate::facets::BindingEffectFacets`]
//! the composition root supplies. The daemon hosts the family's declared
//! effects service ([`crate::effects_service::BINDING_EFFECTS_SERVICE`])
//! per zone from the family's registered factory; no externally built port
//! appears at any construction site (R2).

#![deny(missing_docs)]

#[cfg(any(test, feature = "test-support"))]
/// Recording test doubles shared with downstream crates' unit tests, gated
/// behind the `test-support` Cargo feature so production consumers never
/// pull them in.
pub mod test_support;

mod driver;
mod effects_service;
mod facets;
mod row_readers;

pub use driver::{
    BINDING_CREATIONS, BINDING_TYPE_NAME, BindingDriverArgs, BindingDriverEffects,
    binding_descriptor, binding_spec_decoder,
};
pub use effects_service::{
    BINDING_EFFECTS_SERVICE, BindingEffectsService, BindingEffectsServiceFactory,
};
pub use facets::{BindingEffectFacets, GuestMountSource, SocketReadySource, SocketRemoveSource};
pub use row_readers::{binding_readiness_current, parsed_binding_spec};