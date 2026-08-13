use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use d2b_contracts::v3::ResourceRef;
use d2b_provider_transport_azure_relay::{
    AzureRelayTransportProvider, CreditWindow, RelayAuthenticatedPeer, RelayCredentialError,
    RelayCredentialLease, RelayCredentialMaterial, RelayCredentialPort, RelayCredentialRole,
    RelayEndpoint, RelayFrame, RelayRole, RelaySecret, RelaySocket, RelaySocketConnector,
    RelayTransportConfig, RelayTransportError, RelayTransportSettings,
};

#[derive(Default)]
struct FakeSocket {
    frames: Mutex<VecDeque<RelayFrame>>,
}

#[async_trait]
impl RelaySocket for FakeSocket {
    async fn send(&self, frame: RelayFrame) -> Result<(), RelayTransportError> {
        self.frames.lock().unwrap().push_back(frame);
        Ok(())
    }

    async fn receive(&self) -> Result<Option<RelayFrame>, RelayTransportError> {
        Ok(self.frames.lock().unwrap().pop_front())
    }

    async fn close(&self) -> Result<(), RelayTransportError> {
        Ok(())
    }
}

struct FakeConnector {
    socket: Arc<FakeSocket>,
}

#[async_trait]
impl RelaySocketConnector for FakeConnector {
    async fn connect(
        &self,
        _: &RelayEndpoint,
        _: RelayRole,
        _: &RelayCredentialLease,
    ) -> Result<Arc<dyn RelaySocket>, RelayTransportError> {
        Ok(Arc::clone(&self.socket) as Arc<dyn RelaySocket>)
    }
}

struct FakeCredentials;

#[async_trait]
impl RelayCredentialPort for FakeCredentials {
    async fn acquire(
        &self,
        role: RelayCredentialRole,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Ok(RelayCredentialLease::new(
            RelayCredentialMaterial::SasToken(RelaySecret::new(b"token".to_vec()).unwrap()),
            role,
            1_000,
        ))
    }

    async fn revoke(&self, _: RelayCredentialLease) -> Result<(), RelayCredentialError> {
        Ok(())
    }
}

fn provider() -> AzureRelayTransportProvider<FakeCredentials, FakeConnector> {
    AzureRelayTransportProvider::new(
        RelayTransportConfig {
            execution_ref: ResourceRef::parse("Guest/gateway").unwrap(),
            network_ref: ResourceRef::parse("Network/relay").unwrap(),
            max_concurrent_sessions: 4,
            connect_timeout_seconds: 30,
        },
        RelayEndpoint {
            settings: RelayTransportSettings::new("relns-d2b-prod", "hc-d2b-k2").unwrap(),
        },
        Arc::new(FakeCredentials),
        Arc::new(FakeConnector {
            socket: Arc::new(FakeSocket::default()),
        }),
    )
    .unwrap()
}

#[tokio::test]
async fn sender_roundtrip_is_bounded_and_relay_has_no_local_admin() {
    let provider = provider();
    let connection = provider.open(RelayRole::Sender, 1_000).await.unwrap();
    connection
        .send(RelayFrame::new(b"hello".to_vec()).unwrap())
        .await
        .unwrap();
    assert!(connection.receive().await.unwrap().is_some());
    let peer = RelayAuthenticatedPeer;
    assert!(!peer.local_admin());
}

#[test]
fn helper_surface_does_not_reintroduce_unbounded_window() {
    assert!(CreditWindow::new(256 * 1024).is_ok());
}
