//! The EndpointBinding provider crate: the `EndpointBinding` resource
//! type's driver, its spec decoder, its read-side row helpers, its driver
//! declaration, and the implementation of the family's driver effects.
//!
//! The crate owns the binding ROW's complete resource knowledge: the
//! exact-endpoint selector rule that keeps a relationship from naming a
//! neighbourhood, the endpoint-policy admission taken from the Endpoint
//! provider's own declaration, the verified connected or listening
//! descriptor a delivery is established on, the pinned-identity fence that
//! invalidates a delivery whose endpoint was replaced underneath it, the
//! driver's validate, recover, reconcile, pre-drain, and delete verbs, and
//! the [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! The row itself is the contract's own
//! [`EndpointBindingSpec`](d2b_contracts_resource::v3::endpoint_binding::EndpointBindingSpec).
//! The crate reads a row through exactly one seam and never copies a row
//! field into a local struct, so the contract this provider serves and the
//! provider cannot drift apart.
//!
//! The crate is the row's owner and server, NOT the consumer: which consumer
//! an endpoint admits, which attachment kind reaches it, and which facets
//! realize it are [`d2b_provider_endpoint`]'s own binding knowledge, and
//! this crate reuses them rather than restating them.
//!
//! The family's driver effects (U6) are implemented by this crate itself
//! ([`EndpointBindingEffectsService`]): the exact endpoint verification, the
//! delivery, the pre-drain fence, the attachment observation the teardown
//! gate reads, and the release all cross the provider boundary as the
//! declared [`EndpointBindingEffectFacets`] the composition root supplies.
//! The daemon hosts the family's declared effects service
//! ([`BINDING_EFFECTS_SERVICE`]) per zone from the family's registered
//! factory; no externally built port appears at any construction site (R2).

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
    BINDING_PROVIDER_REF, ENDPOINT_BINDING_CREATIONS, ENDPOINT_BINDING_READS,
    ENDPOINT_BINDING_TYPE_NAME, ENDPOINT_DIRECTORY_LISTABLE, ENDPOINT_IDENTITY_REPLACED,
    ENDPOINT_NOT_READY, ENDPOINT_TYPE_NAME, EndpointBindingDelivery, EndpointDeliveryTarget,
    EndpointBindingDriverArgs, EndpointBindingDriverEffects, EndpointBindingReadinessFence,
    EndpointBindingStatusResource, EndpointSelectorRejection, binding_descriptor,
    binding_spec_decoder, committed_source_selector, exact_endpoint_selector,
};
pub use effects_service::{
    BINDING_EFFECTS_SERVICE, EndpointBindingEffectsService, EndpointBindingEffectsServiceFactory,
};
pub use facets::{
    EndpointAttachmentSource, EndpointBindingEffectFacets, EndpointDeliverSource,
    EndpointFenceSource, EndpointReleaseSource, EndpointVerifySource,
};
pub use row_readers::{binding_readiness_current, names_one_exact_endpoint, parsed_binding_spec};
