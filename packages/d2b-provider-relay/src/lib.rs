//! `d2b-provider-relay`: the Azure Relay transport auth/credential core
//! for the realm gateway (ADR 0032).
//!
//! This crate is the d2b-native home for the **credential model +
//! connect contract** that the gateway's relay transport and the in-sandbox
//! sender are built on.
//!
//! ## Three-plane mapping
//! - The **gateway** (host side) holds the relay **Listen** credential and
//!   opens the listener control channel. Listen auth is a gateway-side SAS
//!   minted from the `gateway-listen` rule key, or (later) the gateway's own
//!   Entra **Listener** role.
//! - The **container** (sandbox sender) authenticates with either an **Entra
//!   bearer token from its managed identity** or a **gateway-minted,
//!   short-lived Send SAS bearer**. The ACA display path uses the latter because
//!   ACA Relay Entra substreams closed during Waypipe forwarding; the long-lived
//!   SAS rule key still never enters the sandbox.
//!
//! Every secret ([`RelayCredential`] material, minted SAS, bearer token) has
//! a redacted `Debug` so it can never reach a log, span, or audit record.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine;
use d2b_realm_core::{ErrorKind, NodeId, ProviderId};
use d2b_realm_provider::error::{ProviderError, ProviderResult};
use d2b_realm_provider::provider::{TransportListener, TransportProvider};
use d2b_realm_provider::types::{NodeRegistration, SafeLabel, TransportSession, TransportTarget};
use hmac::{Hmac, Mac};
use rustls_pki_types::{CertificateDer, pem::PemObject};
use sha2::Sha256;
use tokio::sync::{Mutex, mpsc};

type HmacSha256 = Hmac<Sha256>;

/// The Entra resource (audience) a managed identity requests a token for to
/// authenticate to Azure Relay. Confirmed against the Azure Relay docs.
pub const RELAY_TOKEN_RESOURCE: &str = "https://relay.azure.net/";

/// The role an endpoint plays on the hybrid connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayRole {
    /// The gateway side that accepts sender connections.
    Listener,
    /// The sandbox side that dials out to send.
    Sender,
}

impl RelayRole {
    /// The `sb-hc-action` query value for this role.
    fn action(self) -> &'static str {
        match self {
            RelayRole::Listener => "listen",
            RelayRole::Sender => "connect",
        }
    }
}

/// A hybrid-connection endpoint: the relay namespace FQDN + the entity
/// (hybrid connection) name. Non-secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayEndpoint {
    /// Namespace FQDN, e.g. `relns-xxxx.servicebus.windows.net`.
    pub namespace: String,
    /// Hybrid connection (entity) name, e.g. `hc-d2b-display`.
    pub entity: String,
}

/// How an endpoint authenticates to the relay. Both variants wrap secret
/// material and therefore redact their `Debug`.
#[derive(Clone)]
pub enum RelayCredential {
    /// A Shared Access Signature: an authorization-rule name + its key. Used
    /// gateway-side (the Listen rule), and transitionally for non-MI senders.
    Sas {
        /// The authorization-rule (key) name, e.g. `gateway-listen`.
        key_name: String,
        /// The rule's key. Secret.
        key: String,
    },
    /// A pre-minted Shared Access Signature bearer. The gateway uses this for
    /// short-lived Send tokens handed to ACA sandboxes without exposing the
    /// underlying rule key.
    SasToken(String),
    /// A Microsoft Entra bearer token acquired by a managed identity for
    /// [`RELAY_TOKEN_RESOURCE`]. The productionized container path. Secret.
    EntraBearer(String),
}

impl fmt::Debug for RelayCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The key name is a non-secret label; the key/token are redacted.
            RelayCredential::Sas { key_name, .. } => f
                .debug_struct("RelayCredential::Sas")
                .field("key_name", key_name)
                .field("key", &"<redacted>")
                .finish(),
            RelayCredential::SasToken(_) => f.write_str("RelayCredential::SasToken(<redacted>)"),
            RelayCredential::EntraBearer(_) => {
                f.write_str("RelayCredential::EntraBearer(<redacted>)")
            }
        }
    }
}

/// The bytes a [`RelayCredential`] resolves to for a WebSocket connect: a SAS
/// goes in the `sb-hc-token` query parameter; an Entra token goes in the
/// `ServiceBusAuthorization` header. Exactly one is set. Redacted `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayConnect {
    /// The `wss://…` URL (already URL-encoded; never contains the bearer).
    pub url: String,
    /// The `ServiceBusAuthorization` header value (`Bearer <jwt>`), when the
    /// credential is an Entra token.
    pub auth_header: Option<String>,
}

