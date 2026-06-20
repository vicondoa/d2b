use async_trait::async_trait;
use nixling_constellation_core::{PrincipalId, ProviderId};
use nixling_constellation_provider::{
    error::{ProviderError, ProviderResult},
    provider::DaemonAccessTransport,
    types::{DaemonAccessMode, TransportSession, TransportTarget},
};

use crate::{
    DaemonAccessAdmissionSource, RELAY_DAEMON_ACCESS_TRANSPORT_ID,
    RelayDaemonAccessAdmissionSource, RelayDaemonAccessCredential,
};

/// Declared relay daemon-access slot.
#[derive(Debug, Clone, Default)]
pub struct RelayDaemonAccess;

impl RelayDaemonAccess {
    /// Construct the fail-closed relay slot.
    pub fn new() -> Self {
        Self
    }

    /// Build an admission source using this transport's advertised mode/id.
    pub fn admission_source(
        &self,
        credential: RelayDaemonAccessCredential,
        principal_id: Option<PrincipalId>,
    ) -> DaemonAccessAdmissionSource {
        DaemonAccessAdmissionSource::Relay(RelayDaemonAccessAdmissionSource::new(
            self.transport_id(),
            self.mode(),
            credential,
            principal_id,
        ))
    }
}

#[async_trait]
impl DaemonAccessTransport for RelayDaemonAccess {
    fn transport_id(&self) -> ProviderId {
        ProviderId::parse(RELAY_DAEMON_ACCESS_TRANSPORT_ID).expect("static provider id is valid")
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
