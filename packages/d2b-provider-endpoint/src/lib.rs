//! The Endpoint provider crate: the Endpoint resource type's driver, its
//! spec decoder, its driver declaration, and the implementation of the
//! family's driver effects.
//!
//! The crate owns the Endpoint type's complete resource knowledge: the closed
//! set of endpoint shapes the v3 plane realizes, the admission rules that
//! classify a stored spec onto that set, the driver's validate, recover,
//! reconcile, finalize, and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! The family's driver effects (U6) are implemented by this crate itself
//! ([`crate::effects_service`]): the purpose derivations classify one
//! purpose onto the realization the plane owns from the declaring providers'
//! own vocabularies, and the daemon-owned realization surfaces - the host
//! socket effect for the binding-owned virtiofsd socket and the two
//! row-evidence probes - cross the provider boundary as the declared
//! [`crate::facets::EndpointEffectFacets`] the composition root supplies.
//! The daemon hosts the family's declared effects service
//! ([`crate::effects_service::ENDPOINT_EFFECTS_SERVICE`]) per zone from the
//! family's registered factory; no externally built port appears at any
//! construction site (R2).

#![deny(missing_docs)]

mod driver;

mod effects_service;
mod facets;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use driver::{
    EndpointDriver, EndpointDriverArgs, EndpointDriverEffects, EndpointDriverError,
    EndpointDriverFactory, EndpointDriverStatus, EndpointPurposeVocabulary, EndpointRealization,
    GuestControlProducer, VIRTIOFSD_PURPOSE, endpoint_descriptor, endpoint_realization,
    endpoint_spec_decoder,
};
pub use effects_service::{
    ENDPOINT_EFFECTS_SERVICE, EndpointEffectsService, EndpointEffectsServiceFactory,
    device_worker_endpoint_class, device_worker_purpose, guest_control_producer,
    guest_control_purpose,
};
pub use facets::{DeviceWorkerEvidenceSource, EndpointEffectFacets, EndpointSocketSource, GuestVmmEvidenceSource};

/// The Endpoint ResourceType spec and status shapes owned by this crate.
pub mod endpoint;