impl fmt::Debug for RelayConnect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The URL may carry an sb-hc-token SAS; redact the whole URL query and
        // never print the header (it carries the bearer).
        let scheme_host = self.url.split('?').next().unwrap_or("");
        f.debug_struct("RelayConnect")
            .field("url", &format!("{scheme_host}?<redacted>"))
            .field(
                "auth_header",
                &self.auth_header.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// The maximum minted-SAS lifetime (seconds) accepted for relay sessions.
pub const MAX_SAS_TTL_SECS: u64 = 15 * 60;

/// The default minted-SAS lifetime (seconds). The gateway mints short-lived
/// SAS bearers; a long-lived token is never persisted.
pub const DEFAULT_SAS_TTL_SECS: u64 = MAX_SAS_TTL_SECS;

/// Mint a Service Bus SAS token conferring the rule's rights on the entity,
/// expiring `ttl_secs` from now. This is the gateway-side minting the POC's
/// relay bridge proved; it is reproduced here byte-for-byte.
///
/// The returned string is secret (it is a bearer); callers must treat it as
/// such (it is never logged by this crate).
pub fn mint_sas(
    endpoint: &RelayEndpoint,
    key_name: &str,
    key: &str,
    ttl_secs: u64,
) -> Result<String, RelayError> {
    if ttl_secs > MAX_SAS_TTL_SECS {
        return Err(RelayError::TtlTooLong {
            requested: ttl_secs,
            max: MAX_SAS_TTL_SECS,
        });
    }

    let resource = format!("http://{}/{}", endpoint.namespace, endpoint.entity);
    let resource_enc = urlencoding::encode(&resource).to_lowercase();
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RelayError::Clock)?
        .as_secs()
        + ttl_secs;
    let to_sign = format!("{resource_enc}\n{expiry}");
    let mut mac = HmacSha256::new_from_slice(key.as_bytes()).map_err(|_| RelayError::Key)?;
    mac.update(to_sign.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    let sig_enc = urlencoding::encode(&sig);
    Ok(format!(
        "SharedAccessSignature sr={resource_enc}&sig={sig_enc}&se={expiry}&skn={key_name}"
    ))
}

/// Build the relay WebSocket connect contract for `role` using `credential`.
/// SAS authentication mints a token into the `sb-hc-token` query parameter;
/// Entra authentication leaves the URL token-free and returns the
/// `ServiceBusAuthorization: Bearer <jwt>` header. A pre-minted SAS bearer is
/// also accepted for the ACA path; it is already scoped/expiring, so this
/// function only URL-encodes it into `sb-hc-token`.
pub fn build_connect(
    endpoint: &RelayEndpoint,
    role: RelayRole,
    credential: &RelayCredential,
    ttl_secs: u64,
) -> Result<RelayConnect, RelayError> {
    // The sender does NOT supply its own `sb-hc-id`. Azure Relay generates the
    // rendezvous correlation id (a GUID) and embeds it in the accept message's
    // address; a caller-supplied non-GUID id yields an unserviceable rendezvous
    // address that the listener's accept connect rejects with 400. This matches
    // the official Relay SDKs, which omit `sb-hc-id` on connect.
    let base = format!(
        "wss://{}/$hc/{}?sb-hc-action={}",
        endpoint.namespace,
        urlencoding::encode(&endpoint.entity),
        role.action(),
    );
    match credential {
        RelayCredential::EntraBearer(token) => Ok(RelayConnect {
            url: base,
            auth_header: Some(format!("Bearer {token}")),
        }),
        RelayCredential::SasToken(token) => Ok(RelayConnect {
            url: format!("{base}&sb-hc-token={}", urlencoding::encode(token)),
            auth_header: None,
        }),
        RelayCredential::Sas { key_name, key } => {
            let token = mint_sas(endpoint, key_name, key, ttl_secs)?;
            Ok(RelayConnect {
                url: format!("{base}&sb-hc-token={}", urlencoding::encode(&token)),
                auth_header: None,
            })
        }
    }
}

/// Errors building relay auth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayError {
    /// The system clock was before the Unix epoch.
    Clock,
    /// The SAS key was not valid HMAC key material.
    Key,
    /// The requested SAS TTL exceeded the short-lived bearer bound.
    TtlTooLong { requested: u64, max: u64 },
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayError::Clock => write!(f, "system clock is before the unix epoch"),
            RelayError::Key => write!(f, "relay SAS key is invalid"),
            RelayError::TtlTooLong { requested, max } => write!(
                f,
                "relay SAS TTL {requested}s exceeds maximum short-lived bound {max}s"
            ),
        }
    }
}

impl std::error::Error for RelayError {}

/// A connected relay WebSocket stream (TLS over TCP).
pub type RelayStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Install the process-global rustls crypto provider (ring) if one is not
/// already installed. [`connect`] calls this lazily, so consumers normally do
/// not need to; it is exposed so an application that wants to pick the
/// provider can install its own first (this call then no-ops). Idempotent.
pub fn install_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // install_default() returns Err if a provider is already installed
        // (e.g. the host application chose one); respect that and no-op.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Connect to the relay for `role` with `credential`, returning the live
/// WebSocket stream. This is the host/gateway-side connect; it uses the
/// public webpki roots (the ACA egress-proxy CA is only needed *inside* the
/// sandbox, not on the gateway). The Entra bearer, when present, is sent in
/// the `ServiceBusAuthorization` header — never in the URL.
pub async fn connect(
    endpoint: &RelayEndpoint,
    role: RelayRole,
    credential: &RelayCredential,
    ttl_secs: u64,
) -> Result<RelayStream, RelayConnectError> {
    connect_with_ca(endpoint, role, credential, ttl_secs, None).await
}

/// Like [`connect`], but trusts an extra PEM CA bundle in addition to the
/// built-in webpki roots. Required **inside an ACA sandbox**, whose
/// transparent egress proxy terminates TLS with the injected
/// `adc-egress-proxy-ca`; the gateway (host) side passes `None`.
pub async fn connect_with_ca(
    endpoint: &RelayEndpoint,
    role: RelayRole,
    credential: &RelayCredential,
    ttl_secs: u64,
    ca_pem: Option<&[u8]>,
) -> Result<RelayStream, RelayConnectError> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};

    install_crypto_provider();

    let connect =
        build_connect(endpoint, role, credential, ttl_secs).map_err(RelayConnectError::Auth)?;
    let mut request = connect
        .url
        .into_client_request()
        .map_err(|_| RelayConnectError::BadRequest)?;
    if let Some(value) = &connect.auth_header {
        request.headers_mut().insert(
            HeaderName::from_static("servicebusauthorization"),
            HeaderValue::from_str(value).map_err(|_| RelayConnectError::BadRequest)?,
        );
    }
    connect_request(request, ca_pem).await
}

/// Connect a rendezvous URL (the listener-side accept address; it already
/// carries its own token and rendezvous id) with the optional extra CA. The
/// relay routes the rendezvous to a per-connection backend host
/// (`g<N>-prod-…-sb.servicebus.windows.net`); the dial targets that host
/// verbatim, exactly as the official Relay SDK listeners do.
async fn connect_raw(url: &str, ca_pem: Option<&[u8]>) -> Result<RelayStream, RelayConnectError> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    install_crypto_provider();
    let request = url
        .into_client_request()
        .map_err(|_| RelayConnectError::BadRequest)?;
    connect_request(request, ca_pem).await
}

