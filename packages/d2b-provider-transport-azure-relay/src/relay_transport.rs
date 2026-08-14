//! Azure Relay byte-stream Provider.

use std::{fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use d2b_contracts::v3::ResourceRef;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tokio::time::sleep;
use zeroize::Zeroizing;

use crate::{
    backpressure::{BackpressureError, CreditWindow, MAX_RELAY_FRAME_BYTES},
    credential_client::{
        RelayCredentialLease, RelayCredentialPort, RelayCredentialRole, RelaySecret,
    },
    reconnect::{ReconnectBackoff, ReconnectDecision},
    transport_settings::RelayTransportSettings,
};

type HmacSha256 = Hmac<Sha256>;

/// Relay endpoint role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayRole {
    /// Gateway listener.
    Listener,
    /// Gateway sender.
    Sender,
}

/// Non-secret Relay endpoint settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayEndpoint {
    /// Validated transport settings.
    pub settings: RelayTransportSettings,
}

/// Provider root configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayTransportConfig {
    /// Gateway Guest execution boundary.
    pub execution_ref: ResourceRef,
    /// Gateway egress Network.
    pub network_ref: ResourceRef,
    /// Session cap.
    pub max_concurrent_sessions: u32,
    /// Connect timeout.
    pub connect_timeout_seconds: u32,
}

impl RelayTransportConfig {
    /// Validate placement and bounds.
    pub fn validate(&self) -> Result<(), RelayTransportError> {
        if self.execution_ref.resource_type().as_str() != "Guest"
            || self.network_ref.resource_type().as_str() != "Network"
            || !(1..=256).contains(&self.max_concurrent_sessions)
            || !(5..=300).contains(&self.connect_timeout_seconds)
        {
            return Err(RelayTransportError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl fmt::Debug for RelayTransportConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayTransportConfig")
            .field("execution_ref", &"<redacted>")
            .field("network_ref", &"<redacted>")
            .field("max_concurrent_sessions", &self.max_concurrent_sessions)
            .field("connect_timeout_seconds", &self.connect_timeout_seconds)
            .finish()
    }
}

/// Relay application frame.
pub struct RelayFrame(Zeroizing<Vec<u8>>);

impl RelayFrame {
    /// Construct a bounded frame.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, RelayTransportError> {
        let bytes = bytes.into();
        if bytes.len() > MAX_RELAY_FRAME_BYTES {
            return Err(RelayTransportError::FrameTooLarge);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// Borrow bytes at the socket effect boundary.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for RelayFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RelayFrame(<redacted>)")
    }
}

/// Relay socket seam. The production adapter can back this with a
/// WebSocket; tests use a real duplex byte stream.
#[async_trait]
pub trait RelaySocket: Send + Sync {
    /// Write one bounded frame.
    async fn send(&self, frame: RelayFrame) -> Result<(), RelayTransportError>;
    /// Read one bounded frame.
    async fn receive(&self) -> Result<Option<RelayFrame>, RelayTransportError>;
    /// Close the socket.
    async fn close(&self) -> Result<(), RelayTransportError>;
}

/// Connector seam that owns endpoint and WebSocket details.
#[async_trait]
pub trait RelaySocketConnector: Send + Sync {
    /// Connect one role using gateway-local credential material.
    async fn connect(
        &self,
        endpoint: &RelayEndpoint,
        role: RelayRole,
        lease: &RelayCredentialLease,
    ) -> Result<Arc<dyn RelaySocket>, RelayTransportError>;
}

/// Session phase, including the bootstrap-to-enrolled transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelaySessionPhase {
    /// One-time IKpsk2 bootstrap is not yet committed.
    Bootstrap,
    /// Core persisted enrollment before opening KK.
    EnrollmentCommitted,
    /// Enrolled KK session is active.
    EnrolledKk,
    /// Session closed.
    Closed,
}

/// Evidence produced by an authenticated enrollment handshake.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RelayEnrollmentProof {
    transcript_digest: [u8; 32],
}

impl fmt::Debug for RelayEnrollmentProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RelayEnrollmentProof(<redacted>)")
    }
}

/// Verifies the authenticated enrollment transcript.
pub trait RelayEnrollmentVerifier: Send + Sync {
    /// Verify the transcript and admit it for one KK session.
    fn verify_enrollment(&self, transcript: &[u8]) -> bool;
}

impl RelayEnrollmentProof {
    /// Verify an enrollment transcript and mint an unforgeable proof token.
    pub fn authenticate<V: RelayEnrollmentVerifier>(
        verifier: &V,
        transcript: &[u8],
    ) -> Result<Self, RelayTransportError> {
        if transcript.is_empty() || !verifier.verify_enrollment(transcript) {
            return Err(RelayTransportError::AuthenticationFailed);
        }
        Ok(Self {
            transcript_digest: Sha256::digest(transcript).into(),
        })
    }
}

