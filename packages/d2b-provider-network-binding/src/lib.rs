//! The NetworkBinding provider crate: the `NetworkBinding` resource type's
//! driver, its spec decoder, its read-side row helpers, its driver
//! declaration, and the implementation of the family's driver effects.
//!
//! The crate owns the binding type's complete row-side resource knowledge: the
//! membership value one committed row derives, the identity the driver fences
//! every readiness report to, the driver's validate, recover, reconcile, drain,
//! and delete verbs, and the [`DriverDescriptor`](d2b_resource_types::DriverDescriptor)
//! the plane registers the type by.
//!
//! What it deliberately does not own is the fabric. One Network's host fabric -
//! its bridges, routes, ownership markers, NetworkManager policy, and single
//! ownership-scoped firewall projection - belongs to the Network provider and
//! is realized once per `(Zone, Network, execution target)`; the per-consumer
//! admission of a membership belongs to that provider's own source decision.
//! This crate serves the row and drives the four operations a membership's
//! realization has - observe, join, drain, release - through the effect port
//! ([`crate::driver::NetworkBindingDriverEffects`]), never by reaching host
//! state itself.
//!
//! The family's effects (U6) are implemented by this crate itself
//! ([`crate::effects_service`]) over the declared facets the composition root
//! supplies, so no externally built port appears at any construction site
//! (R2). The family declares no hosted zone-plane service: the driver factory is
//! the whole surface, and a `ServiceDecl` nothing hosts would advertise a
//! capability that does not exist.

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
    FabricDrain, FabricMembership, FabricMembershipState, FabricRelease, MembershipHandle,
    MembershipIdentity, NETWORK_BINDING_CREATIONS, NETWORK_BINDING_PROVIDER_REF,
    NETWORK_BINDING_READS, NETWORK_BINDING_TYPE_NAME, NetworkBindingDriverArgs,
    NetworkBindingDriverEffects, NetworkBindingReadinessFence, NetworkBindingStatusResource,
    binding_descriptor, binding_spec_decoder,
};
pub use effects_service::NetworkBindingEffectsService;
pub use facets::{
    FabricDrainSource, FabricJoinSource, FabricObserveSource, FabricReleaseSource,
    NetworkBindingEffectFacets,
};
pub use row_readers::{binding_readiness_current, parsed_binding_spec, parsed_fabric_generation};
