//! Canonical Azure Relay transport provider.
//!
//! Azure credentials and endpoint coordinates remain behind an injected,
//! co-located [`RelayControlPort`]. Canonical provider requests and results
//! carry only opaque bounded identifiers and closed state.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

mod factory;
mod port;
mod production;
#[cfg(test)]
mod production_tests;
mod provider;
#[cfg(test)]
mod tests;

pub use factory::{
    AzureRelayFactoryBuildError, AzureRelayFactoryEntry, AzureRelayProviderFactory,
    azure_relay_factory_key, azure_relay_implementation_id,
};
pub use port::{
    RELAY_ACCEPT_QUEUE_CAPACITY, RELAY_MAX_CREDENTIAL_TTL_SECS, RELAY_MAX_FRAME_BYTES,
    RELAY_MAX_PROLOGUE_BYTES, RELAY_MAX_RECONNECT_BACKOFF_MS, RELAY_RECONNECT_STABLE_RESET_MS,
    RELAY_SENDER_RETRY_DELAY_MS, RELAY_SENDER_RETRY_LIMIT, RelayAdoptRequest, RelayCloseOutcome,
    RelayCloseRequest, RelayControlPort, RelayExpectedResource, RelayIdentifierError,
    RelayInspectRequest, RelayInspection, RelayOpenRequest, RelayPortCapabilities,
    RelayPortFailure, RelayRendezvousId, RelayResource, RelayResourceState, RelayTransportLimits,
};
pub use production::{
    AzureRelayBinding, ProductionRelayControlPort, RelayCredentialLease, RelayCredentialMaterial,
    RelayCredentialSource, RelayCredentialSourceFailure, RelayCredentialUse, RelayFrame,
    RelayProductionBuildError, RelaySecret, RelaySocket, RelaySocketConnectRequest,
    RelaySocketConnection, RelaySocketConnector, RelaySocketEvent, RelaySocketFailure,
    RelaySocketRole, TungsteniteRelaySocketConnector,
};
pub use provider::{
    AZURE_RELAY_IMPLEMENTATION_ID, AzureRelayConfiguration, AzureRelayProviderBuildError,
    AzureRelayTransportProvider, azure_relay_capabilities,
};
