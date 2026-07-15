use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{
        Arc, Mutex as StdMutex, Once,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::Engine;
use d2b_contracts::{
    v2_identity::ProviderId,
    v2_provider::{
        Fingerprint, Generation, HandleId, LeaseId, MAX_PROVIDER_REGISTRY_ENTRIES, OperationId,
        TransportBindingId,
    },
};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use hmac::{Hmac, Mac};
use rustls_pki_types::{CertificateDer, pem::PemObject};
use sha2::Sha256;
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
};
use tokio_tungstenite::tungstenite::{
    Message, client::IntoClientRequest, http::HeaderValue, protocol::WebSocketConfig,
};
use zeroize::{Zeroize, Zeroizing};

use crate::{
    RelayAdoptRequest, RelayCloseOutcome, RelayCloseRequest, RelayControlPort,
    RelayExpectedResource, RelayInspectRequest, RelayInspection, RelayOpenRequest,
    RelayPortCapabilities, RelayPortFailure, RelayRendezvousId, RelayResource, RelayResourceState,
    RelayTransportLimits,
};

const MAX_RELAY_SECRET_BYTES: usize = 16 * 1024;
const MAX_RELAY_NAMESPACE_BYTES: usize = 253;
const MAX_RELAY_ENTITY_BYTES: usize = 256;
const MAX_WEBSOCKET_CONTROL_BYTES: usize = 125;

type HmacSha256 = Hmac<Sha256>;
type WebSocketStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WebSocketSink = SplitSink<WebSocketStream, Message>;
type WebSocketSource = SplitStream<WebSocketStream>;
type SharedRelaySocket = Arc<dyn RelaySocket>;
type AcceptedRelaySockets = Arc<Mutex<mpsc::Receiver<SharedRelaySocket>>>;

/// A bounded in-process secret that zeroizes its owned bytes on drop.
pub struct RelaySecret(Vec<u8>);

impl RelaySecret {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, RelayProductionBuildError> {
        let mut bytes = bytes.into();
        if bytes.is_empty() {
            bytes.zeroize();
            return Err(RelayProductionBuildError::EmptySecret);
        }
        if bytes.len() > MAX_RELAY_SECRET_BYTES {
            bytes.zeroize();
            return Err(RelayProductionBuildError::SecretTooLarge);
        }
        Ok(Self(bytes))
    }

    fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn utf8(&self) -> Result<&str, RelaySocketFailure> {
        std::str::from_utf8(self.as_bytes()).map_err(|_| RelaySocketFailure::AuthenticationFailed)
    }
}

impl Clone for RelaySecret {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Drop for RelaySecret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for RelaySecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RelaySecret(<redacted>)")
    }
}

/// Exact use assigned to an opaque credential lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayCredentialUse {
    Connect,
    Listen,
}

/// Secret credential material returned only inside the owning provider agent.
pub enum RelayCredentialMaterial {
    SasRule {
        key_name: RelaySecret,
        key: RelaySecret,
    },
    SasToken(RelaySecret),
    EntraBearer(RelaySecret),
}

impl fmt::Debug for RelayCredentialMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SasRule { .. } => "RelayCredentialMaterial::SasRule(<redacted>)",
            Self::SasToken(_) => "RelayCredentialMaterial::SasToken(<redacted>)",
            Self::EntraBearer(_) => "RelayCredentialMaterial::EntraBearer(<redacted>)",
        })
    }
}

/// A resolved short-lived credential and its absolute expiry.
pub struct RelayCredentialLease {
    material: RelayCredentialMaterial,
    expires_at_unix_ms: u64,
}

impl RelayCredentialLease {
    pub fn new(material: RelayCredentialMaterial, expires_at_unix_ms: u64) -> Self {
        Self {
            material,
            expires_at_unix_ms,
        }
    }
}

impl fmt::Debug for RelayCredentialLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayCredentialLease")
            .field("material", &self.material)
            .field("expires_at_unix_ms", &"<redacted>")
            .finish()
    }
}

/// Closed failures from a co-located credential module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayCredentialSourceFailure {
    LeaseUnknown,
    LeaseExpired,
    RoleMismatch,
    Unavailable,
}

impl fmt::Display for RelayCredentialSourceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LeaseUnknown => "relay credential lease is unknown",
            Self::LeaseExpired => "relay credential lease is expired",
            Self::RoleMismatch => "relay credential lease has the wrong role",
            Self::Unavailable => "relay credential source is unavailable",
        })
    }
}

impl Error for RelayCredentialSourceFailure {}

/// Co-located credential source. There is deliberately no environment fallback.
#[async_trait]
pub trait RelayCredentialSource: Send + Sync {
    async fn resolve(
        &self,
        lease_id: &LeaseId,
        credential_use: RelayCredentialUse,
    ) -> Result<RelayCredentialLease, RelayCredentialSourceFailure>;
}

/// One private mapping from opaque provider IDs to an Azure Relay endpoint.
#[derive(Clone)]
pub struct AzureRelayBinding {
    provider_id: ProviderId,
    transport_binding_id: TransportBindingId,
    rendezvous_id: RelayRendezvousId,
    namespace: String,
    entity: String,
    additional_ca_pem: Option<RelaySecret>,
}

impl AzureRelayBinding {
    pub fn new(
        provider_id: ProviderId,
        transport_binding_id: TransportBindingId,
        rendezvous_id: RelayRendezvousId,
        namespace: impl Into<String>,
        entity: impl Into<String>,
        additional_ca_pem: Option<RelaySecret>,
    ) -> Result<Self, RelayProductionBuildError> {
        let namespace = namespace.into();
        let entity = entity.into();
        if !valid_namespace(&namespace) || !valid_entity(&entity) {
            return Err(RelayProductionBuildError::InvalidEndpoint);
        }
        Ok(Self {
            provider_id,
            transport_binding_id,
            rendezvous_id,
            namespace,
            entity,
            additional_ca_pem,
        })
    }
}

impl fmt::Debug for AzureRelayBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureRelayBinding")
            .field("provider_id", &"<redacted>")
            .field("transport_binding_id", &"<redacted>")
            .field("rendezvous_id", &"<redacted>")
            .field("endpoint", &"<redacted>")
            .field(
                "additional_ca_pem",
                &self.additional_ca_pem.as_ref().map(|_| "<configured>"),
            )
            .finish()
    }
}

