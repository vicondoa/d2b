//! Fail-closed compatibility adapter for the retired display relay path.

use async_trait::async_trait;
use d2b_gateway::{
    DisplayListener, DisplaySessionContext, GatewayError, ListenerHandle, SessionBinding,
    SessionSecret,
};
use d2b_provider_relay::{LocalTarget, RelayCredential, RelayEndpoint};

use crate::NowFn;

/// Retained constructor surface. Operations reject so callers cannot bypass
/// the typed `d2b.provider.v2` display provider.
pub struct RelayDisplayListener;

impl RelayDisplayListener {
    pub fn new(
        _endpoint: RelayEndpoint,
        _credential: RelayCredential,
        _target: LocalTarget,
        _ttl_secs: u64,
        _ca_pem: Option<Vec<u8>>,
        _now: NowFn,
    ) -> Self {
        Self
    }
}

#[async_trait]
impl DisplayListener for RelayDisplayListener {
    async fn arm(
        &self,
        _ctx: &DisplaySessionContext,
        _binding: &SessionBinding,
        _secret: &SessionSecret,
    ) -> Result<ListenerHandle, GatewayError> {
        Err(GatewayError::ProviderAllocationFailed)
    }

    async fn await_handshake(&self, _handle: &ListenerHandle) -> Result<(), GatewayError> {
        Err(GatewayError::ProviderAllocationFailed)
    }

    async fn close(&self, _handle: &ListenerHandle) -> Result<(), GatewayError> {
        Err(GatewayError::ProviderAllocationFailed)
    }
}
