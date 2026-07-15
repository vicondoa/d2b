//! Canonical local transport providers over pre-authorized endpoints.
//!
//! This crate never discovers endpoints or turns human names into paths. A
//! provider receives a closed [`TransportBinding`] table and delegates all
//! endpoint work to an injected asynchronous [`LocalEndpointPort`]. The port
//! sees only opaque bundle/lease identifiers or validated owned descriptors.
//! Reaching an endpoint is deliberately reported as reachability only;
//! `ComponentSession` remains the authentication and descriptor-policy owner.

#![forbid(unsafe_code)]

mod binding;
mod connector;
mod provider;

#[cfg(test)]
mod tests;

pub use binding::{
    AttachmentCapability, AuthenticationOwner, BundleEndpointId, EndpointLeaseId,
    EndpointProvenance, EndpointSource, LocalTransportKind, OwnedEndpointDescriptor,
    OwnedEndpointError, TransportBinding, TransportCapabilityProfile,
};
pub use connector::{
    EndpointCloseRequest, EndpointCloseResult, EndpointCloseState, EndpointConnectRequest,
    EndpointConnection, EndpointInspectRequest, EndpointObservation, EndpointObservationState,
    EndpointPortError, LocalEndpointPort, ReachabilityEvidence,
};
pub use provider::{
    LocalTransportClock, LocalTransportConfigurationError, LocalTransportLimits,
    LocalTransportProvider, MAX_ACTIVE_LOCAL_TRANSPORTS, MAX_LOCAL_TRANSPORT_BINDINGS,
    SystemTransportClock, local_transport_capabilities,
};
