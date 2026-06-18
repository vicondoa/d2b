use async_trait::async_trait;
use nixling_constellation_core::ProviderId;
use nixling_constellation_provider::{
    error::{ProviderError, ProviderResult},
    provider::DaemonAccessTransport,
    types::{DaemonAccessMode, TransportSession, TransportTarget},
};

/// Declared relay daemon-access slot.
#[derive(Debug, Clone, Default)]
pub struct RelayDaemonAccess;

impl RelayDaemonAccess {
    /// Construct the fail-closed relay slot.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DaemonAccessTransport for RelayDaemonAccess {
    fn transport_id(&self) -> ProviderId {
        ProviderId::parse("relay-daemon-access").expect("static provider id is valid")
    }

    fn mode(&self) -> DaemonAccessMode {
        DaemonAccessMode::Relay
    }

    async fn connect(&self, _endpoint: TransportTarget) -> ProviderResult<TransportSession> {
        Err(ProviderError::unsupported(
            "relay daemon-access not implemented in this wave",
        ))
    }
}