/// Closed construction failures with no endpoint or credential context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayProductionBuildError {
    EmptyBindings,
    TooManyBindings,
    DuplicateBinding,
    InvalidEndpoint,
    EmptySecret,
    SecretTooLarge,
}

impl fmt::Display for RelayProductionBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyBindings => "production Relay port has no configured bindings",
            Self::TooManyBindings => "production Relay port exceeds its binding bound",
            Self::DuplicateBinding => "production Relay port has a duplicate binding",
            Self::InvalidEndpoint => "production Relay endpoint is invalid",
            Self::EmptySecret => "production Relay secret is empty",
            Self::SecretTooLarge => "production Relay secret exceeds its bound",
        })
    }
}

impl Error for RelayProductionBuildError {}

/// WebSocket role used by the testable socket seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelaySocketRole {
    Sender,
    Listener,
}

/// Closed WebSocket failures; provider SDK diagnostics never cross this seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelaySocketFailure {
    AuthenticationFailed,
    ListenerNotReady,
    InvalidEndpoint,
    FrameTooLarge,
    Protocol,
    Unavailable,
}

impl fmt::Display for RelaySocketFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AuthenticationFailed => "relay WebSocket authentication failed",
            Self::ListenerNotReady => "relay listener is not ready",
            Self::InvalidEndpoint => "relay WebSocket endpoint is invalid",
            Self::FrameTooLarge => "relay WebSocket frame exceeds its bound",
            Self::Protocol => "relay WebSocket protocol failed",
            Self::Unavailable => "relay WebSocket is unavailable",
        })
    }
}

impl Error for RelaySocketFailure {}

/// Redacted, zeroizing application bytes from one bounded Relay frame.
pub struct RelayFrame(Vec<u8>);

impl RelayFrame {
    fn new(bytes: impl Into<Vec<u8>>, max_frame_bytes: usize) -> Result<Self, RelaySocketFailure> {
        let mut bytes = bytes.into();
        if bytes.len() > max_frame_bytes {
            bytes.zeroize();
            return Err(RelaySocketFailure::FrameTooLarge);
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for RelayFrame {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for RelayFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RelayFrame(<redacted>)")
    }
}

/// Bounded events emitted by a managed Relay WebSocket.
pub enum RelaySocketEvent {
    Binary(RelayFrame),
    Text(RelayFrame),
    Ping(RelayFrame),
    Closed,
}

impl fmt::Debug for RelaySocketEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Binary(_) => "RelaySocketEvent::Binary(<redacted>)",
            Self::Text(_) => "RelaySocketEvent::Text(<redacted>)",
            Self::Ping(_) => "RelaySocketEvent::Ping(<redacted>)",
            Self::Closed => "RelaySocketEvent::Closed",
        })
    }
}

/// One bounded, live Relay WebSocket.
#[async_trait]
pub trait RelaySocket: Send + Sync {
    fn is_open(&self) -> bool;
    async fn receive(&self) -> Result<RelaySocketEvent, RelaySocketFailure>;
    async fn send_binary(&self, bytes: &[u8]) -> Result<(), RelaySocketFailure>;
    async fn send_pong(&self, bytes: &[u8]) -> Result<(), RelaySocketFailure>;
    async fn close(&self) -> Result<(), RelaySocketFailure>;
}

#[derive(Clone)]
enum RelaySocketTarget {
    Endpoint {
        binding: AzureRelayBinding,
        credential: Arc<RelayCredentialLease>,
    },
}

/// Secret-redacting request delivered to the socket seam.
#[derive(Clone)]
pub struct RelaySocketConnectRequest {
    role: RelaySocketRole,
    target: RelaySocketTarget,
    limits: RelayTransportLimits,
}

impl RelaySocketConnectRequest {
    pub const fn role(&self) -> RelaySocketRole {
        self.role
    }

    pub const fn limits(&self) -> RelayTransportLimits {
        self.limits
    }
}

impl fmt::Debug for RelaySocketConnectRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelaySocketConnectRequest")
            .field("role", &self.role)
            .field("target", &"<redacted>")
            .field("limits", &self.limits)
            .finish()
    }
}

/// Live socket plus the listener's bounded accepted-session queue.
pub struct RelaySocketConnection {
    socket: SharedRelaySocket,
    accepted: Option<AcceptedRelaySockets>,
    listener_live: Option<Arc<AtomicBool>>,
    listener_task: StdMutex<Option<JoinHandle<()>>>,
}

impl RelaySocketConnection {
    pub fn connected(socket: Arc<dyn RelaySocket>) -> Self {
        Self {
            socket,
            accepted: None,
            listener_live: None,
            listener_task: StdMutex::new(None),
        }
    }

    pub(crate) fn listening(
        socket: Arc<dyn RelaySocket>,
        accepted: mpsc::Receiver<Arc<dyn RelaySocket>>,
        listener_live: Arc<AtomicBool>,
        listener_task: JoinHandle<()>,
    ) -> Self {
        Self {
            socket,
            accepted: Some(Arc::new(Mutex::new(accepted))),
            listener_live: Some(listener_live),
            listener_task: StdMutex::new(Some(listener_task)),
        }
    }

    pub fn is_open(&self) -> bool {
        self.listener_live.as_ref().map_or_else(
            || self.socket.is_open(),
            |live| live.load(Ordering::Acquire),
        )
    }

    pub fn socket(&self) -> Arc<dyn RelaySocket> {
        Arc::clone(&self.socket)
    }

    pub async fn accept(
        &self,
        deadline_remaining_ms: u32,
    ) -> Result<Arc<dyn RelaySocket>, RelaySocketFailure> {
        let receiver = self.accepted.as_ref().ok_or(RelaySocketFailure::Protocol)?;
        tokio::time::timeout(
            Duration::from_millis(u64::from(deadline_remaining_ms)),
            async { receiver.lock().await.recv().await },
        )
        .await
        .map_err(|_| RelaySocketFailure::Unavailable)?
        .ok_or(RelaySocketFailure::Unavailable)
    }

    async fn close(&self) -> Result<(), RelaySocketFailure> {
        if let Some(live) = &self.listener_live {
            live.store(false, Ordering::Release);
        }
        if let Some(task) = self
            .listener_task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
        self.socket.close().await
    }
}