async fn connect_request(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
    ca_pem: Option<&[u8]>,
) -> Result<RelayStream, RelayConnectError> {
    let connector = tls_connector(ca_pem)?;
    let (ws, _resp) =
        tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
            .await
            .map_err(|err| RelayConnectError::Handshake(err.to_string()))?;
    Ok(ws)
}

/// Build a rustls connector trusting the built-in webpki roots plus any extra
/// CA certificates in `ca_pem`.
fn tls_connector(ca_pem: Option<&[u8]>) -> Result<tokio_tungstenite::Connector, RelayConnectError> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(pem) = ca_pem {
        for cert in CertificateDer::pem_slice_iter(pem) {
            roots
                .add(cert.map_err(|_| RelayConnectError::BadRequest)?)
                .map_err(|_| RelayConnectError::BadRequest)?;
        }
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(tokio_tungstenite::Connector::Rustls(std::sync::Arc::new(
        config,
    )))
}

/// Errors connecting the relay WebSocket.
#[derive(Debug)]
pub enum RelayConnectError {
    /// Building the auth (SAS mint / header) failed.
    Auth(RelayError),
    /// The connect URL/header could not be turned into a request.
    BadRequest,
    /// The relay rejected or failed the WebSocket handshake (e.g. a 401 when
    /// the credential is unauthorized). The message is the bounded tungstenite
    /// error class; it never carries the token.
    Handshake(String),
}

impl fmt::Display for RelayConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayConnectError::Auth(e) => write!(f, "relay auth: {e}"),
            RelayConnectError::BadRequest => write!(f, "relay connect request was malformed"),
            RelayConnectError::Handshake(m) => write!(f, "relay websocket handshake failed: {m}"),
        }
    }
}

impl std::error::Error for RelayConnectError {}

/// A constellation [`TransportProvider`] backed by Azure Relay Hybrid
/// Connections. It converts each Relay WebSocket rendezvous into a
/// bidirectional [`TransportSession`] and leaves authentication/authorization
/// to the peer-session layer above it.
pub struct AzureRelayTransportProvider {
    id: ProviderId,
    endpoint: RelayEndpoint,
    credential: RelayCredential,
    ttl_secs: u64,
    ca_pem: Option<Vec<u8>>,
    accept_queue: usize,
}

impl AzureRelayTransportProvider {
    /// Build a Relay transport provider with default short-lived SAS TTL and
    /// a bounded accept queue.
    pub fn new(endpoint: RelayEndpoint, credential: RelayCredential) -> Self {
        Self {
            id: ProviderId::parse("azure-relay").expect("valid provider id"),
            endpoint,
            credential,
            ttl_secs: DEFAULT_SAS_TTL_SECS,
            ca_pem: None,
            accept_queue: 16,
        }
    }

    /// Override the TTL used when a SAS rule key must mint a connect token.
    pub fn with_ttl_secs(mut self, ttl_secs: u64) -> Self {
        self.ttl_secs = ttl_secs;
        self
    }

    /// Trust an additional PEM CA bundle (used by sandbox egress proxies).
    pub fn with_ca_pem(mut self, ca_pem: Option<Vec<u8>>) -> Self {
        self.ca_pem = ca_pem;
        self
    }

    /// Override the listener accept queue size. Zero is rounded up to one.
    pub fn with_accept_queue(mut self, accept_queue: usize) -> Self {
        self.accept_queue = accept_queue.max(1);
        self
    }
}

impl fmt::Debug for AzureRelayTransportProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AzureRelayTransportProvider")
            .field("id", &self.id)
            .field("endpoint", &self.endpoint)
            .field("credential", &self.credential)
            .field("ttl_secs", &self.ttl_secs)
            .field("ca_pem", &self.ca_pem.as_ref().map(|_| "<configured>"))
            .field("accept_queue", &self.accept_queue)
            .finish()
    }
}

#[async_trait]
impl TransportProvider for AzureRelayTransportProvider {
    fn transport_id(&self) -> ProviderId {
        self.id.clone()
    }

    async fn connect(&self, _target: TransportTarget) -> ProviderResult<TransportSession> {
        let ws = connect_with_ca(
            &self.endpoint,
            RelayRole::Sender,
            &self.credential,
            self.ttl_secs,
            self.ca_pem.as_deref(),
        )
        .await
        .map_err(relay_provider_error)?;
        Ok(transport_session_from_relay("azure-relay-connect", ws))
    }

    async fn listen(
        &self,
        registration: NodeRegistration,
    ) -> ProviderResult<Box<dyn TransportListener>> {
        let (tx, rx) = mpsc::channel(self.accept_queue);
        let endpoint = self.endpoint.clone();
        let credential = self.credential.clone();
        let ttl_secs = self.ttl_secs;
        let ca_pem = self.ca_pem.clone();
        tokio::spawn(async move {
            relay_transport_listener_task(endpoint, credential, ttl_secs, ca_pem, tx).await;
        });
        Ok(Box::new(AzureRelayTransportListener {
            node: registration.node,
            rx: Mutex::new(rx),
        }))
    }
}

struct AzureRelayTransportListener {
    node: NodeId,
    rx: Mutex<mpsc::Receiver<TransportSession>>,
}

#[async_trait]
impl TransportListener for AzureRelayTransportListener {
    fn node(&self) -> NodeId {
        self.node.clone()
    }

    async fn accept(&self) -> ProviderResult<TransportSession> {
        self.rx
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| ProviderError::new(ErrorKind::RelayUnavailable, "relay listener closed"))
    }
}

