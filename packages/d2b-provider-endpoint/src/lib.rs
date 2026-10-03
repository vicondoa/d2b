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
//! own vocabularies, and the surfaces the daemon owns - the host socket
//! effect for the binding-owned virtiofsd socket, the two row-evidence
//! probes, the Provider vocabularies that admit a Provider-committed shape,
//! and the daemon's private host observation - cross the provider boundary as
//! the declared [`crate::facets::EndpointEffectFacets`] the composition root
//! supplies.
//! The daemon hosts the family's declared effects service
//! ([`crate::effects_service::ENDPOINT_EFFECTS_SERVICE`]) per zone from the
//! family's registered factory; no externally built port appears at any
//! construction site (R2).
//!
//! A declaring Provider that realizes a shape this crate knows nothing about
//! commits it through [`EndpointPurposeVocabulary::committed_endpoint_shape`]
//! and owns every constant and every field of that match itself: this crate
//! never names a Provider, a Provider type, or a Provider's vocabulary
//! (KTD5). A shape committed that way is realized behind one of the two
//! evidence kinds the shape names - the daemon's private observation of a
//! host socket ([`crate::facets::HostSocketEvidenceSource`]) or the
//! generation-fenced producer row the Endpoint itself declares - and the
//! incarnation token it publishes is a digest over committed facts and the
//! Provider's own reconnect generation, never over a locator (KTD8).

#![deny(missing_docs)]

mod binding;
mod driver;
mod effects_service;
mod facets;


#[cfg(any(test, feature = "test-support"))]
/// Recording doubles shared with downstream crates unit tests.
pub mod test_support;

pub use driver::{
    CommittedEndpointShape, EndpointConnectability, EndpointDriver, EndpointDriverArgs,
    EndpointDriverEffects, EndpointDriverError, EndpointDriverFactory, EndpointDriverStatus,
    EndpointPurposeVocabulary, EndpointRealization, GuestControlProducer,
    ProviderRealizationEvidence, VIRTIOFSD_PURPOSE, endpoint_child_support_ceiling,
    endpoint_descriptor, endpoint_realization, endpoint_spec_decoder,
    provider_committed_endpoint_shape,
};
pub use effects_service::{
    ENDPOINT_EFFECTS_SERVICE, EndpointEffectsService, EndpointEffectsServiceFactory,
    device_worker_endpoint_class, device_worker_purpose, guest_control_producer,
    guest_control_purpose,
};
pub use facets::{
    CommittedEndpointShapeSource, DeviceWorkerEvidenceSource, EndpointAccessDispatch,
    EndpointAccessDispatchError, EndpointEffectFacets, EndpointSocketSource,
    GuestVmmEvidenceSource, HostSocketEvidenceSource, MIN_REALIZATION_NONCE_CHARS,
    RealizationHandle, UnwiredCommittedShapes, UnwiredEndpointAccess,
    UnwiredHostSocketEvidence,
};

pub use binding::{
    ENDPOINT_BINDING_TYPE_NAME, AdmittedEndpointBinding, BindingDeliveryProjection, BindingReadiness,
    DeclaredEndpointBinding, DeliveryFenceViolation, DeliveryForm, EndpointAccessObservation,
    EndpointBindingAdmission, EndpointBindingDriver, EndpointBindingDriverArgs,
    EndpointBindingDriverError, EndpointBindingDriverFactory, EndpointBindingDriverStatus,
    EndpointBindingError, EndpointBindingRegistry, EndpointBindingRow, EndpointConsumerTarget,
    EndpointDelivery, EndpointDeliveryRefusal, EndpointProvenance, EndpointSocketIdentity,
    EndpointSourceKey, EndpointTeardown, ParentInputOutcome, binding_row_name,
    canonical_binding_row, canonical_binding_rows, declared_attachment, declared_delivery_form,
    declared_endpoint_bindings, endpoint_access_request, endpoint_binding_descriptor,
    endpoint_binding_spec_decoder, endpoint_binding_support,
    endpoint_binding_support_ceiling, endpoint_delivery_slot, endpoint_grants_observe,
    ensure_realizable, expected_bindings_for_process, fence_delivery_environment,
    fence_delivery_environment_all, fence_delivery_payload, fence_delivery_payload_all,
    required_right_bits,
};

/// The Endpoint ResourceType spec and status shapes owned by this crate.
pub mod endpoint;