impl Drop for RelaySocketConnection {
    fn drop(&mut self) {
        if let Some(live) = &self.listener_live {
            live.store(false, Ordering::Release);
        }
        if let Some(task) = self
            .listener_task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
}

impl fmt::Debug for RelaySocketConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelaySocketConnection")
            .field("open", &self.is_open())
            .field("listener", &self.accepted.is_some())
            .finish_non_exhaustive()
    }
}

/// Injectable WebSocket connector; production uses tungstenite/rustls.
#[async_trait]
pub trait RelaySocketConnector: Send + Sync {
    async fn connect(
        &self,
        request: RelaySocketConnectRequest,
    ) -> Result<RelaySocketConnection, RelaySocketFailure>;
}

/// Concrete Azure Relay WebSocket connector.
#[derive(Debug, Default, Clone, Copy)]
pub struct TungsteniteRelaySocketConnector;

#[async_trait]
impl RelaySocketConnector for TungsteniteRelaySocketConnector {
    async fn connect(
        &self,
        request: RelaySocketConnectRequest,
    ) -> Result<RelaySocketConnection, RelaySocketFailure> {
        let RelaySocketTarget::Endpoint {
            binding,
            credential,
        } = request.target;
        let socket = connect_endpoint(&binding, request.role, &credential, request.limits).await?;
        if request.role == RelaySocketRole::Sender {
            return Ok(RelaySocketConnection::connected(socket));
        }

        let (accepted_tx, accepted_rx) =
            mpsc::channel(usize::from(request.limits.accept_queue_capacity()));
        let control = Arc::clone(&socket);
        let limits = request.limits;
        let listener_live = Arc::new(AtomicBool::new(true));
        let task_live = Arc::clone(&listener_live);
        let listener_task = tokio::spawn(async move {
            relay_listener_task(binding, credential, control, limits, accepted_tx, task_live).await;
        });
        Ok(RelaySocketConnection::listening(
            socket,
            accepted_rx,
            listener_live,
            listener_task,
        ))
    }
}

struct WebSocketRelaySocket {
    sink: Mutex<WebSocketSink>,
    source: Mutex<WebSocketSource>,
    open: AtomicBool,
    max_frame_bytes: usize,
}

impl WebSocketRelaySocket {
    fn new(socket: WebSocketStream, max_frame_bytes: usize) -> Self {
        let (sink, source) = socket.split();
        Self {
            sink: Mutex::new(sink),
            source: Mutex::new(source),
            open: AtomicBool::new(true),
            max_frame_bytes,
        }
    }
}

