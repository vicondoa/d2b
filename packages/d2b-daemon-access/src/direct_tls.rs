use async_trait::async_trait;
use d2b_realm_core::{PrincipalId, ProviderId};
use d2b_realm_provider::{
    error::{ProviderError, ProviderResult},
    provider::DaemonAccessTransport,
    types::{DaemonAccessMode, TransportSession, TransportTarget},
};

use crate::{
    DIRECT_TLS_DAEMON_ACCESS_TRANSPORT_ID, DaemonAccessAdmissionSource,
    RemoteDaemonAccessAdmissionSource, RemoteDaemonAccessCredential,
};

/// Declared direct-TLS daemon-access slot.
#[derive(Debug, Clone, Default)]
pub struct DirectTlsDaemonAccess;

impl DirectTlsDaemonAccess {
    /// Construct the fail-closed direct-TLS slot.
    pub fn new() -> Self {
        Self
    }

    /// Build an admission source using this transport's advertised mode/id.
    pub fn admission_source(
        &self,
        credential: RemoteDaemonAccessCredential,
        principal_id: Option<PrincipalId>,
    ) -> DaemonAccessAdmissionSource {
        DaemonAccessAdmissionSource::Remote(RemoteDaemonAccessAdmissionSource::new(
            self.transport_id(),
            self.mode(),
            credential,
            principal_id,
        ))
    }
}

#[async_trait]
impl DaemonAccessTransport for DirectTlsDaemonAccess {
    fn transport_id(&self) -> ProviderId {
        ProviderId::parse(DIRECT_TLS_DAEMON_ACCESS_TRANSPORT_ID)
            .expect("static provider id is valid")
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
