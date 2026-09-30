//! Canonical `Provider/transport-azure-relay` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod auth;
pub mod backpressure;
pub mod credential_client;
pub mod graph_binding;
pub mod guest_credential;
pub mod guest_zone_link;
pub mod relay_transport;
pub mod transport_settings;

pub use backpressure::{BackpressureError, CreditWindow};
pub use credential_client::{
    GraphBoundCredentialClient, GraphBoundCredentialError, GraphBoundCredentialScope,
    MAX_ACTIVE_RELAY_LEASES, MAX_RELAY_BINDING_COMPONENT_BYTES, MAX_RELAY_LEASE_TTL_MS,
    RelayCredentialBinding, RelayCredentialError, RelayCredentialLease, RelayCredentialMaterial,
    RelayCredentialPort, RelayCredentialRole, RelaySecret, ScopedCredentialClient,
    ScopedCredentialRequest,
};
pub use graph_binding::{
    AdmittedRelayDelivery, AdmittedTransportBinding, AdmittedTransportRoute, CarriageClass,
    ControlPlaneInjectionRefusal, ControlPlaneRequest, ControlRouteToken,
    MAX_ADMITTED_TRANSPORT_BINDINGS, RELAY_CREDENTIAL_AUDIENCE, RelayCarriageHandle,
    RelationshipFence, RelationshipPhase, TransportAttachEvidence, TransportAttachRefusal,
    TransportBindingRefusal, TransportBindingRegistry, TransportControlOperation, admit_attach,
    admit_relay_credential_delivery, admit_relay_delivery, classify_carriage,
};
pub use guest_credential::{
    CredentialEnvelopeMeta, CredentialError, CredentialFilePolicy, GATEWAY_CREDENTIAL_MODE,
    GATEWAY_CREDENTIAL_SCHEMA_VERSION, GATEWAY_SEAL_KEY_LEN, GATEWAY_SEAL_KEY_MODE,
    GatewayCredential, GatewayCredentialMaterial, GatewayGuestCredentialPort,
    GatewayTransportConfiguration, SealingKey,
};
pub use guest_zone_link::{
    GatewayGuestZoneLinkError, GatewayGuestZoneLinkRuntime,
    GatewayGuestZoneLinkTransportConfig, RelayCarriageRequest, ZoneLinkCredentialRefusal,
};
pub use relay_transport::{
    AzureRelaySocketConnector, AzureRelayTransportProvider, GraphBoundRelayError,
    MAX_RELAY_CA_BYTES, MAX_RELAY_GENERATION_FENCES, MAX_RELAY_WS_WRITE_BUFFER_BYTES,
    RelayAuthenticatedPeer, RelayComponentSessionTransport, RelayConnection, RelayEndpoint,
    RelayEnrollmentChallenge, RelayEnrollmentProof, RelayEnrollmentVerifier, RelayFrame, RelayRole,
    RelaySessionPhase, RelaySocket, RelaySocketConnector, RelayTransportConfig,
    RelayTransportError, RelayTransportObservation,
};
pub use transport_settings::{RelayTransportSettings, RelayTransportSettingsError};

/// Stable Provider implementation identifier.
pub const AZURE_RELAY_IMPLEMENTATION_ID: &str = "azure-relay";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/transport-azure-relay";