impl fmt::Debug for WebSocketRelaySocket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebSocketRelaySocket")
            .field("open", &self.is_open())
            .field("max_frame_bytes", &self.max_frame_bytes)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl RelaySocket for WebSocketRelaySocket {
    fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    async fn receive(&self) -> Result<RelaySocketEvent, RelaySocketFailure> {
        loop {
            let message = self.source.lock().await.next().await;
            let Some(message) = message else {
                self.open.store(false, Ordering::Release);
                return Ok(RelaySocketEvent::Closed);
            };
            let message = match message {
                Ok(message) => message,
                Err(error) => {
                    self.open.store(false, Ordering::Release);
                    return Err(classify_socket_error(error));
                }
            };
            match message {
                Message::Binary(bytes) => {
                    return RelayFrame::new(bytes.to_vec(), self.max_frame_bytes)
                        .map(RelaySocketEvent::Binary);
                }
                Message::Text(text) => {
                    return RelayFrame::new(text.as_bytes().to_vec(), self.max_frame_bytes)
                        .map(RelaySocketEvent::Text);
                }
                Message::Ping(bytes) => {
                    return RelayFrame::new(bytes.to_vec(), self.max_frame_bytes)
                        .map(RelaySocketEvent::Ping);
                }
                Message::Close(_) => {
                    self.open.store(false, Ordering::Release);
                    return Ok(RelaySocketEvent::Closed);
                }
                Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }

    async fn send_binary(&self, bytes: &[u8]) -> Result<(), RelaySocketFailure> {
        if bytes.len() > self.max_frame_bytes {
            return Err(RelaySocketFailure::FrameTooLarge);
        }
        self.sink
            .lock()
            .await
            .send(Message::Binary(bytes.to_vec()))
            .await
            .map_err(|error| {
                self.open.store(false, Ordering::Release);
                classify_socket_error(error)
            })
    }

    async fn send_pong(&self, bytes: &[u8]) -> Result<(), RelaySocketFailure> {
        if bytes.len() > MAX_WEBSOCKET_CONTROL_BYTES {
            return Err(RelaySocketFailure::FrameTooLarge);
        }
        self.sink
            .lock()
            .await
            .send(Message::Pong(bytes.to_vec()))
            .await
            .map_err(|error| {
                self.open.store(false, Ordering::Release);
                classify_socket_error(error)
            })
    }

    async fn close(&self) -> Result<(), RelaySocketFailure> {
        self.open.store(false, Ordering::Release);
        self.sink
            .lock()
            .await
            .close()
            .await
            .map_err(classify_socket_error)
    }
}

struct ActiveRelayResource {
    resource: RelayResource,
    connection: Arc<RelaySocketConnection>,
}

struct OpenReplay {
    request_digest: Fingerprint,
    role: RelaySocketRole,
    resource: RelayResource,
}

struct CloseReplay {
    request_digest: Fingerprint,
    provider_id: ProviderId,
    transport_binding_id: TransportBindingId,
    rendezvous_id: RelayRendezvousId,
    outcome: RelayCloseOutcome,
}

struct ProductionState {
    active: BTreeMap<HandleId, ActiveRelayResource>,
    open_replays: BTreeMap<OperationId, OpenReplay>,
    close_replays: BTreeMap<OperationId, CloseReplay>,
    next_generation: Generation,
}

impl ProductionState {
    fn new() -> Self {
        Self {
            active: BTreeMap::new(),
            open_replays: BTreeMap::new(),
            close_replays: BTreeMap::new(),
            next_generation: Generation::new(1)
                .unwrap_or_else(|_| unreachable!("one is a valid generation")),
        }
    }

    fn take_generation(&mut self) -> Result<Generation, RelayPortFailure> {
        let generation = self.next_generation;
        self.next_generation = generation
            .next()
            .map_err(|_| RelayPortFailure::Unavailable)?;
        Ok(generation)
    }
}

/// Production co-located Relay control/client port.
pub struct ProductionRelayControlPort {
    bindings: BTreeMap<(ProviderId, TransportBindingId, RelayRendezvousId), AzureRelayBinding>,
    credentials: Arc<dyn RelayCredentialSource>,
    connector: Arc<dyn RelaySocketConnector>,
    state: Mutex<ProductionState>,
}

impl ProductionRelayControlPort {
    /// Construct the production port with the real tungstenite/rustls connector.
    pub fn new(
        bindings: impl IntoIterator<Item = AzureRelayBinding>,
        credentials: Arc<dyn RelayCredentialSource>,
    ) -> Result<Self, RelayProductionBuildError> {
        Self::with_socket_connector(
            bindings,
            credentials,
            Arc::new(TungsteniteRelaySocketConnector),
        )
    }

    /// Construct with an injected socket seam for hermetic tests.
    pub fn with_socket_connector(
        bindings: impl IntoIterator<Item = AzureRelayBinding>,
        credentials: Arc<dyn RelayCredentialSource>,
        connector: Arc<dyn RelaySocketConnector>,
    ) -> Result<Self, RelayProductionBuildError> {
        let mut by_id = BTreeMap::new();
        for binding in bindings {
            let key = (
                binding.provider_id.clone(),
                binding.transport_binding_id.clone(),
                binding.rendezvous_id.clone(),
            );
            if by_id.insert(key, binding).is_some() {
                return Err(RelayProductionBuildError::DuplicateBinding);
            }
            if by_id.len() > MAX_PROVIDER_REGISTRY_ENTRIES {
                return Err(RelayProductionBuildError::TooManyBindings);
            }
        }
        if by_id.is_empty() {
            return Err(RelayProductionBuildError::EmptyBindings);
        }
        Ok(Self {
            bindings: by_id,
            credentials,
            connector,
            state: Mutex::new(ProductionState::new()),
        })
    }

    pub async fn connected_socket(&self, handle_id: &HandleId) -> Option<Arc<dyn RelaySocket>> {
        self.state
            .lock()
            .await
            .active
            .get(handle_id)
            .filter(|active| active.resource.state() == RelayResourceState::Connected)
            .map(|active| active.connection.socket())
    }

    pub async fn accept_socket(
        &self,
        handle_id: &HandleId,
        deadline_remaining_ms: u32,
    ) -> Result<Arc<dyn RelaySocket>, RelayPortFailure> {
        if deadline_remaining_ms == 0 {
            return Err(RelayPortFailure::HandshakeTimeout);
        }
        tokio::time::timeout(
            Duration::from_millis(u64::from(deadline_remaining_ms)),
            async {
                let connection = self
                    .state
                    .lock()
                    .await
                    .active
                    .get(handle_id)
                    .filter(|active| active.resource.state() == RelayResourceState::Listening)
                    .map(|active| Arc::clone(&active.connection))
                    .ok_or(RelayPortFailure::IdentityMismatch)?;
                connection
                    .accept(deadline_remaining_ms)
                    .await
                    .map_err(map_socket_failure)
            },
        )
        .await
        .map_err(|_| RelayPortFailure::HandshakeTimeout)?
    }

    fn binding(
        &self,
        provider_id: &ProviderId,
        transport_binding_id: &TransportBindingId,
        rendezvous_id: &RelayRendezvousId,
    ) -> Result<AzureRelayBinding, RelayPortFailure> {
        self.bindings
            .get(&(
                provider_id.clone(),
                transport_binding_id.clone(),
                rendezvous_id.clone(),
            ))
            .cloned()
            .ok_or(RelayPortFailure::BindingMismatch)
    }

    async fn open(
        &self,
        request: RelayOpenRequest,
        credential_use: RelayCredentialUse,
        role: RelaySocketRole,
        state: RelayResourceState,
    ) -> Result<RelayResource, RelayPortFailure> {
        if request.deadline_remaining_ms() == 0 {
            return Err(RelayPortFailure::HandshakeTimeout);
        }
        let timeout = Duration::from_millis(u64::from(request.deadline_remaining_ms()));
        tokio::time::timeout(
            timeout,
            self.open_inner(request, credential_use, role, state),
        )
        .await
        .map_err(|_| RelayPortFailure::CompletionAmbiguous)?
    }

    async fn open_inner(
        &self,
        request: RelayOpenRequest,
        credential_use: RelayCredentialUse,
        role: RelaySocketRole,
        resource_state: RelayResourceState,
    ) -> Result<RelayResource, RelayPortFailure> {
        let binding = self.binding(
            &request.operation().provider_id,
            request.transport_binding_id(),
            request.rendezvous_id(),
        )?;
        let mut state = self.state.lock().await;
        if let Some(replay) = state.open_replays.get(&request.operation().operation_id) {
            return if replay.request_digest == request.operation().request_digest
                && replay.role == role
                && replay.resource.provider_id() == &request.operation().provider_id
                && replay.resource.transport_binding_id() == request.transport_binding_id()
                && replay.resource.rendezvous_id() == request.rendezvous_id()
            {
                Ok(replay.resource.clone())
            } else {
                Err(RelayPortFailure::IdentityMismatch)
            };
        }
        if state.open_replays.len() >= MAX_PROVIDER_REGISTRY_ENTRIES {
            return Err(RelayPortFailure::QueueFull);
        }

        let credential = self
            .credentials
            .resolve(request.credential_lease_id(), credential_use)
            .await
            .map_err(map_credential_failure)?;
        validate_credential_expiry(&credential, request.limits())?;
        let expires_at_unix_ms = credential.expires_at_unix_ms;
        let socket_request = RelaySocketConnectRequest {
            role,
            target: RelaySocketTarget::Endpoint {
                binding: binding.clone(),
                credential: Arc::new(credential),
            },
            limits: request.limits(),
        };
        let connection = match role {
            RelaySocketRole::Sender => {
                connect_sender_retrying(self.connector.as_ref(), socket_request, request.limits())
                    .await?
            }
            RelaySocketRole::Listener => self
                .connector
                .connect(socket_request)
                .await
                .map_err(map_socket_failure)?,
        };
        let handle_id = HandleId::parse(request.operation().operation_id.as_str())
            .map_err(|_| RelayPortFailure::IdentityMismatch)?;
        let resource = RelayResource::new(
            request.operation().provider_id.clone(),
            binding.transport_binding_id,
            binding.rendezvous_id,
            handle_id.clone(),
            request.operation().provider_generation,
            state.take_generation()?,
            resource_state,
            Some(expires_at_unix_ms),
        );
        state.active.insert(
            handle_id,
            ActiveRelayResource {
                resource: resource.clone(),
                connection: Arc::new(connection),
            },
        );
        state.open_replays.insert(
            request.operation().operation_id.clone(),
            OpenReplay {
                request_digest: request.operation().request_digest.clone(),
                role,
                resource: resource.clone(),
            },
        );
        Ok(resource)
    }

    async fn inspect_inner(
        &self,
        request: &RelayInspectRequest,
    ) -> Result<RelayInspection, RelayPortFailure> {
        self.binding(
            &request.operation().provider_id,
            request.transport_binding_id(),
            request.rendezvous_id(),
        )?;
        let state = self.state.lock().await;
        let active = state
            .active
            .values()
            .filter(|active| {
                active.resource.transport_binding_id() == request.transport_binding_id()
                    && active.resource.rendezvous_id() == request.rendezvous_id()
                    && active.resource.provider_id() == &request.operation().provider_id
            })
            .max_by_key(|active| active.resource.resource_generation());
        let Some(active) = active else {
            return Ok(RelayInspection::Absent);
        };
        if active.connection.is_open() {
            Ok(RelayInspection::Present(active.resource.clone()))
        } else {
            Ok(RelayInspection::Present(resource_with_state(
                &active.resource,
                RelayResourceState::Closed,
            )))
        }
    }

    async fn close_inner(
        &self,
        request: RelayCloseRequest,
    ) -> Result<RelayCloseOutcome, RelayPortFailure> {
        self.binding(
            &request.operation().provider_id,
            request.transport_binding_id(),
            request.rendezvous_id(),
        )?;
        let mut state = self.state.lock().await;
        if let Some(replay) = state.close_replays.get(&request.operation().operation_id) {
            return if replay.request_digest == request.operation().request_digest
                && replay.provider_id == request.operation().provider_id
                && &replay.transport_binding_id == request.transport_binding_id()
                && &replay.rendezvous_id == request.rendezvous_id()
            {
                Ok(replay.outcome)
            } else {
                Err(RelayPortFailure::IdentityMismatch)
            };
        }
        if state.close_replays.len() >= MAX_PROVIDER_REGISTRY_ENTRIES {
            return Err(RelayPortFailure::QueueFull);
        }
        let matching: Vec<_> = state
            .active
            .iter()
            .filter(|(_, active)| {
                active.resource.transport_binding_id() == request.transport_binding_id()
                    && active.resource.rendezvous_id() == request.rendezvous_id()
                    && active.resource.provider_id() == &request.operation().provider_id
            })
            .map(|(handle_id, _)| handle_id.clone())
            .collect();
        let outcome = if matching.is_empty() {
            if state.open_replays.values().any(|replay| {
                replay.resource.transport_binding_id() == request.transport_binding_id()
                    && replay.resource.rendezvous_id() == request.rendezvous_id()
                    && replay.resource.provider_id() == &request.operation().provider_id
            }) {
                RelayCloseOutcome::AlreadyClosed
            } else {
                RelayCloseOutcome::NotFound
            }
        } else {
            for handle_id in &matching {
                let active = state
                    .active
                    .get(handle_id)
                    .ok_or(RelayPortFailure::CompletionAmbiguous)?;
                active
                    .connection
                    .close()
                    .await
                    .map_err(|_| RelayPortFailure::CompletionAmbiguous)?;
            }
            for handle_id in matching {
                state.active.remove(&handle_id);
            }
            RelayCloseOutcome::Closed
        };
        state.close_replays.insert(
            request.operation().operation_id.clone(),
            CloseReplay {
                request_digest: request.operation().request_digest.clone(),
                provider_id: request.operation().provider_id.clone(),
                transport_binding_id: request.transport_binding_id().clone(),
                rendezvous_id: request.rendezvous_id().clone(),
                outcome,
            },
        );
        Ok(outcome)
    }
}

impl fmt::Debug for ProductionRelayControlPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProductionRelayControlPort")
            .field("binding_count", &self.bindings.len())
            .field("credential_source", &"<co-located>")
            .field("socket_connector", &"<configured>")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl RelayControlPort for ProductionRelayControlPort {
    fn capabilities(&self) -> RelayPortCapabilities {
        RelayPortCapabilities::production()
    }

    async fn connect(&self, request: RelayOpenRequest) -> Result<RelayResource, RelayPortFailure> {
        self.open(
            request,
            RelayCredentialUse::Connect,
            RelaySocketRole::Sender,
            RelayResourceState::Connected,
        )
        .await
    }

    async fn listen(&self, request: RelayOpenRequest) -> Result<RelayResource, RelayPortFailure> {
        self.open(
            request,
            RelayCredentialUse::Listen,
            RelaySocketRole::Listener,
            RelayResourceState::Listening,
        )
        .await
    }

    async fn inspect(
        &self,
        request: RelayInspectRequest,
    ) -> Result<RelayInspection, RelayPortFailure> {
        if request.deadline_remaining_ms() == 0 {
            return Err(RelayPortFailure::HandshakeTimeout);
        }
        tokio::time::timeout(
            Duration::from_millis(u64::from(request.deadline_remaining_ms())),
            self.inspect_inner(&request),
        )
        .await
        .map_err(|_| RelayPortFailure::HandshakeTimeout)?
    }

    async fn close(
        &self,
        request: RelayCloseRequest,
    ) -> Result<RelayCloseOutcome, RelayPortFailure> {
        if request.deadline_remaining_ms() == 0 {
            return Err(RelayPortFailure::HandshakeTimeout);
        }
        tokio::time::timeout(
            Duration::from_millis(u64::from(request.deadline_remaining_ms())),
            self.close_inner(request),
        )
        .await
        .map_err(|_| RelayPortFailure::CompletionAmbiguous)?
    }

    async fn adopt(&self, request: RelayAdoptRequest) -> Result<RelayResource, RelayPortFailure> {
        if request.deadline_remaining_ms() == 0 {
            return Err(RelayPortFailure::HandshakeTimeout);
        }
        tokio::time::timeout(
            Duration::from_millis(u64::from(request.deadline_remaining_ms())),
            async {
                self.binding(
                    &request.operation().provider_id,
                    request.transport_binding_id(),
                    request.rendezvous_id(),
                )?;
                let state = self.state.lock().await;
                let active = state
                    .active
                    .get(request.expected().handle_id())
                    .ok_or(RelayPortFailure::IdentityMismatch)?;
                if active.resource.transport_binding_id() != request.transport_binding_id()
                    || active.resource.rendezvous_id() != request.rendezvous_id()
                {
                    return Err(RelayPortFailure::BindingMismatch);
                }
                validate_expected_resource(&active.resource, request.expected())?;
                if !active.connection.is_open() {
                    return Err(RelayPortFailure::Unavailable);
                }
                Ok(active.resource.clone())
            },
        )
        .await
        .map_err(|_| RelayPortFailure::HandshakeTimeout)?
    }
}

async fn connect_sender_retrying(
    connector: &dyn RelaySocketConnector,
    request: RelaySocketConnectRequest,
    limits: RelayTransportLimits,
) -> Result<RelaySocketConnection, RelayPortFailure> {
    let mut retries = 0u8;
    loop {
        match connector.connect(request.clone()).await {
            Ok(connection) => return Ok(connection),
            Err(RelaySocketFailure::ListenerNotReady) if retries < limits.sender_retry_limit() => {
                retries = retries.saturating_add(1);
                tokio::time::sleep(Duration::from_millis(u64::from(
                    limits.sender_retry_delay_ms(),
                )))
                .await;
            }
            Err(failure) => return Err(map_socket_failure(failure)),
        }
    }
}

async fn connect_endpoint(
    binding: &AzureRelayBinding,
    role: RelaySocketRole,
    credential: &RelayCredentialLease,
    limits: RelayTransportLimits,
) -> Result<Arc<dyn RelaySocket>, RelaySocketFailure> {
    install_crypto_provider();
    let request = endpoint_request(binding, role, credential)?;
    let connector = tls_connector(binding.additional_ca_pem.as_ref())?;
    let max_frame_bytes =
        usize::try_from(limits.max_frame_bytes()).map_err(|_| RelaySocketFailure::FrameTooLarge)?;
    let config = WebSocketConfig {
        max_message_size: Some(max_frame_bytes),
        max_frame_size: Some(max_frame_bytes),
        ..WebSocketConfig::default()
    };
    let (socket, _) = tokio_tungstenite::connect_async_tls_with_config(
        request,
        Some(config),
        false,
        Some(connector),
    )
    .await
    .map_err(classify_socket_error)?;
    Ok(Arc::new(WebSocketRelaySocket::new(socket, max_frame_bytes)))
}

fn endpoint_request(
    binding: &AzureRelayBinding,
    role: RelaySocketRole,
    credential: &RelayCredentialLease,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, RelaySocketFailure> {
    let action = match role {
        RelaySocketRole::Sender => "connect",
        RelaySocketRole::Listener => "listen",
    };
    let base = format!(
        "wss://{}/$hc/{}?sb-hc-action={action}",
        binding.namespace,
        urlencoding::encode(&binding.entity),
    );
    let mut auth_header: Option<Zeroizing<String>> = None;
    let url = match &credential.material {
        RelayCredentialMaterial::SasRule { key_name, key } => {
            let token = mint_sas(binding, key_name, key, credential.expires_at_unix_ms)?;
            Zeroizing::new(format!(
                "{base}&sb-hc-token={}",
                urlencoding::encode(token.as_str())
            ))
        }
        RelayCredentialMaterial::SasToken(token) => Zeroizing::new(format!(
            "{base}&sb-hc-token={}",
            urlencoding::encode(token.utf8()?)
        )),
        RelayCredentialMaterial::EntraBearer(token) => {
            auth_header = Some(Zeroizing::new(token.utf8()?.to_owned()));
            Zeroizing::new(base)
        }
    };
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| RelaySocketFailure::InvalidEndpoint)?;
    if let Some(header) = auth_header {
        request.headers_mut().insert(
            "servicebusauthorization",
            HeaderValue::from_bytes(header.as_bytes())
                .map_err(|_| RelaySocketFailure::AuthenticationFailed)?,
        );
    }
    Ok(request)
}

fn mint_sas(
    binding: &AzureRelayBinding,
    key_name: &RelaySecret,
    key: &RelaySecret,
    expires_at_unix_ms: u64,
) -> Result<Zeroizing<String>, RelaySocketFailure> {
    let resource = format!("http://{}/{}", binding.namespace, binding.entity);
    let resource = urlencoding::encode(&resource).to_lowercase();
    let expiry = expires_at_unix_ms / 1_000;
    let to_sign = Zeroizing::new(format!("{resource}\n{expiry}"));
    let mut mac = HmacSha256::new_from_slice(key.as_bytes())
        .map_err(|_| RelaySocketFailure::AuthenticationFailed)?;
    mac.update(to_sign.as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    let signature = Zeroizing::new(signature);
    Ok(Zeroizing::new(format!(
        "SharedAccessSignature sr={resource}&sig={}&se={expiry}&skn={}",
        urlencoding::encode(signature.as_str()),
        urlencoding::encode(key_name.utf8()?)
    )))
}

fn tls_connector(
    additional_ca_pem: Option<&RelaySecret>,
) -> Result<tokio_tungstenite::Connector, RelaySocketFailure> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(pem) = additional_ca_pem {
        for certificate in CertificateDer::pem_slice_iter(pem.as_bytes()) {
            roots
                .add(certificate.map_err(|_| RelaySocketFailure::InvalidEndpoint)?)
                .map_err(|_| RelaySocketFailure::InvalidEndpoint)?;
        }
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(tokio_tungstenite::Connector::Rustls(Arc::new(config)))
}

fn install_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

async fn relay_listener_task(
    binding: AzureRelayBinding,
    credential: Arc<RelayCredentialLease>,
    initial_control: Arc<dyn RelaySocket>,
    limits: RelayTransportLimits,
    accepted: mpsc::Sender<Arc<dyn RelaySocket>>,
    live: Arc<AtomicBool>,
) {
    let initial_backoff_ms = u32::from(limits.sender_retry_delay_ms());
    let mut backoff_ms = initial_backoff_ms;
    let mut control = initial_control;
    'listener: loop {
        live.store(true, Ordering::Release);
        let connected_at = Instant::now();
        relay_listener_accept_loop(
            &control,
            binding.additional_ca_pem.as_ref(),
            limits,
            &accepted,
        )
        .await;
        live.store(false, Ordering::Release);
        if accepted.is_closed() || !credential_is_unexpired(&credential) {
            break;
        }
        if connected_at.elapsed()
            >= Duration::from_millis(u64::from(limits.reconnect_stable_reset_ms()))
        {
            backoff_ms = initial_backoff_ms;
        }

        loop {
            let wait_ms = backoff_ms;
            backoff_ms = backoff_ms
                .saturating_mul(2)
                .min(limits.max_reconnect_backoff_ms());
            tokio::time::sleep(Duration::from_millis(u64::from(wait_ms))).await;
            if accepted.is_closed() || !credential_is_unexpired(&credential) {
                break 'listener;
            }
            let reconnect = tokio::time::timeout(
                Duration::from_millis(u64::from(limits.max_reconnect_backoff_ms())),
                connect_endpoint(&binding, RelaySocketRole::Listener, &credential, limits),
            )
            .await;
            if let Ok(Ok(next)) = reconnect {
                control = next;
                break;
            }
        }
    }
    live.store(false, Ordering::Release);
}

async fn relay_listener_accept_loop(
    control: &Arc<dyn RelaySocket>,
    ca_pem: Option<&RelaySecret>,
    limits: RelayTransportLimits,
    accepted: &mpsc::Sender<Arc<dyn RelaySocket>>,
) {
    while !accepted.is_closed() && control.is_open() {
        match control.receive().await {
            Ok(RelaySocketEvent::Text(text)) => {
                let Some(address) = extract_accept_address(&text, limits.max_frame_bytes()) else {
                    continue;
                };
                let connection = tokio::time::timeout(
                    Duration::from_millis(u64::from(limits.max_reconnect_backoff_ms())),
                    connect_rendezvous(address, ca_pem, limits),
                )
                .await;
                if let Ok(Ok(socket)) = connection {
                    match accepted.try_send(socket) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(socket)) => {
                            let _ = socket.close().await;
                        }
                        Err(mpsc::error::TrySendError::Closed(socket)) => {
                            let _ = socket.close().await;
                            break;
                        }
                    }
                }
            }
            Ok(RelaySocketEvent::Ping(bytes)) => {
                if control.send_pong(bytes.as_bytes()).await.is_err() {
                    break;
                }
            }
            Ok(RelaySocketEvent::Closed) | Err(_) => break,
            Ok(RelaySocketEvent::Binary(_)) => {}
        }
    }
}

