//! Gateway-owned Azure Relay WebSocket and local-socket bridge.
//!
//! The canonical Provider crate owns Relay authentication and typed transport
//! contracts. This composition-root module owns the host-side socket effects
//! needed by gateway binaries and display listeners.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;

use d2b_provider_transport_azure_relay::auth::{
    RelayCredential, RelayEndpoint, RelayError, RelayRole, build_connect,
};
use rustls_pki_types::{CertificateDer, pem::PemObject};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use tokio::time::{Duration, timeout};

pub type RelayStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

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
/// the `ServiceBusAuthorization` header - never in the URL.
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
    let (ws, _resp) = timeout(
        RELAY_CONNECT_TIMEOUT,
        tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector)),
    )
    .await
    .map_err(|_| RelayConnectError::Handshake("relay connect timeout".into()))?
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
    /// The authenticated bridge ended. The session prologue has already been
    /// consumed, so retrying it would be a replay and must not be attempted.
    Bridge(String),
}

impl fmt::Display for RelayConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayConnectError::Auth(e) => write!(f, "relay auth: {e}"),
            RelayConnectError::BadRequest => write!(f, "relay connect request was malformed"),
            RelayConnectError::Handshake(m) => write!(f, "relay websocket handshake failed: {m}"),
            RelayConnectError::Bridge(m) => write!(f, "relay bridge ended: {m}"),
        }
    }
}

impl std::error::Error for RelayConnectError {}

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

#[derive(Default)]
struct RendezvousTasks {
    tasks: JoinSet<()>,
}

impl RendezvousTasks {
    fn spawn<F>(&mut self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tasks.spawn(task);
    }

    fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.tasks.len()
    }

    async fn reap_one(&mut self) {
        if let Some(Err(err)) = self.tasks.join_next().await
            && !err.is_cancelled()
        {
            tracing::warn!(error = %err, "relay rendezvous task failed");
        }
    }

    async fn cancel_and_join(&mut self) {
        self.tasks.abort_all();
        while let Some(result) = self.tasks.join_next().await {
            if let Err(err) = result
                && !err.is_cancelled()
            {
                tracing::warn!(error = %err, "relay rendezvous task failed while stopping");
            }
        }
    }
}

