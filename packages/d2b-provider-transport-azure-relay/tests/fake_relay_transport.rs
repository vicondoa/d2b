use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts_zone_session::v3::ResourceRef;
use d2b_provider_transport_azure_relay::{
    AzureRelayTransportProvider, CreditWindow, ReconnectBackoff, RelayAuthenticatedPeer,
    RelayCredentialError, RelayCredentialLease, RelayCredentialMaterial, RelayCredentialPort,
    RelayCredentialRole, RelayEndpoint, RelayEnrollmentProof, RelayEnrollmentVerifier, RelayFrame,
    RelayRole, RelaySecret, RelaySocket, RelaySocketConnector, RelayTransportConfig,
    RelayTransportError, RelayTransportSettings,
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

fn valid_expiry() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 60_000
}

struct FakeEnrollment;

impl RelayEnrollmentVerifier for FakeEnrollment {
    fn verify_enrollment(
        &self,
        transcript: &[u8],
        _: &d2b_provider_transport_azure_relay::RelayEnrollmentChallenge,
    ) -> bool {
        transcript == b"authenticated-enrollment"
    }
}

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
            valid_expiry(),
        ))
    }

    async fn revoke(&self, _: RelayCredentialLease) -> Result<(), RelayCredentialError> {
        Ok(())
    }
}

struct RetryConnector {
    attempts: Arc<Mutex<usize>>,
    failures: usize,
    socket: Arc<FakeSocket>,
}

#[async_trait]
impl RelaySocketConnector for RetryConnector {
    async fn connect(
        &self,
        _: &RelayEndpoint,
        _: RelayRole,
        _: &RelayCredentialLease,
    ) -> Result<Arc<dyn RelaySocket>, RelayTransportError> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        if *attempts <= self.failures {
            return Err(RelayTransportError::Unavailable);
        }
        Ok(Arc::clone(&self.socket) as Arc<dyn RelaySocket>)
    }
}

struct RevokeFails;

#[async_trait]
impl RelayCredentialPort for RevokeFails {
    async fn acquire(
        &self,
        role: RelayCredentialRole,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Ok(RelayCredentialLease::new(
            RelayCredentialMaterial::SasToken(RelaySecret::new(b"token".to_vec()).unwrap()),
            role,
            valid_expiry(),
        ))
    }

    async fn revoke(&self, _: RelayCredentialLease) -> Result<(), RelayCredentialError> {
        Err(RelayCredentialError::Unavailable)
    }
}

struct TrackingSocket {
    closed: Arc<Mutex<bool>>,
}

#[async_trait]
impl RelaySocket for TrackingSocket {
    async fn send(&self, _: RelayFrame) -> Result<(), RelayTransportError> {
        Ok(())
    }

    async fn receive(&self) -> Result<Option<RelayFrame>, RelayTransportError> {
        Ok(None)
    }