fn credential_is_unexpired(credential: &RelayCredentialLease) -> bool {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .is_some_and(|now| now < credential.expires_at_unix_ms)
}

fn extract_accept_address(text: &RelayFrame, max_frame_bytes: u32) -> Option<RelaySecret> {
    let value: serde_json::Value = serde_json::from_slice(text.as_bytes()).ok()?;
    let address = value.get("accept")?.get("address")?.as_str()?;
    if address.len() > usize::try_from(max_frame_bytes).ok()? || !valid_rendezvous_address(address)
    {
        return None;
    }
    RelaySecret::new(address.as_bytes().to_vec()).ok()
}

async fn connect_rendezvous(
    address: RelaySecret,
    additional_ca_pem: Option<&RelaySecret>,
    limits: RelayTransportLimits,
) -> Result<Arc<dyn RelaySocket>, RelaySocketFailure> {
    install_crypto_provider();
    let address = address.utf8()?;
    let request = address
        .into_client_request()
        .map_err(|_| RelaySocketFailure::InvalidEndpoint)?;
    let connector = tls_connector(additional_ca_pem)?;
    let max_frame_bytes =
        usize::try_from(limits.max_frame_bytes()).map_err(|_| RelaySocketFailure::FrameTooLarge)?;
    let config = WebSocketConfig {
        max_message_size: Some(max_frame_bytes),
        max_frame_size: Some(max_frame_bytes),
        ..WebSocketConfig::default()
    };
    let (socket, _) = tokio_tungstenite::connect_async_tls_with_config(
        request,
        Some(config),
        false,
        Some(connector),
    )
    .await
    .map_err(classify_socket_error)?;
    Ok(Arc::new(WebSocketRelaySocket::new(socket, max_frame_bytes)))
}