impl RelaySessionPhase {
    /// Accept the one-time enrollment transition using authenticated proof.
    pub fn establish_enrolled_kk(
        self,
        proof: &RelayEnrollmentProof,
        offered_bootstrap_continuation: bool,
    ) -> Result<Self, RelayTransportError> {
        let _ = proof.transcript_digest;
        match self {
            Self::Bootstrap if offered_bootstrap_continuation => {
                Err(RelayTransportError::InvalidSessionTransition)
            }
            Self::Bootstrap => Ok(Self::EnrolledKk),
            Self::EnrollmentCommitted if offered_bootstrap_continuation => {
                Err(RelayTransportError::InvalidSessionTransition)
            }
            Self::EnrollmentCommitted => Ok(Self::EnrolledKk),
            Self::EnrolledKk => Ok(Self::EnrolledKk),
            Self::Closed => Err(RelayTransportError::InvalidSessionTransition),
        }
    }
}

/// A relay-authenticated peer carries no local d2b authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayAuthenticatedPeer;

impl RelayAuthenticatedPeer {
    /// Relay evidence never grants local Admin.
    pub const fn local_admin(self) -> bool {
        false
    }
}

/// One open relay connection with bounded named-stream credits.
pub struct RelayConnection {
    socket: Arc<dyn RelaySocket>,
    credits: Mutex<CreditWindow>,
    phase: Mutex<RelaySessionPhase>,
}

impl RelayConnection {
    /// Construct a connection that is not yet enrolled.
    pub fn from_socket(
        socket: Arc<dyn RelaySocket>,
        credit_bytes: usize,
    ) -> Result<Self, RelayTransportError> {
        Ok(Self {
            socket,
            credits: Mutex::new(
                CreditWindow::new(credit_bytes)
                    .map_err(|_| RelayTransportError::CreditExhausted)?,
            ),
            phase: Mutex::new(RelaySessionPhase::Bootstrap),
        })
    }

    /// Commit authenticated enrollment before any application frame is sent.
    pub async fn enroll(&self, proof: &RelayEnrollmentProof) -> Result<(), RelayTransportError> {
        let mut phase = self.phase.lock().await;
        *phase = phase.establish_enrolled_kk(proof, false)?;
        Ok(())
    }

    /// Send one frame only when credits are available.
    pub async fn send(&self, frame: RelayFrame) -> Result<(), RelayTransportError> {
        if self.phase().await != RelaySessionPhase::EnrolledKk {
            return Err(RelayTransportError::InvalidSessionTransition);
        }
        let size = frame.as_bytes().len();
        {
            let mut credits = self.credits.lock().await;
            credits.reserve(size).map_err(|error| match error {
                BackpressureError::FrameTooLarge => RelayTransportError::FrameTooLarge,
                BackpressureError::CreditExhausted => RelayTransportError::CreditExhausted,
            })?;
        }
        self.socket.send(frame).await
    }

    /// Receive one frame.
    pub async fn receive(&self) -> Result<Option<RelayFrame>, RelayTransportError> {
        self.socket.receive().await
    }

    /// Grant credits from the remote named stream.
    pub async fn grant(&self, bytes: usize) {
        self.credits.lock().await.grant(bytes);
    }

    /// Release send credits after a remote acknowledgement.
    pub async fn acknowledge(&self, bytes: usize) {
        self.credits.lock().await.acknowledge(bytes);
    }

    /// Return available and in-flight send credits.
    pub async fn credit_state(&self) -> (usize, usize) {
        let credits = self.credits.lock().await;
        (credits.available(), credits.in_flight())
    }

    /// Close the exact connection.
    pub async fn close(&self) -> Result<(), RelayTransportError> {
        *self.phase.lock().await = RelaySessionPhase::Closed;
        self.socket.close().await
    }

    /// Return current session phase.
    pub async fn phase(&self) -> RelaySessionPhase {
        *self.phase.lock().await
    }
}

/// Stable transport errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayTransportError {
    /// Provider config or settings were invalid.
    InvalidConfiguration,
    /// Credentials were unavailable.
    CredentialUnavailable,
    /// Authentication failed.
    AuthenticationFailed,
    /// Endpoint was not ready.
    Unavailable,
    /// A frame exceeded the fixed bound.
    FrameTooLarge,
    /// Credits were exhausted.
    CreditExhausted,
    /// The wire protocol was malformed.
    Protocol,
    /// The session transition was invalid.
    InvalidSessionTransition,
    /// The operation deadline elapsed.
    DeadlineExpired,
}

impl fmt::Display for RelayTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfiguration => "relay-invalid-configuration",
            Self::CredentialUnavailable => "relay-credential-unavailable",
            Self::AuthenticationFailed => "relay-authentication-failed",
            Self::Unavailable => "relay-unavailable",
            Self::FrameTooLarge => "relay-frame-too-large",
            Self::CreditExhausted => "relay-credit-exhausted",
            Self::Protocol => "relay-protocol",
            Self::InvalidSessionTransition => "relay-invalid-session-transition",
            Self::DeadlineExpired => "relay-deadline-expired",
        })
    }
}