async fn relay_transport_listener_task(
    endpoint: RelayEndpoint,
    credential: RelayCredential,
    ttl_secs: u64,
    ca_pem: Option<Vec<u8>>,
    tx: mpsc::Sender<TransportSession>,
) {
    let mut backoff_secs = 1_u64;
    while !tx.is_closed() {
        let connected_at = std::time::Instant::now();
        match relay_transport_accept_loop(
            endpoint.clone(),
            credential.clone(),
            ttl_secs,
            ca_pem.clone(),
            tx.clone(),
        )
        .await
        {
            Ok(()) if tx.is_closed() => break,
            Ok(()) => tracing::warn!("azure relay transport control channel closed; reconnecting"),
            Err(err) => tracing::warn!(error = %err, "azure relay transport listener reconnecting"),
        }
        if connected_at.elapsed() >= std::time::Duration::from_secs(30) {
            backoff_secs = 1;
        }
        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs.saturating_mul(2)).min(30);
    }
}

async fn relay_transport_accept_loop(
    endpoint: RelayEndpoint,
    credential: RelayCredential,
    ttl_secs: u64,
    ca_pem: Option<Vec<u8>>,
    tx: mpsc::Sender<TransportSession>,
) -> Result<(), RelayConnectError> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let control = connect_with_ca(
        &endpoint,
        RelayRole::Listener,
        &credential,
        ttl_secs,
        ca_pem.as_deref(),
    )
    .await?;
    let (mut sink, mut stream) = control.split();
    loop {
        let msg = tokio::select! {
            _ = tx.closed() => return Ok(()),
            msg = stream.next() => match msg {
                Some(msg) => msg,
                None => return Ok(()),
            },
        };
        let msg =
            msg.map_err(|err| RelayConnectError::Handshake(format!("control channel: {err}")))?;
        match msg {
            Message::Text(text) => {
                let v: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(addr) = v
                    .get("accept")
                    .and_then(|a| a.get("address"))
                    .and_then(|s| s.as_str())
                {
                    let address = addr.to_owned();
                    let ca = ca_pem.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let ws = match connect_raw(&address, ca.as_deref()).await {
                            Ok(ws) => ws,
                            Err(err) => {
                                tracing::warn!(error = %err, "azure relay rendezvous dial failed");
                                return;
                            }
                        };
                        match tx.try_send(transport_session_from_relay("azure-relay-accept", ws)) {
                            Ok(()) => {}
                            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                tracing::warn!("azure relay transport accept queue is full");
                            }
                            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {}
                        }
                    });
                }
            }
            Message::Ping(p) => {
                let _ = sink.send(Message::Pong(p)).await;
            }
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    }
}

fn transport_session_from_relay(label: &str, ws: RelayStream) -> TransportSession {
    let (local, relay_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Err(err) = pump(ws, relay_io).await {
            tracing::warn!(error = %err, "relay transport byte pump ended");
        }
    });
    TransportSession::new(SafeLabel::new(label), Box::new(local))
}

fn relay_provider_error(err: RelayConnectError) -> ProviderError {
    let kind = match err {
        RelayConnectError::Auth(_)
        | RelayConnectError::BadRequest
        | RelayConnectError::Handshake(_) => ErrorKind::RelayUnavailable,
    };
    ProviderError::new(kind, err.to_string())
}

/// A local byte endpoint to bridge a relay stream to/from.
#[derive(Debug, Clone)]
pub enum LocalTarget {
    /// Connect to an existing unix socket (`unix:/path`).
    UnixConnect(String),
    /// Connect to an existing unix socket only if the final socket still has
    /// the expected owner/mode at connect time. This is for user-session
    /// sockets where a daemon must not race a validated path into a root-owned
    /// privileged socket.
    UnixConnectChecked {
        /// Socket path.
        path: String,
        /// Required socket owner uid.
        uid: u32,
        /// Required socket mode bits.
        mode: u32,
    },
    /// Bind+listen a unix socket and accept one connection (`unix-listen:/path`).
    /// Lets the local peer (e.g. `waypipe server`) dial in without a socat hop.
    UnixListen(String),
    /// Connect to a TCP `host:port`.
    TcpConnect(String),
}

impl LocalTarget {
    /// Parse the `unix:` / `unix-listen:` / `tcp:` / bare-host:port forms.
    pub fn parse(spec: &str) -> Self {
        if let Some(p) = spec.strip_prefix("unix-listen:") {
            LocalTarget::UnixListen(p.to_owned())
        } else if let Some(p) = spec.strip_prefix("unix:") {
            LocalTarget::UnixConnect(p.to_owned())
        } else if let Some(a) = spec.strip_prefix("tcp:") {
            LocalTarget::TcpConnect(a.to_owned())
        } else {
            LocalTarget::TcpConnect(spec.to_owned())
        }
    }
}

enum LocalIo {
    Tcp(tokio::net::TcpStream),
    Unix(tokio::net::UnixStream),
}

async fn connect_local(target: &LocalTarget) -> std::io::Result<LocalIo> {
    match target {
        LocalTarget::UnixListen(path) => {
            let _ = std::fs::remove_file(path);
            let listener = tokio::net::UnixListener::bind(path)?;
            let (stream, _) = listener.accept().await?;
            Ok(LocalIo::Unix(stream))
        }
        LocalTarget::UnixConnect(path) => {
            Ok(LocalIo::Unix(tokio::net::UnixStream::connect(path).await?))
        }
        LocalTarget::UnixConnectChecked { path, uid, mode } => {
            let socket_fd = open_checked_unix_socket_target(path, *uid, *mode)?;
            let fd_path = format!("/proc/self/fd/{}", socket_fd.as_raw_fd());
            let stream = tokio::net::UnixStream::connect(fd_path).await?;
            validate_connected_unix_peer(&stream, *uid)?;
            Ok(LocalIo::Unix(stream))
        }
        LocalTarget::TcpConnect(addr) => {
            Ok(LocalIo::Tcp(tokio::net::TcpStream::connect(addr).await?))
        }
    }
}

fn open_checked_unix_socket_target(
    path: &str,
    expected_uid: u32,
    expected_mode: u32,
) -> std::io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(Path::new(path))?;
    let metadata = file.metadata()?;
    let file_type = metadata.file_type();
    if !file_type.is_socket() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "relay local unix target is not a direct unix socket",
        ));
    }
    let uid = metadata.uid();
    let mode = metadata.mode() & 0o777;
    if uid != expected_uid || mode != expected_mode {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "relay local unix target owner or mode changed before connect",
        ));
    }
    Ok(file)
}

