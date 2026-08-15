//! Canonical `Provider/transport-azure-relay` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod audit;
pub mod backpressure;
pub mod credential_client;
pub mod gateway_compat;
pub mod metrics;
pub mod reconnect;
pub mod relay_transport;
pub mod transport_settings;

pub use audit::{RelayAuditEvent, RelayAuditOperation, RelayAuditOutcome};
pub use backpressure::{BackpressureError, CreditWindow};
pub use credential_client::{
    RelayCredentialError, RelayCredentialLease, RelayCredentialMaterial, RelayCredentialPort,
    RelayCredentialRole, RelaySecret,
};
pub use metrics::{RelayMetricEvent, RelayMetricOutcome};
pub use reconnect::{ReconnectBackoff, ReconnectDecision};
pub use relay_transport::{
    AzureRelayTransportProvider, RelayAuthenticatedPeer, RelayConnection, RelayEndpoint,
    RelayEnrollmentChallenge, RelayEnrollmentProof, RelayEnrollmentVerifier, RelayFrame, RelayRole,
    RelaySessionPhase, RelaySocket, RelaySocketConnector, RelayTransportConfig,
    RelayTransportError,
};
pub use transport_settings::{RelayTransportSettings, RelayTransportSettingsError};

/// Stable Provider implementation identifier.
pub const AZURE_RELAY_IMPLEMENTATION_ID: &str = "azure-relay";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/transport-azure-relay";
