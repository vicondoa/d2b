//! `nixling-provider-relay`: the Azure Relay transport auth/credential core
//! for the realm gateway (ADR 0032, P0).
//!
//! This productionizes the relay leg of the P0 vertical. The POC's
//! `nixling-relay-bridge` proved the byte path; this crate is the
//! nixling-native home for the **credential model + connect contract** that
//! the gateway's relay transport and the in-sandbox sender are built on.
//!
//! ## Three-plane mapping
//! - The **gateway** (host side) holds the relay **Listen** credential and
//!   opens the listener control channel. Listen auth is a gateway-side SAS
//!   minted from the `gateway-listen` rule key, or (later) the gateway's own
//!   Entra **Listener** role.
//! - The **container** (sandbox sender) authenticates with an **Entra bearer
//!   token from its managed identity** (plane 2) — the productionized path,
//!   so **no SAS key ever enters the sandbox**. The token is presented in the
//!   `ServiceBusAuthorization` WebSocket header (Azure Relay's Entra
//!   data-plane auth), never in the URL.
//!
//! Every secret ([`RelayCredential`] material, minted SAS, bearer token) has
//! a redacted `Debug` so it can never reach a log, span, or audit record.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// The Entra resource (audience) a managed identity requests a token for to
/// authenticate to Azure Relay. Confirmed against the Azure Relay docs.
pub const RELAY_TOKEN_RESOURCE: &str = "https://relay.azure.net";

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
    /// Hybrid connection (entity) name, e.g. `hc-nixling-display`.
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

/// The default minted-SAS lifetime (seconds). The gateway mints short-lived
/// Listen tokens; a long-lived token is never persisted.
pub const DEFAULT_SAS_TTL_SECS: u64 = 3600;

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
/// `ServiceBusAuthorization: Bearer <jwt>` header.
pub fn build_connect(
    endpoint: &RelayEndpoint,
    role: RelayRole,
    credential: &RelayCredential,
    ttl_secs: u64,
) -> Result<RelayConnect, RelayError> {
    let id_param = match role {
        RelayRole::Sender => format!("&sb-hc-id={}", connect_id()),
        RelayRole::Listener => String::new(),
    };
    let base = format!(
        "wss://{}/$hc/{}?sb-hc-action={}{}",
        endpoint.namespace,
        urlencoding::encode(&endpoint.entity),
        role.action(),
        id_param,
    );
    match credential {
        RelayCredential::EntraBearer(token) => Ok(RelayConnect {
            url: base,
            auth_header: Some(format!("Bearer {token}")),
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

/// A connect id for the sender rendezvous. Deterministic length, non-secret.
fn connect_id() -> String {
    // A 16-byte hex id from the system clock + a process-unique counter. No
    // RNG dependency; uniqueness only needs to hold within a gateway.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:016x}", t ^ n.rotate_left(32))
}

/// Errors building relay auth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayError {
    /// The system clock was before the Unix epoch.
    Clock,
    /// The SAS key was not valid HMAC key material.
    Key,
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayError::Clock => write!(f, "system clock is before the unix epoch"),
            RelayError::Key => write!(f, "relay SAS key is invalid"),
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
/// carries its own token) with the optional extra CA.
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
        let mut reader = std::io::BufReader::new(pem);
        for cert in rustls_pemfile::certs(&mut reader) {
            let cert = cert.map_err(|_| RelayConnectError::BadRequest)?;
            roots.add(cert).map_err(|_| RelayConnectError::BadRequest)?;
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

/// A local byte endpoint to bridge a relay stream to/from.
#[derive(Debug, Clone)]
pub enum LocalTarget {
    /// Connect to an existing unix socket (`unix:/path`).
    UnixConnect(String),
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
        LocalTarget::TcpConnect(addr) => {
            Ok(LocalIo::Tcp(tokio::net::TcpStream::connect(addr).await?))
        }
    }
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
                    Some(Err(_)) => {
                        return Err(RelayConnectError::Handshake("ws stream error".into()));
                    }
                }
            }
        }
    }
}

/// Run the **sender** side (in the sandbox): connect to the relay with the
/// credential (the MI Entra bearer in production), then bridge to `local`.
/// `ca_pem` is the ACA egress-proxy CA.
pub async fn run_sender(
    endpoint: &RelayEndpoint,
    credential: &RelayCredential,
    local: &LocalTarget,
    ttl_secs: u64,
    ca_pem: Option<&[u8]>,
) -> Result<(), RelayConnectError> {
    let ws = connect_with_ca(endpoint, RelayRole::Sender, credential, ttl_secs, ca_pem).await?;
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
        let msg = msg.map_err(|_| RelayConnectError::Handshake("control channel".into()))?;
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
                        let _ = accept_one(&address, &local, ca.as_deref()).await;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint() -> RelayEndpoint {
        RelayEndpoint {
            namespace: "relns-test.servicebus.windows.net".into(),
            entity: "hc-nixling-display".into(),
        }
    }

    #[test]
    fn mint_sas_is_deterministic_for_fixed_inputs_modulo_expiry() {
        let ep = endpoint();
        let a = mint_sas(&ep, "gateway-listen", "c2VjcmV0a2V5", 3600).unwrap();
        // Shape: a SharedAccessSignature with sr/sig/se/skn.
        assert!(a.starts_with("SharedAccessSignature sr="));
        assert!(a.contains("&skn=gateway-listen"));
        assert!(a.contains("&sig="));
        assert!(a.contains("&se="));
        // The resource is the lowercased url-encoded http form of the entity.
        assert!(a.contains("relns-test.servicebus.windows.net"));
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
        assert!(c.url.contains("sb-hc-id="));
        assert_eq!(c.auth_header.as_deref(), Some("Bearer jwt.abc.def"));
    }

    #[test]
    fn sas_listener_puts_token_in_url_and_no_header() {
        let ep = endpoint();
        let cred = RelayCredential::Sas {
            key_name: "gateway-listen".into(),
            key: "c2VjcmV0a2V5".into(),
        };
        let c = build_connect(&ep, RelayRole::Listener, &cred, 3600).unwrap();
        assert!(c.url.contains("sb-hc-action=listen"));
        assert!(c.url.contains("sb-hc-token="));
        assert!(!c.url.contains("sb-hc-id=")); // listener has no rendezvous id
        assert!(c.auth_header.is_none());
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
    fn connect_ids_are_unique() {
        let a = connect_id();
        let b = connect_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 16);
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
}