fn validate_connected_unix_peer(
    stream: &tokio::net::UnixStream,
    expected_uid: u32,
) -> std::io::Result<()> {
    let peer = stream.peer_cred()?;
    if peer.uid() != expected_uid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "relay local unix target peer uid does not match the validated socket owner",
        ));
    }
    Ok(())
}

/// Pump bytes between the relay WebSocket and a local stream until either
/// side closes. Binary frames carry the tunneled bytes; pings are answered;
/// text/close end the pump. This is the productionized form of the POC
/// bridge's byte loop.
async fn pump<T>(ws: RelayStream, io: T) -> Result<(), RelayConnectError>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::Message;

    let (mut sink, mut stream) = ws.split();
    let mut io = io;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        tokio::select! {
            n = io.read(&mut buf) => {
                let n = n.map_err(|_| RelayConnectError::Handshake("local read".into()))?;
                if n == 0 {
                    let _ = sink.send(Message::Close(None)).await;
                    return Ok(());
                }
                sink.send(Message::Binary(buf[..n].to_vec()))
                    .await
                    .map_err(|_| RelayConnectError::Handshake("ws send".into()))?;
            }
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        io.write_all(&data).await
                            .map_err(|_| RelayConnectError::Handshake("local write".into()))?;
                    }
                    Some(Ok(Message::Ping(p))) => { let _ = sink.send(Message::Pong(p)).await; }
                    Some(Ok(Message::Text(_))) | Some(Ok(Message::Pong(_)))
                    | Some(Ok(Message::Frame(_))) => {}
                    Some(Ok(Message::Close(_))) | None => return Ok(()),
                    Some(Err(err)) => {
                        return Err(RelayConnectError::Handshake(format!(
                            "ws stream error: {err}"
                        )));
                    }
                }
            }
        }
    }
}

