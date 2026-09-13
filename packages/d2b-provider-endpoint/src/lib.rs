//! The Endpoint provider crate: the Endpoint resource type's driver, its
//! spec decoder, and its driver declaration.
//!
//! The crate owns the Endpoint type's complete resource knowledge: the closed
//! set of endpoint shapes the v3 plane realizes, the admission rules that
//! classify a stored spec onto that set, the driver's validate, recover,
//! reconcile, finalize, and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! Everything the driver needs from outside arrives through the driver
//! effect port ([`EndpointDriverEffects`]): the socket and evidence effects
//! the daemon realizes, plus the per-provider purpose derivations
//! ([`EndpointPurposeVocabulary`]) that say which purposes a declaring
//! provider commits and on which producer. The production implementation
//! lives in the daemon behind that port, so this crate depends on no
//! provider crate.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    EndpointDriver, EndpointDriverArgs, EndpointDriverEffects, EndpointDriverError,
    EndpointDriverFactory, EndpointDriverStatus, EndpointPurposeVocabulary, EndpointRealization,
    GuestControlProducer, endpoint_descriptor, endpoint_realization, endpoint_spec_decoder,
};