    async fn close(&self) -> Result<(), RelayTransportError> {
        *self.closed.lock().unwrap() = true;
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

fn config() -> RelayTransportConfig {
    RelayTransportConfig {
        execution_ref: ResourceRef::parse("Guest/gateway").unwrap(),
        network_ref: ResourceRef::parse("Network/relay").unwrap(),
        max_concurrent_sessions: 4,
        connect_timeout_seconds: 30,
    }
}

fn endpoint() -> RelayEndpoint {
    RelayEndpoint {
        settings: RelayTransportSettings::new("relns-d2b-prod", "hc-d2b-k2").unwrap(),
    }
}

#[tokio::test]
async fn sender_roundtrip_is_bounded_and_relay_has_no_local_admin() {
    let provider = provider();
    let connection = provider.open(RelayRole::Sender, 1_000).await.unwrap();
    assert_eq!(
        connection.phase().await,
        d2b_provider_transport_azure_relay::RelaySessionPhase::EnrollmentCommitted
    );
    let challenge = connection.enrollment_challenge();
    let proof = RelayEnrollmentProof::authenticate(
        &FakeEnrollment,
        b"authenticated-enrollment",
        &challenge,
    )
    .unwrap();
    connection.enroll(proof).await.unwrap();
    connection
        .send(RelayFrame::new(b"hello".to_vec()).unwrap())
        .await
        .unwrap();
    assert_eq!(connection.credit_state().await, (256 * 1024 - 5, 5));
    assert!(connection.receive().await.unwrap().is_some());
    connection.acknowledge(5).await;
    assert_eq!(connection.credit_state().await, (256 * 1024, 0));
    let peer = RelayAuthenticatedPeer;
    assert!(!peer.local_admin());
}

#[tokio::test]
async fn unauthenticated_connection_cannot_send() {
    let connection = provider().open(RelayRole::Sender, 1_000).await.unwrap();
    assert_eq!(
        connection
            .send(RelayFrame::new(b"blocked".to_vec()).unwrap())
            .await,
        Err(RelayTransportError::InvalidSessionTransition)
    );
}

#[tokio::test]
async fn unauthenticated_connection_cannot_receive() {
    let connection = provider().open(RelayRole::Sender, 1_000).await.unwrap();
    assert!(matches!(
        connection.receive().await,
        Err(RelayTransportError::InvalidSessionTransition)
    ));
}

#[tokio::test]
async fn reconnect_policy_is_used_by_provider_open() {
    let attempts = Arc::new(Mutex::new(0));
    let provider = AzureRelayTransportProvider::new(
        RelayTransportConfig {
            max_concurrent_sessions: 1,
            ..config()
        },
        endpoint(),
        Arc::new(FakeCredentials),
        Arc::new(RetryConnector {
            attempts: Arc::clone(&attempts),
            failures: 2,
            socket: Arc::new(FakeSocket::default()),
        }),
    )
    .unwrap();
    provider
        .open_with_backoff(
            RelayRole::Sender,
            1_000,
            ReconnectBackoff::with_limits(0, 1, 3, 1),
        )
        .await
        .unwrap();
    assert_eq!(*attempts.lock().unwrap(), 3);
}

#[tokio::test]
async fn failed_credential_revoke_closes_connected_socket() {
    let closed = Arc::new(Mutex::new(false));
    let socket = Arc::new(TrackingSocket {
        closed: Arc::clone(&closed),
    });

    struct TrackingConnector {
        socket: Arc<TrackingSocket>,
    }

    #[async_trait]
    impl RelaySocketConnector for TrackingConnector {
        async fn connect(
            &self,
            _: &RelayEndpoint,
            _: RelayRole,
            _: &RelayCredentialLease,
        ) -> Result<Arc<dyn RelaySocket>, RelayTransportError> {
            Ok(Arc::clone(&self.socket) as Arc<dyn RelaySocket>)
        }
    }

    let provider = AzureRelayTransportProvider::new(
        config(),
        endpoint(),
        Arc::new(RevokeFails),
        Arc::new(TrackingConnector { socket }),
    )
    .unwrap();
    assert!(matches!(
        provider.open(RelayRole::Sender, 1_000).await,
        Err(RelayTransportError::CredentialUnavailable)
    ));
    assert!(*closed.lock().unwrap());
}

#[tokio::test]
async fn session_slot_wait_is_bounded_by_open_deadline() {
    let provider = AzureRelayTransportProvider::new(
        RelayTransportConfig {
            max_concurrent_sessions: 1,
            ..config()
        },
        endpoint(),
        Arc::new(FakeCredentials),
        Arc::new(FakeConnector {
            socket: Arc::new(FakeSocket::default()),
        }),
    )
    .unwrap();
    let held = provider.open(RelayRole::Sender, 1_000).await.unwrap();
    assert!(matches!(
        provider.open(RelayRole::Sender, 20).await,
        Err(RelayTransportError::DeadlineExpired)
    ));
    held.close().await.unwrap();
}

struct RoleAndExpiryCredentials {
    role: RelayCredentialRole,
    expiry: u64,
    revoked: Arc<Mutex<usize>>,
}

#[async_trait]
impl RelayCredentialPort for RoleAndExpiryCredentials {
    async fn acquire(
        &self,
        _: RelayCredentialRole,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Ok(RelayCredentialLease::new(
            RelayCredentialMaterial::SasToken(RelaySecret::new(b"token".to_vec()).unwrap()),
            self.role,
            self.expiry,
        ))
    }

    async fn revoke(&self, _: RelayCredentialLease) -> Result<(), RelayCredentialError> {
        *self.revoked.lock().unwrap() += 1;
        Ok(())
    }
}

#[tokio::test]
async fn invalid_lease_role_and_expiry_never_reach_connector() {
    let revoked = Arc::new(Mutex::new(0));
    let provider = AzureRelayTransportProvider::new(
        config(),
        endpoint(),
        Arc::new(RoleAndExpiryCredentials {
            role: RelayCredentialRole::Listen,
            expiry: valid_expiry(),
            revoked: Arc::clone(&revoked),
        }),
        Arc::new(FakeConnector {
            socket: Arc::new(FakeSocket::default()),
        }),
    )
    .unwrap();
    assert!(matches!(
        provider.open(RelayRole::Sender, 1_000).await,
        Err(RelayTransportError::CredentialRoleMismatch)
    ));
    assert_eq!(*revoked.lock().unwrap(), 1);

    let revoked = Arc::new(Mutex::new(0));
    let provider = AzureRelayTransportProvider::new(
        config(),
        endpoint(),
        Arc::new(RoleAndExpiryCredentials {
            role: RelayCredentialRole::Send,
            expiry: 1,
            revoked: Arc::clone(&revoked),
        }),
        Arc::new(FakeConnector {
            socket: Arc::new(FakeSocket::default()),
        }),
    )
    .unwrap();
    assert!(matches!(
        provider.open(RelayRole::Sender, 1_000).await,
        Err(RelayTransportError::CredentialExpired)
    ));
    assert_eq!(*revoked.lock().unwrap(), 1);
}

struct FailingSocket {
    closed: Arc<Mutex<bool>>,
}

#[async_trait]
impl RelaySocket for FailingSocket {
    async fn send(&self, _: RelayFrame) -> Result<(), RelayTransportError> {
        Err(RelayTransportError::Unavailable)
    }

    async fn receive(&self) -> Result<Option<RelayFrame>, RelayTransportError> {
        Err(RelayTransportError::Unavailable)
    }

    async fn close(&self) -> Result<(), RelayTransportError> {
        *self.closed.lock().unwrap() = true;
        Ok(())
    }
}

struct FailingConnector {
    socket: Arc<FailingSocket>,
}

#[async_trait]
impl RelaySocketConnector for FailingConnector {
    async fn connect(
        &self,
        _: &RelayEndpoint,
        _: RelayRole,
        _: &RelayCredentialLease,
    ) -> Result<Arc<dyn RelaySocket>, RelayTransportError> {
        Ok(Arc::clone(&self.socket) as Arc<dyn RelaySocket>)
    }
}

struct EofSocket {
    closed: Arc<Mutex<bool>>,
}

#[async_trait]
impl RelaySocket for EofSocket {
    async fn send(&self, _: RelayFrame) -> Result<(), RelayTransportError> {
        Ok(())
    }

    async fn receive(&self) -> Result<Option<RelayFrame>, RelayTransportError> {
        Ok(None)
    }

    async fn close(&self) -> Result<(), RelayTransportError> {
        *self.closed.lock().unwrap() = true;
        Ok(())
    }
}

struct EofConnector {
    socket: Arc<EofSocket>,
}

#[async_trait]
impl RelaySocketConnector for EofConnector {
    async fn connect(
        &self,
        _: &RelayEndpoint,
        _: RelayRole,
        _: &RelayCredentialLease,
    ) -> Result<Arc<dyn RelaySocket>, RelayTransportError> {
        Ok(Arc::clone(&self.socket) as Arc<dyn RelaySocket>)
    }
}

#[tokio::test]
async fn failed_send_closes_the_session() {
    let closed = Arc::new(Mutex::new(false));
    let socket = Arc::new(FailingSocket {
        closed: Arc::clone(&closed),
    });
    let provider = AzureRelayTransportProvider::new(
        config(),
        endpoint(),
        Arc::new(FakeCredentials),
        Arc::new(FailingConnector {
            socket: Arc::clone(&socket),
        }),
    )
    .unwrap();
    let connection = provider.open(RelayRole::Sender, 1_000).await.unwrap();
    let challenge = connection.enrollment_challenge();
    let proof = RelayEnrollmentProof::authenticate(
        &FakeEnrollment,
        b"authenticated-enrollment",
        &challenge,
    )
    .unwrap();
    connection.enroll(proof).await.unwrap();
    assert_eq!(
        connection
            .send(RelayFrame::new(b"x".to_vec()).unwrap())
            .await,
        Err(RelayTransportError::Unavailable)
    );
    assert_eq!(
        connection.phase().await,
        d2b_provider_transport_azure_relay::RelaySessionPhase::Closed
    );
    assert!(*closed.lock().unwrap());
    let connection = provider.open(RelayRole::Sender, 1_000).await.unwrap();
    connection.close().await.unwrap();
}

#[tokio::test]
async fn eof_closes_the_session_and_releases_the_slot() {
    let closed = Arc::new(Mutex::new(false));
    let socket = Arc::new(EofSocket {
        closed: Arc::clone(&closed),
    });
    let provider = AzureRelayTransportProvider::new(
        RelayTransportConfig {
            max_concurrent_sessions: 1,
            ..config()
        },
        endpoint(),
        Arc::new(FakeCredentials),
        Arc::new(EofConnector {
            socket: Arc::clone(&socket),
        }),
    )
    .unwrap();
    let connection = provider.open(RelayRole::Sender, 1_000).await.unwrap();
    let challenge = connection.enrollment_challenge();
    let proof = RelayEnrollmentProof::authenticate(
        &FakeEnrollment,
        b"authenticated-enrollment",
        &challenge,
    )
    .unwrap();
    connection.enroll(proof).await.unwrap();
    assert!(connection.receive().await.unwrap().is_none());
    assert_eq!(
        connection.phase().await,
        d2b_provider_transport_azure_relay::RelaySessionPhase::Closed
    );
    assert!(*closed.lock().unwrap());
    provider
        .open(RelayRole::Sender, 1_000)
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
}

#[test]
fn helper_surface_does_not_reintroduce_unbounded_window() {
    assert!(CreditWindow::new(256 * 1024).is_ok());
}