fn classify_socket_error(error: tokio_tungstenite::tungstenite::Error) -> RelaySocketFailure {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => match response.status().as_u16() {
            401 | 403 => RelaySocketFailure::AuthenticationFailed,
            404 => RelaySocketFailure::ListenerNotReady,
            _ => RelaySocketFailure::Unavailable,
        },
        tokio_tungstenite::tungstenite::Error::Capacity(_) => RelaySocketFailure::FrameTooLarge,
        tokio_tungstenite::tungstenite::Error::Url(_)
        | tokio_tungstenite::tungstenite::Error::HttpFormat(_) => {
            RelaySocketFailure::InvalidEndpoint
        }
        tokio_tungstenite::tungstenite::Error::Protocol(_) => RelaySocketFailure::Protocol,
        _ => RelaySocketFailure::Unavailable,
    }
}

fn validate_credential_expiry(
    credential: &RelayCredentialLease,
    limits: RelayTransportLimits,
) -> Result<(), RelayPortFailure> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RelayPortFailure::CredentialLeaseInvalid)
        .and_then(|elapsed| {
            u64::try_from(elapsed.as_millis()).map_err(|_| RelayPortFailure::CredentialLeaseInvalid)
        })?;
    let max_lifetime_ms = u64::from(limits.max_credential_ttl_secs()) * 1_000;
    if credential.expires_at_unix_ms <= now
        || credential.expires_at_unix_ms.saturating_sub(now) > max_lifetime_ms
    {
        return Err(RelayPortFailure::CredentialLeaseInvalid);
    }
    Ok(())
}