/// Connect as a sender, retrying briefly on a 404. Azure Relay returns 404
/// to a sender when no listener is registered for the entity yet; the gateway
/// listener may still be completing its control-channel registration, so a
/// few short retries close that startup race without masking a real failure.
async fn connect_sender_retrying(
    endpoint: &RelayEndpoint,
    credential: &RelayCredential,
    ttl_secs: u64,
    ca_pem: Option<&[u8]>,
) -> Result<RelayStream, RelayConnectError> {
    let mut attempt = 0u32;
    loop {
        match connect_with_ca(endpoint, RelayRole::Sender, credential, ttl_secs, ca_pem).await {
            Ok(ws) => return Ok(ws),
            Err(RelayConnectError::Handshake(ref m)) if m.contains("404") && attempt < 30 => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Run the **sender** side (in the sandbox): connect to the relay with the
/// credential (the MI Entra bearer in production), then bridge to `local`.
/// `ca_pem` is the ACA egress-proxy CA.
///
/// For a `unix-listen` target the socket is **bound before** the relay
/// connect, so the local peer (e.g. `waypipe server`) can connect
/// immediately and never races the relay handshake; the local connection is
/// accepted only after the relay side is up. The relay connect retries
/// briefly on a 404 to ride out the gateway listener's registration race.
pub async fn run_sender(
    endpoint: &RelayEndpoint,
    credential: &RelayCredential,
    local: &LocalTarget,
    ttl_secs: u64,
    ca_pem: Option<&[u8]>,
) -> Result<(), RelayConnectError> {
    if let LocalTarget::UnixListen(path) = local {
        let _ = std::fs::remove_file(path);
        let listener = tokio::net::UnixListener::bind(path)
            .map_err(|_| RelayConnectError::Handshake("bind unix-listen".into()))?;
        let ws = connect_sender_retrying(endpoint, credential, ttl_secs, ca_pem).await?;
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|_| RelayConnectError::Handshake("accept unix-listen".into()))?;
        return pump(ws, stream).await;
    }
    let ws = connect_sender_retrying(endpoint, credential, ttl_secs, ca_pem).await?;
    let io = connect_local(local)
        .await
        .map_err(|_| RelayConnectError::Handshake("local connect".into()))?;
    match io {
        LocalIo::Tcp(s) => pump(ws, s).await,
        LocalIo::Unix(s) => pump(ws, s).await,
    }
}

/// Run the **listener** control channel (on the gateway/host): for each
/// sender rendezvous, open the rendezvous stream and bridge it to a fresh
/// `local` connection. Returns when the control channel closes (the caller
/// reconnects). `ca_pem` is `None` on the gateway (public roots).
pub async fn run_listener(
    endpoint: &RelayEndpoint,
    credential: &RelayCredential,
    local: &LocalTarget,
    ttl_secs: u64,
    ca_pem: Option<&[u8]>,
) -> Result<(), RelayConnectError> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let control =
        connect_with_ca(endpoint, RelayRole::Listener, credential, ttl_secs, ca_pem).await?;
    let (mut sink, mut stream) = control.split();
    while let Some(msg) = stream.next().await {
        let msg =
            msg.map_err(|err| RelayConnectError::Handshake(format!("control channel: {err}")))?;
        match msg {
            Message::Text(text) => {
                let v: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(addr) = v
                    .get("accept")
                    .and_then(|a| a.get("address"))
                    .and_then(|s| s.as_str())
                {
                    let address = addr.to_owned();
                    let local = local.clone();
                    let ca = ca_pem.map(|c| c.to_vec());
                    tokio::spawn(async move {
                        if let Err(err) = accept_one(&address, &local, ca.as_deref()).await {
                            tracing::warn!(error = %err, "relay rendezvous ended");
                        }
                    });
                }
            }
            Message::Ping(p) => {
                let _ = sink.send(Message::Pong(p)).await;
            }
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    }
    Ok(())
}

async fn accept_one(
    address: &str,
    local: &LocalTarget,
    ca_pem: Option<&[u8]>,
) -> Result<(), RelayConnectError> {
    let ws = connect_raw(address, ca_pem).await?;
    let io = connect_local(local)
        .await
        .map_err(|_| RelayConnectError::Handshake("local connect".into()))?;
    match io {
        LocalIo::Tcp(s) => pump(ws, s).await,
        LocalIo::Unix(s) => pump(ws, s).await,
    }
}

/// A prologue verifier: given the first length-delimited frame's body, decide
/// whether to admit the connection. The relay treats the frame as **opaque
/// bytes** (it never depends on the gateway); the gateway supplies a closure
/// that runs its session-handshake verification.
pub type PrologueVerifier = std::sync::Arc<dyn Fn(&[u8]) -> bool + Send + Sync>;

/// Max prologue frame body the listener will buffer before rejecting.
const MAX_PROLOGUE: usize = 16 * 1024;

/// Try to extract one length-delimited frame (`u32-be length || body`) from the
/// front of `buf`. Returns `Ok(Some((body, consumed)))` once a full frame is
/// present, `Ok(None)` if more bytes are needed, and `Err` if the declared
/// length exceeds [`MAX_PROLOGUE`]. Pure + unit-testable.
fn extract_prologue_frame(buf: &[u8]) -> Result<Option<(Vec<u8>, usize)>, RelayConnectError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_be_bytes(buf[..4].try_into().expect("4 bytes")) as usize;
    if len > MAX_PROLOGUE {
        return Err(RelayConnectError::Handshake("prologue too large".into()));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    Ok(Some((buf[4..4 + len].to_vec(), 4 + len)))
}

/// Read the prologue frame off the relay WebSocket, returning `(frame_body,
/// leftover_bytes)`. Leftover bytes belong to the bridged stream and must be
/// written to the local socket before pumping.
async fn read_prologue(ws: &mut RelayStream) -> Result<(Vec<u8>, Vec<u8>), RelayConnectError> {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        if let Some((frame, consumed)) = extract_prologue_frame(&buf)? {
            let leftover = buf[consumed..].to_vec();
            return Ok((frame, leftover));
        }
        match ws.next().await {
            Some(Ok(Message::Binary(data))) => buf.extend_from_slice(&data),
            Some(Ok(Message::Ping(_)))
            | Some(Ok(Message::Pong(_)))
            | Some(Ok(Message::Text(_)))
            | Some(Ok(Message::Frame(_))) => {}
            Some(Ok(Message::Close(_))) | None => {
                return Err(RelayConnectError::Handshake("prologue eof".into()));
            }
            Some(Err(_)) => return Err(RelayConnectError::Handshake("prologue read".into())),
        }
    }
}

/// Like [`run_listener`], but each accepted rendezvous must present a prologue
/// frame that `verify` admits **before any byte is bridged** to `local`. A
/// rejected or missing prologue closes the rendezvous with no bytes forwarded.
/// This is how the gateway makes its per-session credential gate the display
/// byte stream over the relay.
pub async fn run_listener_verified(
    endpoint: &RelayEndpoint,
    credential: &RelayCredential,
    local: &LocalTarget,
    ttl_secs: u64,
    ca_pem: Option<&[u8]>,
    verify: PrologueVerifier,
) -> Result<(), RelayConnectError> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let control =
        connect_with_ca(endpoint, RelayRole::Listener, credential, ttl_secs, ca_pem).await?;
    let (mut sink, mut stream) = control.split();
    while let Some(msg) = stream.next().await {
        let msg =
            msg.map_err(|err| RelayConnectError::Handshake(format!("control channel: {err}")))?;
        match msg {
            Message::Text(text) => {
                let v: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(addr) = v
                    .get("accept")
                    .and_then(|a| a.get("address"))
                    .and_then(|s| s.as_str())
                {
                    let address = addr.to_owned();
                    let local = local.clone();
                    let ca = ca_pem.map(|c| c.to_vec());
                    let verify = verify.clone();
                    tokio::spawn(async move {
                        if let Err(err) =
                            accept_one_verified(&address, &local, ca.as_deref(), verify).await
                        {
                            tracing::warn!(error = %err, "verified relay rendezvous ended");
                        }
                    });
                }
            }
            Message::Ping(p) => {
                let _ = sink.send(Message::Pong(p)).await;
            }
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    }
    Ok(())
}

async fn accept_one_verified(
    address: &str,
    local: &LocalTarget,
    ca_pem: Option<&[u8]>,
    verify: PrologueVerifier,
) -> Result<(), RelayConnectError> {
    use tokio::io::AsyncWriteExt;
    let mut ws = connect_raw(address, ca_pem).await?;
    let (frame, leftover) = read_prologue(&mut ws).await?;
    if !verify(&frame) {
        // Fail closed: never connect the local socket, never forward a byte.
        return Err(RelayConnectError::Handshake("prologue rejected".into()));
    }
    let io = connect_local(local)
        .await
        .map_err(|_| RelayConnectError::Handshake("local connect".into()))?;
    match io {
        LocalIo::Tcp(mut s) => {
            if !leftover.is_empty() {
                s.write_all(&leftover)
                    .await
                    .map_err(|_| RelayConnectError::Handshake("local write".into()))?;
            }
            pump(ws, s).await
        }
        LocalIo::Unix(mut s) => {
            if !leftover.is_empty() {
                s.write_all(&leftover)
                    .await
                    .map_err(|_| RelayConnectError::Handshake("local write".into()))?;
            }
            pump(ws, s).await
        }
    }
}

/// Like [`run_sender`], but writes `prologue` as the first bytes on the relay
/// channel before bridging the local stream. The in-sandbox agent uses this to
/// present its session handshake frame, which the gateway's
/// [`run_listener_verified`] consumes and verifies before bridging.
pub async fn run_sender_with_prologue(
    endpoint: &RelayEndpoint,
    credential: &RelayCredential,
    local: &LocalTarget,
    ttl_secs: u64,
    ca_pem: Option<&[u8]>,
    prologue: &[u8],
) -> Result<(), RelayConnectError> {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let frame = Message::Binary(prologue.to_vec());
    if let LocalTarget::UnixListen(path) = local {
        let _ = std::fs::remove_file(path);
        let listener = tokio::net::UnixListener::bind(path)
            .map_err(|_| RelayConnectError::Handshake("bind unix-listen".into()))?;
        let mut ws = connect_sender_retrying(endpoint, credential, ttl_secs, ca_pem).await?;
        ws.send(frame)
            .await
            .map_err(|_| RelayConnectError::Handshake("prologue send".into()))?;
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|_| RelayConnectError::Handshake("accept unix-listen".into()))?;
        return pump(ws, stream).await;
    }
    let mut ws = connect_sender_retrying(endpoint, credential, ttl_secs, ca_pem).await?;
    ws.send(frame)
        .await
        .map_err(|_| RelayConnectError::Handshake("prologue send".into()))?;
    let io = connect_local(local)
        .await
        .map_err(|_| RelayConnectError::Handshake("local connect".into()))?;
    match io {
        LocalIo::Tcp(s) => pump(ws, s).await,
        LocalIo::Unix(s) => pump(ws, s).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn extract_prologue_needs_full_length_prefix() {
        // Fewer than 4 bytes -> need more.
        assert_eq!(extract_prologue_frame(&[0, 0]).unwrap(), None);
    }

    #[test]
    fn extract_prologue_waits_for_full_body() {
        // length=5 but only 3 body bytes present -> need more.
        let mut buf = (5u32).to_be_bytes().to_vec();
        buf.extend_from_slice(b"abc");
        assert_eq!(extract_prologue_frame(&buf).unwrap(), None);
    }

    #[test]
    fn extract_prologue_returns_frame_and_consumed() {
        let mut buf = (5u32).to_be_bytes().to_vec();
        buf.extend_from_slice(b"hello");
        buf.extend_from_slice(b"LEFTOVER");
        let (frame, consumed) = extract_prologue_frame(&buf).unwrap().unwrap();
        assert_eq!(frame, b"hello");
        assert_eq!(consumed, 9); // 4 + 5
        assert_eq!(&buf[consumed..], b"LEFTOVER");
    }

    #[test]
    fn extract_prologue_rejects_oversize() {
        let buf = (u32::MAX).to_be_bytes().to_vec();
        assert!(extract_prologue_frame(&buf).is_err());
    }

    fn endpoint() -> RelayEndpoint {
        RelayEndpoint {
            namespace: "relns-test.servicebus.windows.net".into(),
            entity: "hc-d2b-display".into(),
        }
    }

    fn now_unix_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn sas_param<'a>(token: &'a str, name: &str) -> &'a str {
        let prefix = format!("{name}=");
        token
            .strip_prefix("SharedAccessSignature ")
            .unwrap()
            .split('&')
            .find_map(|part| part.strip_prefix(&prefix))
            .unwrap()
    }

    #[test]
    fn mint_sas_is_deterministic_for_fixed_inputs_modulo_expiry() {
        let ep = endpoint();
        let a = mint_sas(&ep, "gateway-listen", "c2VjcmV0a2V5", DEFAULT_SAS_TTL_SECS).unwrap();
        // Shape: a SharedAccessSignature with sr/sig/se/skn.
        assert!(a.starts_with("SharedAccessSignature sr="));
        assert!(a.contains("&skn=gateway-listen"));
        assert!(a.contains("&sig="));
        assert!(a.contains("&se="));
        // The resource is the lowercased url-encoded http form of the entity.
        assert!(a.contains("relns-test.servicebus.windows.net"));
    }

    #[test]
    fn mint_sas_rejects_ttl_above_short_lived_cap() {
        let ep = endpoint();
        assert_eq!(
            mint_sas(&ep, "gateway-send", "c2VjcmV0a2V5", MAX_SAS_TTL_SECS + 1).unwrap_err(),
            RelayError::TtlTooLong {
                requested: MAX_SAS_TTL_SECS + 1,
                max: MAX_SAS_TTL_SECS
            }
        );
    }

    #[test]
    fn mint_sas_expiry_matches_requested_short_ttl() {
        let ep = endpoint();
        let ttl = 60;
        let before = now_unix_secs();
        let token = mint_sas(&ep, "gateway-send", "c2VjcmV0a2V5", ttl).unwrap();
        let after = now_unix_secs();
        let expiry = sas_param(&token, "se").parse::<u64>().unwrap();
        assert!(expiry >= before + ttl);
        assert!(expiry <= after + ttl);
        assert!(expiry <= before + MAX_SAS_TTL_SECS);
    }

    #[test]
    fn entra_sender_uses_header_not_url_token() {
        let ep = endpoint();
        let cred = RelayCredential::EntraBearer("jwt.abc.def".into());
        let c = build_connect(&ep, RelayRole::Sender, &cred, 3600).unwrap();
        // The bearer NEVER appears in the URL.
        assert!(!c.url.contains("jwt.abc.def"));
        assert!(!c.url.contains("sb-hc-token"));
        assert!(c.url.contains("sb-hc-action=connect"));
        // The sender omits sb-hc-id; the relay generates the rendezvous GUID.
        assert!(!c.url.contains("sb-hc-id="));
        assert_eq!(c.auth_header.as_deref(), Some("Bearer jwt.abc.def"));
    }

    #[test]
    fn sas_listener_puts_token_in_url_and_no_header() {
        let ep = endpoint();
        let cred = RelayCredential::Sas {
            key_name: "gateway-listen".into(),
            key: "c2VjcmV0a2V5".into(),
        };
        let c = build_connect(&ep, RelayRole::Listener, &cred, DEFAULT_SAS_TTL_SECS).unwrap();
        assert!(c.url.contains("sb-hc-action=listen"));
        assert!(c.url.contains("sb-hc-token="));
        assert!(!c.url.contains("sb-hc-id=")); // listener has no rendezvous id
        assert!(c.auth_header.is_none());
    }

    #[test]
    fn build_connect_rejects_sas_ttl_above_short_lived_cap() {
        let ep = endpoint();
        let cred = RelayCredential::Sas {
            key_name: "gateway-listen".into(),
            key: "c2VjcmV0a2V5".into(),
        };
        assert!(matches!(
            build_connect(&ep, RelayRole::Listener, &cred, MAX_SAS_TTL_SECS + 1),
            Err(RelayError::TtlTooLong { .. })
        ));
    }

    #[test]
    fn credential_debug_redacts_secrets() {
        let sas = RelayCredential::Sas {
            key_name: "gateway-send".into(),
            key: "supersecretkey".into(),
        };
        let d = format!("{sas:?}");
        assert!(d.contains("gateway-send"));
        assert!(!d.contains("supersecretkey"));
        let bearer = RelayCredential::EntraBearer("jwt.secret.token".into());
        let d = format!("{bearer:?}");
        assert!(!d.contains("jwt.secret.token"));
        let token = RelayCredential::SasToken("SharedAccessSignature secret".into());
        let d = format!("{token:?}");
        assert!(!d.contains("SharedAccessSignature secret"));
    }

    #[test]
    fn pre_minted_sas_sender_puts_token_in_url_without_key() {
        let ep = endpoint();
        let cred = RelayCredential::SasToken("SharedAccessSignature sr=x&sig=y".into());
        let c = build_connect(&ep, RelayRole::Sender, &cred, 3600).unwrap();
        assert!(c.url.contains("sb-hc-action=connect"));
        assert!(c.url.contains("sb-hc-token="));
        assert!(!c.url.contains("sb-hc-id="));
        assert!(c.auth_header.is_none());
    }

    #[test]
    fn connect_debug_redacts_preminted_sas_query_token() {
        let ep = endpoint();
        let secret_token =
            "SharedAccessSignature sr=x&sig=very-secret-signature&se=123&skn=gateway-send";
        let cred = RelayCredential::SasToken(secret_token.into());
        let c = build_connect(&ep, RelayRole::Sender, &cred, 3600).unwrap();
        let d = format!("{c:?}");
        assert!(!d.contains("SharedAccessSignature"));
        assert!(!d.contains("very-secret-signature"));
        assert!(d.contains("?<redacted>"));
    }

    #[test]
    fn connect_debug_redacts_url_query_and_header() {
        let ep = endpoint();
        let cred = RelayCredential::EntraBearer("jwt.abc.def".into());
        let c = build_connect(&ep, RelayRole::Sender, &cred, 3600).unwrap();
        let d = format!("{c:?}");
        assert!(!d.contains("jwt.abc.def"));
        assert!(!d.contains("Bearer"));
        assert!(d.contains("<redacted>"));
    }

    #[test]
    fn azure_relay_transport_debug_redacts_credentials() {
        let provider = AzureRelayTransportProvider::new(
            endpoint(),
            RelayCredential::SasToken("SharedAccessSignature sr=x&sig=secret".into()),
        )
        .with_accept_queue(0)
        .with_ca_pem(Some(b"ca".to_vec()));
        assert_eq!(provider.transport_id().as_str(), "azure-relay");
        let rendered = format!("{provider:?}");
        assert!(rendered.contains("azure-relay"));
        assert!(!rendered.contains("SharedAccessSignature"));
        assert!(!rendered.contains("secret"));
        assert!(rendered.contains("<configured>"));
    }

    #[tokio::test]
    async fn azure_relay_transport_maps_auth_failures_to_typed_error() {
        let provider = AzureRelayTransportProvider::new(
            endpoint(),
            RelayCredential::Sas {
                key_name: "gateway-send".into(),
                key: "c2VjcmV0a2V5".into(),
            },
        )
        .with_ttl_secs(MAX_SAS_TTL_SECS + 1);
        let err = provider
            .connect(TransportTarget {
                endpoint: "ignored".to_owned(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::RelayUnavailable);
    }

    #[test]
    fn relay_provider_bad_request_is_transport_unavailable() {
        let err = relay_provider_error(RelayConnectError::BadRequest);
        assert_eq!(err.kind(), ErrorKind::RelayUnavailable);
    }

    #[test]
    fn local_target_parses_each_form() {
        assert!(matches!(
            LocalTarget::parse("unix-listen:/run/wp.sock"),
            LocalTarget::UnixListen(p) if p == "/run/wp.sock"
        ));
        assert!(matches!(
            LocalTarget::parse("unix:/run/wpc.sock"),
            LocalTarget::UnixConnect(p) if p == "/run/wpc.sock"
        ));
        assert!(matches!(
            LocalTarget::parse("tcp:127.0.0.1:8080"),
            LocalTarget::TcpConnect(a) if a == "127.0.0.1:8080"
        ));
        assert!(matches!(
            LocalTarget::parse("127.0.0.1:9000"),
            LocalTarget::TcpConnect(a) if a == "127.0.0.1:9000"
        ));
    }

    #[test]
    fn checked_unix_target_revalidates_owner_and_mode() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("wpc.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let meta = std::fs::symlink_metadata(&socket_path).unwrap();
        let path = socket_path.to_string_lossy().into_owned();
        open_checked_unix_socket_target(&path, meta.uid(), 0o600).unwrap();
        open_checked_unix_socket_target(&path, meta.uid(), 0o660).unwrap_err();

        let link_path = dir.path().join("link.sock");
        std::os::unix::fs::symlink(&socket_path, &link_path).unwrap();
        open_checked_unix_socket_target(&link_path.to_string_lossy(), meta.uid(), 0o600)
            .unwrap_err();
    }

    #[tokio::test]
    async fn checked_unix_target_validates_connected_peer_uid() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("wpc.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let meta = std::fs::symlink_metadata(&socket_path).unwrap();
        let stream = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
        let (_accepted, _) = listener.accept().await.unwrap();
        validate_connected_unix_peer(&stream, meta.uid()).unwrap();
        validate_connected_unix_peer(&stream, meta.uid().saturating_add(1)).unwrap_err();
    }
}