impl std::error::Error for RelayTransportError {}

/// Canonical Azure Relay Provider.
pub struct AzureRelayTransportProvider<C, K> {
    config: RelayTransportConfig,
    endpoint: RelayEndpoint,
    credentials: Arc<C>,
    connector: Arc<K>,
}

impl<C, K> AzureRelayTransportProvider<C, K>
where
    C: RelayCredentialPort + 'static,
    K: RelaySocketConnector + 'static,
{
    /// Construct a Provider with gateway-local effect ports.
    pub fn new(
        config: RelayTransportConfig,
        endpoint: RelayEndpoint,
        credentials: Arc<C>,
        connector: Arc<K>,
    ) -> Result<Self, RelayTransportError> {
        config.validate()?;
        endpoint
            .settings
            .validate()
            .map_err(|_| RelayTransportError::InvalidConfiguration)?;
        Ok(Self {
            config,
            endpoint,
            credentials,
            connector,
        })
    }

    /// Open a named stream using one short-lived role credential.
    pub async fn open(
        &self,
        role: RelayRole,
        deadline_ms: u32,
    ) -> Result<RelayConnection, RelayTransportError> {
        self.open_with_backoff(
            role,
            deadline_ms,
            ReconnectBackoff::new(4_000, deadline_ms.min(30_000)),
        )
        .await
    }

    /// Open a connection with a finite reconnect policy.
    pub async fn open_with_backoff(
        &self,
        role: RelayRole,
        deadline_ms: u32,
        mut backoff: ReconnectBackoff,
    ) -> Result<RelayConnection, RelayTransportError> {
        if deadline_ms == 0 {
            return Err(RelayTransportError::DeadlineExpired);
        }
        let credential_role = match role {
            RelayRole::Listener => RelayCredentialRole::Listen,
            RelayRole::Sender => RelayCredentialRole::Send,
        };
        let mut elapsed_ms = 0_u32;
        loop {
            let remaining_ms = deadline_ms.saturating_sub(elapsed_ms);
            if remaining_ms == 0 {
                return Err(RelayTransportError::DeadlineExpired);
            }
            let lease = self
                .credentials
                .acquire(credential_role, remaining_ms)
                .await
                .map_err(|_| RelayTransportError::CredentialUnavailable)?;
            let socket_result = self.connector.connect(&self.endpoint, role, &lease).await;
            let revoke_result = self.credentials.revoke(lease).await;
            let socket = match socket_result {
                Ok(socket) => socket,
                Err(error) => {
                    revoke_result.map_err(|_| RelayTransportError::CredentialUnavailable)?;
                    if !matches!(error, RelayTransportError::Unavailable) {
                        return Err(error);
                    }
                    match backoff.failed() {
                        ReconnectDecision::RetryAfter(delay)
                            if delay <= deadline_ms.saturating_sub(elapsed_ms) =>
                        {
                            sleep(Duration::from_millis(u64::from(delay))).await;
                            elapsed_ms = elapsed_ms.saturating_add(delay);
                            continue;
                        }
                        ReconnectDecision::RetryAfter(_) | ReconnectDecision::Closed => {
                            return Err(RelayTransportError::Unavailable);
                        }
                        ReconnectDecision::OpenNow => continue,
                    }
                }
            };
            if revoke_result.is_err() {
                let _ = socket.close().await;
                return Err(RelayTransportError::CredentialUnavailable);
            }
            return RelayConnection::from_socket(socket, 256 * 1024);
        }
    }

    /// Return the gateway execution boundary.
    pub const fn config(&self) -> &RelayTransportConfig {
        &self.config
    }
}

/// Mint a short-lived SAS token inside the gateway Guest.
pub fn mint_sas(
    resource_uri: &str,
    key_name: &str,
    key: &RelaySecret,
    ttl_secs: u64,
    now_unix_secs: u64,
) -> Result<RelaySecret, RelayTransportError> {
    if ttl_secs == 0 || ttl_secs > 15 * 60 || resource_uri.is_empty() || key_name.is_empty() {
        return Err(RelayTransportError::InvalidConfiguration);
    }
    let expiry = now_unix_secs.saturating_add(ttl_secs);
    let encoded_uri = urlencoding::encode(resource_uri);
    let string_to_sign = format!("{encoded_uri}\n{expiry}");
    let mut mac = HmacSha256::new_from_slice(key.as_bytes())
        .map_err(|_| RelayTransportError::AuthenticationFailed)?;
    mac.update(string_to_sign.as_bytes());
    let signature = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        mac.finalize().into_bytes(),
    );
    RelaySecret::new(format!(
        "SharedAccessSignature sr={encoded_uri}&sig={}&se={expiry}&skn={key_name}",
        urlencoding::encode(&signature)
    ))
    .map_err(|_| RelayTransportError::AuthenticationFailed)
}