fn map_credential_failure(failure: RelayCredentialSourceFailure) -> RelayPortFailure {
    match failure {
        RelayCredentialSourceFailure::LeaseUnknown
        | RelayCredentialSourceFailure::LeaseExpired
        | RelayCredentialSourceFailure::RoleMismatch => RelayPortFailure::CredentialLeaseInvalid,
        RelayCredentialSourceFailure::Unavailable => RelayPortFailure::Unavailable,
    }
}

fn map_socket_failure(failure: RelaySocketFailure) -> RelayPortFailure {
    match failure {
        RelaySocketFailure::AuthenticationFailed => RelayPortFailure::AuthenticationFailed,
        RelaySocketFailure::ListenerNotReady | RelaySocketFailure::Unavailable => {
            RelayPortFailure::Unavailable
        }
        RelaySocketFailure::InvalidEndpoint => RelayPortFailure::BindingMismatch,
        RelaySocketFailure::FrameTooLarge => RelayPortFailure::FrameTooLarge,
        RelaySocketFailure::Protocol => RelayPortFailure::Unavailable,
    }
}

fn resource_with_state(resource: &RelayResource, state: RelayResourceState) -> RelayResource {
    RelayResource::new(
        resource.provider_id().clone(),
        resource.transport_binding_id().clone(),
        resource.rendezvous_id().clone(),
        resource.handle_id().clone(),
        resource.provider_generation(),
        resource.resource_generation(),
        state,
        resource.expires_at_unix_ms(),
    )
}

