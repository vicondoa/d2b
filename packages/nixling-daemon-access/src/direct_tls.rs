use async_trait::async_trait;
use nixling_constellation_core::ProviderId;
use nixling_constellation_provider::{
    error::{ProviderError, ProviderResult},
    provider::DaemonAccessTransport,
    types::{DaemonAccessMode, TransportSession, TransportTarget},
};

/// Declared direct-TLS daemon-access slot.
#[derive(Debug, Clone, Default)]
pub struct DirectTlsDaemonAccess;

impl DirectTlsDaemonAccess {
    /// Construct the fail-closed direct-TLS slot.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DaemonAccessTransport for DirectTlsDaemonAccess {
    fn transport_id(&self) -> ProviderId {
        ProviderId::parse("direct-tls-daemon-access").expect("static provider id is valid")
    }

    fn mode(&self) -> DaemonAccessMode {
        DaemonAccessMode::DirectTls
    }

    async fn connect(&self, _endpoint: TransportTarget) -> ProviderResult<TransportSession> {
        Err(ProviderError::unsupported(
            "direct-tls daemon-access not implemented in this wave",
        ))
    }
}
