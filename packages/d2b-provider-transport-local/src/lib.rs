//! Canonical local transport providers over pre-authorized endpoints.
//!
//! This crate never discovers endpoints or turns human names into paths. A
//! provider receives a closed [`TransportBinding`] table and delegates all
//! endpoint work to an injected asynchronous [`LocalEndpointPort`]. The port
//! sees only opaque bundle/lease identifiers or validated owned descriptors.
//! Reaching an endpoint is deliberately reported as reachability only;
//! `ComponentSession` remains the authentication and descriptor-policy owner.
//! Connected descriptors remain RAII-owned in a single-use handoff registry
//! until the session layer claims the canonical provider handle.

#![forbid(unsafe_code)]

mod binding;
mod connector;
mod factory;
mod production;
mod provider;

#[cfg(test)]
mod tests;

pub use binding::{
    AttachmentCapability, AuthenticationOwner, BundleEndpointId,
    CLOUD_HYPERVISOR_VSOCK_FACTORY_KEY, CLOUD_HYPERVISOR_VSOCK_IMPLEMENTATION_ID,
    CloudHypervisorVsockPort, EndpointLeaseId, EndpointProvenance, EndpointSource,
    LocalTransportKind, NATIVE_VSOCK_FACTORY_KEY, NATIVE_VSOCK_IMPLEMENTATION_ID,
    OwnedEndpointCapability, OwnedEndpointDescriptor, OwnedEndpointError, TransportBinding,
    TransportCapabilityProfile, UNIX_SEQPACKET_FACTORY_KEY, UNIX_SEQPACKET_IMPLEMENTATION_ID,
    UNIX_STREAM_FACTORY_KEY, UNIX_STREAM_IMPLEMENTATION_ID,
};
pub use connector::{
    EndpointCloseRequest, EndpointCloseResult, EndpointCloseState, EndpointConnectRequest,
    EndpointConnection, EndpointConnectionMetadata, EndpointInspectRequest, EndpointObservation,
    EndpointObservationState, EndpointPortError, LocalEndpointPort, OwnedEndpointConnection,
    OwnedLocalTransport, ReachabilityEvidence, TransportHandoffError,
};
pub use factory::{
    LocalTransportConstruction, LocalTransportFactory, LocalTransportFactoryError,
    MAX_LOCAL_TRANSPORT_FACTORY_PROVIDERS,
};
pub use production::{
    EndpointCapabilityId, EndpointResolveRequest, LocalEndpointResolver, TokioLocalEndpointPort,
};
pub use provider::{
    LocalTransportClock, LocalTransportConfigurationError, LocalTransportHandoffRegistry,
    LocalTransportLimits, LocalTransportProvider, MAX_ACTIVE_LOCAL_TRANSPORTS,
    MAX_LOCAL_TRANSPORT_BINDINGS, SystemTransportClock, local_transport_capabilities,
};
