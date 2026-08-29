//! Canonical `Provider/transport-azure-relay` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod audit;
pub mod auth;
pub mod backpressure;
pub mod credential_client;
pub mod metrics;
pub mod reconnect;
pub mod relay_transport;
pub mod transport_settings;

pub use audit::{RelayAuditEvent, RelayAuditOperation, RelayAuditOutcome};
pub use backpressure::{BackpressureError, CreditWindow};
pub use credential_client::{
    MAX_ACTIVE_RELAY_LEASES, MAX_RELAY_BINDING_COMPONENT_BYTES, MAX_RELAY_LEASE_TTL_MS,
    RelayCredentialBinding, RelayCredentialError, RelayCredentialLease, RelayCredentialMaterial,
    RelayCredentialPort, RelayCredentialRequest, RelayCredentialRole, RelaySecret,
};
pub use metrics::{RelayMetricEvent, RelayMetricOutcome};
pub use reconnect::{ReconnectBackoff, ReconnectDecision};
pub use relay_transport::{
    AzureRelaySocketConnector, AzureRelayTransportProvider, MAX_RELAY_CA_BYTES,
    MAX_RELAY_GENERATION_FENCES, MAX_RELAY_WS_WRITE_BUFFER_BYTES, RelayAuthenticatedPeer,
    RelayComponentSessionTransport, RelayConnection, RelayEndpoint, RelayEnrollmentChallenge,
    RelayEnrollmentProof, RelayEnrollmentVerifier, RelayFrame, RelayRole, RelaySessionPhase,
    RelaySocket, RelaySocketConnector, RelayTransportConfig, RelayTransportError,
};
pub use transport_settings::{RelayTransportSettings, RelayTransportSettingsError};

/// Stable Provider implementation identifier.
pub const AZURE_RELAY_IMPLEMENTATION_ID: &str = "azure-relay";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/transport-azure-relay";