async fn run_listener_control_loop<S, K, F, Fut>(
    stream: S,
    mut sink: K,
    mut cancellation: Option<watch::Receiver<bool>>,
    mut spawn_rendezvous: F,
) -> Result<(), RelayConnectError>
where
    S: futures_util::Stream<
            Item = Result<tokio_tungstenite::tungstenite::Message, RelayConnectError>,
        >,
    K: futures_util::Sink<tokio_tungstenite::tungstenite::Message> + Unpin,
    F: FnMut(String) -> Option<Fut>,
    Fut: Future<Output = ()> + Send + 'static,
{
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let mut stream = Box::pin(stream);
    let mut rendezvous_tasks = RendezvousTasks::default();
    let result: Result<(), RelayConnectError> = 'control: loop {
        tokio::select! {
            biased;
            _ = wait_for_listener_cancellation(&mut cancellation) => {
                break 'control Ok(());
            }
            _ = rendezvous_tasks.reap_one(), if !rendezvous_tasks.is_empty() => {
            }
            msg = stream.next() => {
                let Some(msg) = msg else {
                    break 'control Ok(());
                };
                let msg = match msg {
                    Ok(msg) => msg,
                    Err(err) => break 'control Err(err),
                };
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
                            && let Some(task) = spawn_rendezvous(addr.to_owned())
                        {
                            rendezvous_tasks.spawn(task);
                        }
                    }
                    Message::Ping(p) => {
                        tokio::select! {
                            _ = wait_for_listener_cancellation(&mut cancellation) => {
                                break 'control Ok(());
                            }
                            _ = sink.send(Message::Pong(p)) => {}
                        }
                    }
                    Message::Close(_) => break 'control Ok(()),
                    _ => {}
                }
            }
        }
    };
    rendezvous_tasks.cancel_and_join().await;
    result
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
    let mut available_credit = BRIDGE_CREDIT_BYTES;
    let mut in_flight_credit = 0usize;
    loop {
        let read_len = available_credit.min(buf.len());
        tokio::select! {
            n = io.read(&mut buf[..read_len]), if available_credit > 0 => {
                let n = n.map_err(|_| RelayConnectError::Handshake("local read".into()))?;
                if n == 0 {
                    let _ = sink.send(Message::Close(None)).await;
                    return Ok(());
                }
                available_credit -= n;
                in_flight_credit += n;
                sink.send(Message::Binary(buf[..n].to_vec()))
                    .await
                    .map_err(|_| RelayConnectError::Handshake("ws send".into()))?;
                sink.send(Message::Ping((n as u64).to_be_bytes().to_vec()))
                    .await
                    .map_err(|_| RelayConnectError::Handshake("ws credit".into()))?;
            }
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        io.write_all(&data).await
                            .map_err(|_| RelayConnectError::Handshake("local write".into()))?;
                    }
                    Some(Ok(Message::Ping(p))) => { let _ = sink.send(Message::Pong(p)).await; }
                    Some(Ok(Message::Pong(p))) => {
                        if p.len() == 8 {
                            let released = u64::from_be_bytes(
                                p.as_slice().try_into().expect("8-byte credit acknowledgement"),
                            ) as usize;
                            acknowledge_bridge_credit(
                                &mut available_credit,
                                &mut in_flight_credit,
                                released,
                            );
                        }

                    }
                    Some(Ok(Message::Text(_))) | Some(Ok(Message::Frame(_))) => {}
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

fn acknowledge_bridge_credit(
    available_credit: &mut usize,
    in_flight_credit: &mut usize,
    acknowledged: usize,
) {
    let released = acknowledged.min(*in_flight_credit);
    *in_flight_credit -= released;
    *available_credit = available_credit
        .saturating_add(released)
        .min(BRIDGE_CREDIT_BYTES);
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
    use futures_util::StreamExt;

    let control =
        connect_with_ca(endpoint, RelayRole::Listener, credential, ttl_secs, ca_pem).await?;
    let (sink, stream) = control.split();
    let stream = stream.map(|msg| {
        msg.map_err(|err| RelayConnectError::Handshake(format!("control channel: {err}")))
    });
    run_listener_control_loop(stream, sink, None, |address| {
        let local = local.clone();
        let ca = ca_pem.map(|c| c.to_vec());
        Some(async move {
            if let Err(err) = accept_one(&address, &local, ca.as_deref()).await {
                tracing::warn!(error = %err, "relay rendezvous ended");
            }
        })
    })
    .await
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
const MAX_PENDING_RENDEZVOUS: usize = 128;
const PROLOGUE_TIMEOUT: Duration = Duration::from_secs(15);
const BRIDGE_CREDIT_BYTES: usize = 256 * 1024;

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
    run_listener_verified_with_ready(
        endpoint,
        credential,
        local,
        ttl_secs,
        ca_pem,
        verify,
        std::sync::Arc::new(|| {}),
    )
    .await
}

/// Verified listener variant that signals readiness only after the accepted
/// rendezvous has passed authentication and the local bridge endpoint has
/// attached successfully.
pub async fn run_listener_verified_with_ready(
    endpoint: &RelayEndpoint,
    credential: &RelayCredential,
    local: &LocalTarget,
    ttl_secs: u64,
    ca_pem: Option<&[u8]>,
    verify: PrologueVerifier,
    ready: std::sync::Arc<dyn Fn() + Send + Sync>,
) -> Result<(), RelayConnectError> {
    run_listener_verified_with_ready_and_cancel(
        endpoint, credential, local, ttl_secs, ca_pem, verify, ready, None,
    )
    .await
}

pub(crate) async fn run_listener_verified_with_ready_and_cancel(
    endpoint: &RelayEndpoint,
    credential: &RelayCredential,
    local: &LocalTarget,
    ttl_secs: u64,
    ca_pem: Option<&[u8]>,
    verify: PrologueVerifier,
    ready: std::sync::Arc<dyn Fn() + Send + Sync>,
    mut cancellation: Option<watch::Receiver<bool>>,
) -> Result<(), RelayConnectError> {
    use futures_util::StreamExt;

    if cancellation
        .as_ref()
        .is_some_and(|receiver| *receiver.borrow())
    {
        return Ok(());
    }
    let control = tokio::select! {
        result = connect_with_ca(endpoint, RelayRole::Listener, credential, ttl_secs, ca_pem) => {
            result?
        }
        _ = wait_for_listener_cancellation(&mut cancellation) => {
            return Ok(());
        }
    };
    let (sink, stream) = control.split();
    let rendezvous_slots = std::sync::Arc::new(Semaphore::new(MAX_PENDING_RENDEZVOUS));
    let stream = stream.map(|msg| {
        msg.map_err(|err| RelayConnectError::Handshake(format!("control channel: {err}")))
    });
    run_listener_control_loop(stream, sink, cancellation, |address| {
        let Ok(slot) = rendezvous_slots.clone().try_acquire_owned() else {
            tracing::warn!("relay rendezvous concurrency bound reached");
            return None;
        };
        let local = local.clone();
        let ca = ca_pem.map(|c| c.to_vec());
        let verify = verify.clone();
        let ready = ready.clone();
        Some(async move {
            let _slot = slot;
            if let Err(err) =
                accept_one_verified(&address, &local, ca.as_deref(), verify, ready).await
            {
                tracing::warn!(error = %err, "verified relay rendezvous ended");
            }
        })
    })
    .await
}

async fn wait_for_listener_cancellation(cancellation: &mut Option<watch::Receiver<bool>>) {
    loop {
        match cancellation {
            Some(receiver) => {
                if receiver.changed().await.is_err() || *receiver.borrow() {
                    return;
                }
            }
            None => std::future::pending::<()>().await,
        }
    }
}

async fn accept_one_verified(
    address: &str,
    local: &LocalTarget,
    ca_pem: Option<&[u8]>,
    verify: PrologueVerifier,
    ready: std::sync::Arc<dyn Fn() + Send + Sync>,
) -> Result<(), RelayConnectError> {
    use tokio::io::AsyncWriteExt;
    let mut ws = connect_raw(address, ca_pem).await?;
    let (frame, leftover) = timeout(PROLOGUE_TIMEOUT, read_prologue(&mut ws))
        .await
        .map_err(|_| RelayConnectError::Handshake("prologue timeout".into()))??;
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
            ready();
            pump(ws, s).await
        }
        LocalIo::Unix(mut s) => {
            if !leftover.is_empty() {
                s.write_all(&leftover)
                    .await
                    .map_err(|_| RelayConnectError::Handshake("local write".into()))?;
            }
            ready();
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
            .map_err(|_| RelayConnectError::Bridge("accept unix-listen".into()))?;
        return pump(ws, stream)
            .await
            .map_err(|error| RelayConnectError::Bridge(error.to_string()));
    }
    let mut ws = connect_sender_retrying(endpoint, credential, ttl_secs, ca_pem).await?;
    ws.send(frame)
        .await
        .map_err(|_| RelayConnectError::Handshake("prologue send".into()))?;
    let io = connect_local(local)
        .await
        .map_err(|_| RelayConnectError::Bridge("local connect".into()))?;
    match io {
        LocalIo::Tcp(s) => pump(ws, s)
            .await
            .map_err(|error| RelayConnectError::Bridge(error.to_string())),
        LocalIo::Unix(s) => pump(ws, s)
            .await
            .map_err(|error| RelayConnectError::Bridge(error.to_string())),
    }
}

#[cfg(test)]
mod tests;