fn validate_expected_resource(
    resource: &RelayResource,
    expected: &RelayExpectedResource,
) -> Result<(), RelayPortFailure> {
    if resource.provider_id() != expected.provider_id()
        || resource.handle_id() != expected.handle_id()
    {
        return Err(RelayPortFailure::IdentityMismatch);
    }
    if resource.provider_generation() != expected.provider_generation()
        || resource.resource_generation() != expected.resource_generation()
    {
        return Err(RelayPortFailure::GenerationMismatch);
    }
    Ok(())
}

fn valid_namespace(namespace: &str) -> bool {
    namespace.len() <= MAX_RELAY_NAMESPACE_BYTES && valid_servicebus_host(namespace)
}

fn valid_entity(entity: &str) -> bool {
    !entity.is_empty()
        && entity.len() <= MAX_RELAY_ENTITY_BYTES
        && entity
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_rendezvous_address(address: &str) -> bool {
    let Ok(request) = address.into_client_request() else {
        return false;
    };
    let uri = request.uri();
    let authority = uri.authority().map(|value| value.as_str()).unwrap_or("");
    uri.scheme_str() == Some("wss")
        && !authority.contains('@')
        && uri.host().is_some_and(valid_servicebus_host)
        && uri.port_u16().is_none_or(|port| port == 443)
}

fn valid_servicebus_host(host: &str) -> bool {
    let Some(label) = host.strip_suffix(".servicebus.windows.net") else {
        return false;
    };
    !label.is_empty()
        && label.len() <= 63
        && !label.contains('.')
        && label
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && label
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod internal_tests {
    use super::*;

    fn test_binding() -> AzureRelayBinding {
        AzureRelayBinding::new(
            ProviderId::parse("ccccccccccccccccccca").expect("provider"),
            TransportBindingId::parse("test-binding").expect("binding"),
            RelayRendezvousId::parse("test-rendezvous").expect("rendezvous"),
            "test-relay.servicebus.windows.net",
            "hybrid-connection",
            None,
        )
        .expect("private endpoint")
    }

    #[test]
    fn relay_authentication_uses_the_exact_azure_websocket_surfaces() {
        let sas = RelayCredentialLease::new(
            RelayCredentialMaterial::SasToken(
                RelaySecret::new(b"SharedAccessSignature secret-canary".to_vec())
                    .expect("SAS token"),
            ),
            1_800_000_000_000,
        );
        let request =
            endpoint_request(&test_binding(), RelaySocketRole::Sender, &sas).expect("SAS request");
        let uri = request.uri().to_string();
        assert!(uri.contains("sb-hc-action=connect"));
        assert!(uri.contains("sb-hc-token="));
        assert!(request.headers().get("servicebusauthorization").is_none());

        let entra = RelayCredentialLease::new(
            RelayCredentialMaterial::EntraBearer(
                RelaySecret::new(b"entra-bearer-canary".to_vec()).expect("bearer"),
            ),
            1_800_000_000_000,
        );
        let request = endpoint_request(&test_binding(), RelaySocketRole::Listener, &entra)
            .expect("Entra request");
        let uri = request.uri().to_string();
        assert!(uri.contains("sb-hc-action=listen"));
        assert!(!uri.contains("entra-bearer-canary"));
        assert!(!uri.contains("sb-hc-token="));
        assert!(
            request
                .headers()
                .get("servicebusauthorization")
                .is_some_and(|value| value.as_bytes() == b"entra-bearer-canary")
        );
    }

    #[test]
    fn relay_frames_enforce_the_exact_production_bound_and_redact_bytes() {
        let bound = usize::try_from(RelayTransportLimits::production().max_frame_bytes())
            .expect("frame bound fits usize");
        let frame = RelayFrame::new(vec![b'x'; bound], bound).expect("frame at bound");
        assert_eq!(frame.as_bytes().len(), bound);
        assert!(!format!("{frame:?}").contains("xxxx"));
        assert_eq!(
            RelayFrame::new(vec![b'x'; bound + 1], bound).expect_err("frame above bound"),
            RelaySocketFailure::FrameTooLarge
        );
    }

    #[test]
    fn rendezvous_addresses_are_restricted_to_tls_service_bus_hosts() {
        assert!(valid_rendezvous_address(
            "wss://g1-prod-relay-sb.servicebus.windows.net/$hc/entity?sb-hc-action=accept"
        ));
        assert!(!valid_rendezvous_address(
            "ws://g1-prod-relay-sb.servicebus.windows.net/$hc/entity"
        ));
        assert!(!valid_rendezvous_address(
            "wss://user@g1-prod-relay-sb.servicebus.windows.net/$hc/entity"
        ));
        assert!(!valid_rendezvous_address(
            "wss://relay.example.invalid/$hc/entity"
        ));
    }
}
