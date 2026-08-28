use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::ResourceRef;
use d2b_session::{OwnedTransport, TransportPacket};
use d2b_provider_transport_azure_relay::{
    AzureRelayTransportProvider, CreditWindow, MAX_RELAY_GENERATION_FENCES, ReconnectBackoff,
    RelayAuthenticatedPeer, RelayCredentialBinding, RelayCredentialError, RelayCredentialLease,
    RelayCredentialMaterial, RelayCredentialPort, RelayCredentialRole, RelayEndpoint,
    RelayComponentSessionTransport, RelayEnrollmentProof, RelayEnrollmentVerifier, RelayFrame,
    RelayRole, RelaySecret, RelaySocket, RelaySocketConnector, RelayTransportConfig,
    RelayTransportError, RelayTransportSettings,
};
use tokio::sync::Notify;

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

    async fn acquire_bound(
        &self,
        role: RelayCredentialRole,
        binding: &RelayCredentialBinding,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Ok(RelayCredentialLease::new_bound(
            RelayCredentialMaterial::SasToken(RelaySecret::new(b"token".to_vec()).unwrap()),
            role,
            valid_expiry(),
            binding.clone(),
        )
        .unwrap())
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

struct FailGenerationConnector {
    fail_generation: u64,
    socket: Arc<FakeSocket>,
}

#[async_trait]
impl RelaySocketConnector for FailGenerationConnector {
    async fn connect(
        &self,
        _: &RelayEndpoint,
        _: RelayRole,
        lease: &RelayCredentialLease,
    ) -> Result<Arc<dyn RelaySocket>, RelayTransportError> {
        if lease.reconnect_generation() == self.fail_generation {
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

    async fn acquire_bound(
        &self,
        role: RelayCredentialRole,
        binding: &RelayCredentialBinding,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Ok(RelayCredentialLease::new_bound(
            RelayCredentialMaterial::SasToken(RelaySecret::new(b"token".to_vec()).unwrap()),
            role,
            valid_expiry(),
            binding.clone(),
        )
        .unwrap())
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
async fn enrolled_relay_connection_is_a_component_session_transport() {
    let provider = provider();
    let connection = provider.open(RelayRole::Sender, 1_000).await.unwrap();
    let challenge = connection.enrollment_challenge();
    let proof = RelayEnrollmentProof::authenticate(
        &FakeEnrollment,
        b"authenticated-enrollment",
        &challenge,
    )
    .unwrap();
    connection.enroll(proof).await.unwrap();

    let mut transport = RelayComponentSessionTransport::from_connection(connection);
    let descriptor = transport.descriptor();
    assert_eq!(
        descriptor.class,
        d2b_contracts_zone_session::v3::component_session::TransportClass::ProviderStream
    );
    assert_eq!(
        descriptor.locality,
        d2b_contracts_zone_session::v3::component_session::Locality::Remote
    );
    assert!(!descriptor.supports_attachments);

    transport
        .send(TransportPacket::new(b"encrypted-session-record".to_vec()))
        .await
        .unwrap();
    let packet = transport.receive(1024).await.unwrap();
    assert_eq!(packet.as_bytes(), b"encrypted-session-record");
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

struct TrackingCredentials {
    acquired: Arc<Mutex<usize>>,
    revoked: Arc<Mutex<usize>>,
    binding: Arc<Mutex<Option<RelayCredentialBinding>>>,
}

#[async_trait]
impl RelayCredentialPort for TrackingCredentials {
    async fn acquire(
        &self,
        _: RelayCredentialRole,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        panic!("the transport must use the binding-aware acquisition path");
    }

    async fn acquire_bound(
        &self,
        role: RelayCredentialRole,
        binding: &RelayCredentialBinding,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        *self.acquired.lock().unwrap() += 1;
        *self.binding.lock().unwrap() = Some(binding.clone());
        Ok(RelayCredentialLease::new_bound(
            RelayCredentialMaterial::SasToken(
                RelaySecret::new(b"connection-token".to_vec()).unwrap(),
            ),
            role,
            valid_expiry(),
            binding.clone(),
        )
        .unwrap())
    }

    async fn revoke(&self, _: RelayCredentialLease) -> Result<(), RelayCredentialError> {
        *self.revoked.lock().unwrap() += 1;
        Ok(())
    }
}

#[tokio::test]
async fn bound_open_acquires_and_revokes_one_lease_per_connection() {
    let acquired = Arc::new(Mutex::new(0));
    let revoked = Arc::new(Mutex::new(0));
    let seen_binding = Arc::new(Mutex::new(None));
    let provider = AzureRelayTransportProvider::new(
        config(),
        endpoint(),
        Arc::new(TrackingCredentials {
            acquired: Arc::clone(&acquired),
            revoked: Arc::clone(&revoked),
            binding: Arc::clone(&seen_binding),
        }),
        Arc::new(FakeConnector {
            socket: Arc::new(FakeSocket::default()),
        }),
    )
    .unwrap();
    let binding = RelayCredentialBinding::new("link-1", "session-1", 3).unwrap();
    let connection = provider
        .open_bound(RelayRole::Sender, binding.clone(), 1_000)
        .await
        .unwrap();
    assert_eq!(*acquired.lock().unwrap(), 1);
    assert_eq!(*revoked.lock().unwrap(), 1);
    assert_eq!(*seen_binding.lock().unwrap(), Some(binding.clone()));
    assert_eq!(connection.binding(), &binding);
    connection.close().await.unwrap();
}

#[tokio::test]
async fn failed_new_generation_leaves_the_live_current_connection_usable() {
    let socket = Arc::new(FakeSocket::default());
    let provider = AzureRelayTransportProvider::new(
        config(),
        endpoint(),
        Arc::new(FakeCredentials),
        Arc::new(FailGenerationConnector {
            fail_generation: 2,
            socket: Arc::clone(&socket),
        }),
    )
    .unwrap();
    let first_binding = RelayCredentialBinding::new("link-1", "session-1", 1).unwrap();
    let first = provider
        .open_bound(RelayRole::Sender, first_binding, 1_000)
        .await
        .unwrap();
    let challenge = first.enrollment_challenge();
    let proof = RelayEnrollmentProof::authenticate(
        &FakeEnrollment,
        b"authenticated-enrollment",
        &challenge,
    )
    .unwrap();
    first.enroll(proof).await.unwrap();

    let second_binding = RelayCredentialBinding::new("link-1", "session-1", 2).unwrap();
    assert!(matches!(
        provider
            .open_bound(RelayRole::Sender, second_binding, 50)
            .await,
        Err(RelayTransportError::Unavailable)
    ));
    assert_eq!(
        first
            .send(RelayFrame::new(b"still-current".to_vec()).unwrap())
            .await,
        Ok(())
    );
    first.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_connect_releases_the_active_lease_row() {
    let active = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(
        AzureRelayTransportProvider::new(
            config(),
            endpoint(),
            Arc::new(CleanupCredentials {
                active: Arc::clone(&active),
                pending_revoke: true,
            }),
            Arc::new(PendingConnector),
        )
        .unwrap(),
    );
    let binding = RelayCredentialBinding::new("link-1", "cancelled", 1).unwrap();
    let task_provider = Arc::clone(&provider);
    let task = tokio::spawn(async move {
        task_provider
            .open_bound(RelayRole::Sender, binding, 10_000)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while active.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let join = task.await;
    assert!(matches!(join, Err(error) if error.is_cancelled()));
    wait_for_zero(&active).await;
}

#[tokio::test]
async fn timed_out_connect_releases_the_active_lease_row() {
    let active = Arc::new(AtomicUsize::new(0));
    let provider = AzureRelayTransportProvider::new(
        config(),
        endpoint(),
        Arc::new(CleanupCredentials {
            active: Arc::clone(&active),
            pending_revoke: true,
        }),
        Arc::new(PendingConnector),
    )
    .unwrap();
    let binding = RelayCredentialBinding::new("link-1", "timed-out", 1).unwrap();
    assert!(matches!(
        provider.open_bound(RelayRole::Sender, binding, 20).await,
        Err(RelayTransportError::DeadlineExpired)
    ));
    wait_for_zero(&active).await;
}

#[tokio::test]
async fn connector_error_releases_the_active_lease_row() {
    let active = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(Mutex::new(0));
    let provider = AzureRelayTransportProvider::new(
        config(),
        endpoint(),
        Arc::new(CleanupCredentials {
            active: Arc::clone(&active),
            pending_revoke: false,
        }),
        Arc::new(CountingConnector {
            calls: Arc::clone(&calls),
        }),
    )
    .unwrap();
    let binding = RelayCredentialBinding::new("link-1", "connector-error", 1).unwrap();
    assert!(matches!(
        provider.open_bound(RelayRole::Sender, binding, 1_000).await,
        Err(RelayTransportError::Unavailable)
    ));
    assert_eq!(*calls.lock().unwrap(), 1);
    wait_for_zero(&active).await;
}

#[tokio::test]
async fn timed_out_revoke_releases_the_active_lease_row() {
    let active = Arc::new(AtomicUsize::new(0));
    let provider = AzureRelayTransportProvider::new(
        config(),
        endpoint(),
        Arc::new(CleanupCredentials {
            active: Arc::clone(&active),
            pending_revoke: true,
        }),
        Arc::new(FakeConnector {
            socket: Arc::new(FakeSocket::default()),
        }),
    )
    .unwrap();
    let binding = RelayCredentialBinding::new("link-1", "revoke-timeout", 1).unwrap();
    assert!(matches!(
        provider.open_bound(RelayRole::Sender, binding, 20).await,
        Err(RelayTransportError::DeadlineExpired)
    ));
    wait_for_zero(&active).await;
}

#[tokio::test]
async fn convenience_opens_release_generation_capacity() {
    let provider = provider();
    for _ in 0..=MAX_RELAY_GENERATION_FENCES {
        provider
            .open(RelayRole::Sender, 1_000)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
    }
    provider
        .open_bound(
            RelayRole::Sender,
            RelayCredentialBinding::new("link-final", "session-final", 1).unwrap(),
            1_000,
        )
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
}

struct WrongBindingCredentials {
    wrong_binding: RelayCredentialBinding,
    revoked: Arc<Mutex<usize>>,
}

#[async_trait]
impl RelayCredentialPort for WrongBindingCredentials {
    async fn acquire(
        &self,
        role: RelayCredentialRole,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Ok(RelayCredentialLease::new(
            RelayCredentialMaterial::SasToken(
                RelaySecret::new(b"wrong-binding-token".to_vec()).unwrap(),
            ),
            role,
            valid_expiry(),
        ))
    }

    async fn acquire_bound(
        &self,
        role: RelayCredentialRole,
        _: &RelayCredentialBinding,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Ok(RelayCredentialLease::new_bound(
            RelayCredentialMaterial::SasToken(
                RelaySecret::new(b"wrong-binding-token".to_vec()).unwrap(),
            ),
            role,
            valid_expiry(),
            self.wrong_binding.clone(),
        )
        .unwrap())
    }

    async fn revoke(&self, _: RelayCredentialLease) -> Result<(), RelayCredentialError> {
        *self.revoked.lock().unwrap() += 1;
        Ok(())
    }
}

struct CountingConnector {
    calls: Arc<Mutex<usize>>,
}

#[async_trait]
impl RelaySocketConnector for CountingConnector {
    async fn connect(
        &self,
        _: &RelayEndpoint,
        _: RelayRole,
        _: &RelayCredentialLease,
    ) -> Result<Arc<dyn RelaySocket>, RelayTransportError> {
        *self.calls.lock().unwrap() += 1;
        Err(RelayTransportError::Unavailable)
    }
}

#[tokio::test]
async fn mismatched_bound_lease_is_revoked_before_connector_dispatch() {
    let revoked = Arc::new(Mutex::new(0));
    let calls = Arc::new(Mutex::new(0));
    let provider = AzureRelayTransportProvider::new(
        config(),
        endpoint(),
        Arc::new(WrongBindingCredentials {
            wrong_binding: RelayCredentialBinding::new("link-2", "session-2", 1).unwrap(),
            revoked: Arc::clone(&revoked),
        }),
        Arc::new(CountingConnector {
            calls: Arc::clone(&calls),
        }),
    )
    .unwrap();
    let binding = RelayCredentialBinding::new("link-1", "session-1", 1).unwrap();
    assert!(matches!(
        provider.open_bound(RelayRole::Sender, binding, 1_000).await,
        Err(RelayTransportError::CredentialBindingMismatch)
    ));
    assert_eq!(*revoked.lock().unwrap(), 1);
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[tokio::test]
async fn stale_reconnect_generation_closes_the_old_connection_before_io() {
    let provider = provider();
    let first_binding = RelayCredentialBinding::new("link-1", "session-1", 1).unwrap();
    let first = provider
        .open_bound(RelayRole::Sender, first_binding, 1_000)
        .await
        .unwrap();
    let challenge = first.enrollment_challenge();
    let proof = RelayEnrollmentProof::authenticate(
        &FakeEnrollment,
        b"authenticated-enrollment",
        &challenge,
    )
    .unwrap();
    first.enroll(proof).await.unwrap();

    let second_binding = RelayCredentialBinding::new("link-1", "session-1", 2).unwrap();
    let second = provider
        .open_bound(RelayRole::Sender, second_binding, 1_000)
        .await
        .unwrap();
    assert_eq!(
        first
            .send(RelayFrame::new(b"stale".to_vec()).unwrap())
            .await,
        Err(RelayTransportError::StaleGeneration)
    );
    assert_eq!(
        first.phase().await,
        d2b_provider_transport_azure_relay::RelaySessionPhase::Closed
    );
    second.close().await.unwrap();
}

#[tokio::test]
async fn newer_success_remains_authoritative_after_older_cleanup() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let provider = Arc::new(
        AzureRelayTransportProvider::new(
            config(),
            endpoint(),
            Arc::new(FakeCredentials),
            Arc::new(BlockingGenerationConnector {
                started: Arc::clone(&started),
                release: Arc::clone(&release),
                socket: Arc::new(FakeSocket::default()),
            }),
        )
        .unwrap(),
    );
    let first_binding = RelayCredentialBinding::new("link-1", "session-1", 1).unwrap();
    let old_provider = Arc::clone(&provider);
    let old_open = tokio::spawn(async move {
        old_provider
            .open_bound(RelayRole::Sender, first_binding, 10_000)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), started.notified())
        .await
        .unwrap();

    let second_binding = RelayCredentialBinding::new("link-1", "session-1", 2).unwrap();
    let second = provider
        .open_bound(RelayRole::Sender, second_binding, 1_000)
        .await
        .unwrap();
    release.notify_one();
    assert!(matches!(
        old_open.await.unwrap(),
        Err(RelayTransportError::StaleGeneration)
    ));

    let challenge = second.enrollment_challenge();
    let proof = RelayEnrollmentProof::authenticate(
        &FakeEnrollment,
        b"authenticated-enrollment",
        &challenge,
    )
    .unwrap();
    second.enroll(proof).await.unwrap();
    second
        .send(RelayFrame::new(b"new-current".to_vec()).unwrap())
        .await
        .unwrap();
    second.close().await.unwrap();
}

struct UnavailableCredentials {
    attempts: Arc<Mutex<usize>>,
}

#[async_trait]
impl RelayCredentialPort for UnavailableCredentials {
    async fn acquire(
        &self,
        _: RelayCredentialRole,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        *self.attempts.lock().unwrap() += 1;
        Err(RelayCredentialError::Unavailable)
    }

    async fn acquire_bound(
        &self,
        _: RelayCredentialRole,
        _: &RelayCredentialBinding,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        *self.attempts.lock().unwrap() += 1;
        Err(RelayCredentialError::Unavailable)
    }

    async fn revoke(&self, _: RelayCredentialLease) -> Result<(), RelayCredentialError> {
        Ok(())
    }
}

#[tokio::test]
async fn unavailable_credential_provider_uses_bounded_retries() {
    let attempts = Arc::new(Mutex::new(0));
    let provider = AzureRelayTransportProvider::new(
        config(),
        endpoint(),
        Arc::new(UnavailableCredentials {
            attempts: Arc::clone(&attempts),
        }),
        Arc::new(FakeConnector {
            socket: Arc::new(FakeSocket::default()),
        }),
    )
    .unwrap();
    let binding = RelayCredentialBinding::new("link-1", "session-1", 1).unwrap();
    assert!(matches!(
        provider
            .open_with_backoff_bound(
                RelayRole::Sender,
                binding,
                100,
                ReconnectBackoff::with_limits(1, 0, 2, 50),
            )
            .await,
        Err(RelayTransportError::CredentialUnavailable)
    ));
    assert!(*attempts.lock().unwrap() <= 3);
}

struct CleanupCredentials {
    active: Arc<AtomicUsize>,
    pending_revoke: bool,
}

#[async_trait]
impl RelayCredentialPort for CleanupCredentials {
    async fn acquire(
        &self,
        _: RelayCredentialRole,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Err(RelayCredentialError::BindingRequired)
    }

    async fn acquire_bound(
        &self,
        role: RelayCredentialRole,
        binding: &RelayCredentialBinding,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        self.active.fetch_add(1, Ordering::SeqCst);
        let active = Arc::clone(&self.active);
        let mut lease = RelayCredentialLease::new_bound(
            RelayCredentialMaterial::SasToken(RelaySecret::new(b"cleanup-token".to_vec()).unwrap()),
            role,
            valid_expiry(),
            binding.clone(),
        )
        .unwrap();
        lease.set_drop_hook(Arc::new(move |_| {
            active.fetch_sub(1, Ordering::SeqCst);
        }));
        Ok(lease)
    }

    async fn revoke(&self, _: RelayCredentialLease) -> Result<(), RelayCredentialError> {
        if self.pending_revoke {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}

struct PendingConnector;

#[async_trait]
impl RelaySocketConnector for PendingConnector {
    async fn connect(
        &self,
        _: &RelayEndpoint,
        _: RelayRole,
        _: &RelayCredentialLease,
    ) -> Result<Arc<dyn RelaySocket>, RelayTransportError> {
        std::future::pending::<Result<Arc<dyn RelaySocket>, RelayTransportError>>().await
    }
}

struct BlockingGenerationConnector {
    started: Arc<Notify>,
    release: Arc<Notify>,
    socket: Arc<FakeSocket>,
}

#[async_trait]
impl RelaySocketConnector for BlockingGenerationConnector {
    async fn connect(
        &self,
        _: &RelayEndpoint,
        _: RelayRole,
        lease: &RelayCredentialLease,
    ) -> Result<Arc<dyn RelaySocket>, RelayTransportError> {
        if lease.reconnect_generation() == 1 {
            self.started.notify_one();
            self.release.notified().await;
        }
        Ok(Arc::clone(&self.socket) as Arc<dyn RelaySocket>)
    }
}

async fn wait_for_zero(active: &AtomicUsize) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if active.load(Ordering::SeqCst) == 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
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

    async fn acquire_bound(
        &self,
        _: RelayCredentialRole,
        binding: &RelayCredentialBinding,
        _: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError> {
        Ok(RelayCredentialLease::new_bound(
            RelayCredentialMaterial::SasToken(RelaySecret::new(b"token".to_vec()).unwrap()),
            self.role,
            self.expiry,
            binding.clone(),
        )
        .unwrap())
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
