//! nixling-relay-bridge — tunnel a raw byte stream over an Azure Relay
//! Hybrid Connection (ADR 0032, Wave P0).
//!
//! Two modes:
//!
//! - `send`   (runs in the ACA sandbox): connects to the relay as a
//!   *sender*, then connects to a local TCP target (the in-sandbox
//!   `waypipe server` reached via the agent's socat on 127.0.0.1:8080) and
//!   pumps bytes both ways.
//! - `listen` (runs on the operator host): connects to the relay as a
//!   *listener* control channel; for each accepted sender connection it
//!   opens the rendezvous WebSocket and connects to a local target (the
//!   host `waypipe client` unix socket) and pumps bytes both ways.
//!
//! No inbound ports are opened on either side: both ends dial the relay
//! outbound. The SAS token is passed in at run time (the gateway mints a
//! short-lived Send token); it is never baked into the sandbox image.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UnixListener, UnixStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::Connector;

type HmacSha256 = Hmac<Sha256>;

/// Build a rustls TLS connector that trusts the compiled-in Mozilla
/// webpki roots, plus any extra CA certs from `ca_file`. The extra CA is
/// required inside ACA sandboxes whose transparent egress proxy
/// terminates TLS with the injected `adc-egress-proxy-ca`.
fn build_connector(ca_file: Option<&str>) -> Result<Connector> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(path) = ca_file {
        let pem = std::fs::read(path).with_context(|| format!("read ca-file {path}"))?;
        let mut reader = std::io::BufReader::new(&pem[..]);
        let mut added = 0usize;
        for cert in rustls_pemfile::certs(&mut reader) {
            let cert = cert.context("parse ca-file cert")?;
            roots.add(cert).context("add ca-file cert")?;
            added += 1;
        }
        eprintln!("[relay-bridge] trusting {added} extra CA cert(s) from {path}");
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}

/// Connect a WebSocket using the provided TLS connector.
async fn ws_connect(
    url: &str,
    connector: &Connector,
) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>> {
    let (ws, _resp) = tokio_tungstenite::connect_async_tls_with_config(
        url,
        None,
        false,
        Some(connector.clone()),
    )
    .await
    .context("websocket connect")?;
    Ok(ws)
}

#[derive(Parser)]
#[command(name = "nixling-relay-bridge", about = "Tunnel a byte stream over an Azure Relay Hybrid Connection")]
struct Cli {
    /// Relay namespace FQDN, e.g. relns-xxxx.servicebus.windows.net.
    #[arg(long, env = "NIXLING_RELAY_NAMESPACE")]
    namespace: String,

    /// Hybrid connection name (the entity path), e.g. hc-nixling-display.
    #[arg(long, env = "NIXLING_RELAY_ENTITY")]
    entity: String,

    /// SAS key name (authorization rule), e.g. gateway-send / gateway-listen.
    #[arg(long, env = "NIXLING_RELAY_KEY_NAME")]
    key_name: String,

    /// SAS key (primary key of the rule).
    #[arg(long, env = "NIXLING_RELAY_KEY")]
    key: String,

    /// SAS token lifetime in seconds.
    #[arg(long, default_value_t = 3600)]
    token_ttl: u64,

    /// Extra CA certificate file (PEM) to trust, in addition to the
    /// built-in Mozilla roots. Required inside ACA sandboxes, whose
    /// transparent egress proxy terminates TLS and presents a cert signed
    /// by the injected `adc-egress-proxy-ca` (typically
    /// /etc/ssl/certs/adc-egress-proxy-ca.crt).
    #[arg(long, env = "NIXLING_RELAY_CA_FILE")]
    ca_file: Option<String>,

    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// Sender role (run in the sandbox): relay <-> tcp:HOST:PORT.
    Send {
        /// Local TCP target to bridge, host:port (e.g. 127.0.0.1:8080).
        #[arg(long, default_value = "127.0.0.1:8080")]
        target: String,
    },
    /// Listener role (run on the host): relay <-> unix:PATH (or tcp:HOST:PORT).
    Listen {
        /// Local target to bridge each accepted connection to.
        /// `unix:/path` or `tcp:host:port`.
        #[arg(long)]
        target: String,
    },
}

/// Build a Service Bus SAS token conferring rights on the entity.
fn sas_token(namespace: &str, entity: &str, key_name: &str, key: &str, ttl: u64) -> Result<String> {
    // Resource URI: the http form of the entity address, URL-encoded, lowercased.
    let resource = format!("http://{namespace}/{entity}");
    let resource_enc = urlencoding::encode(&resource).to_lowercase();
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        + ttl;
    let to_sign = format!("{resource_enc}\n{expiry}");
    let mut mac = HmacSha256::new_from_slice(key.as_bytes()).map_err(|e| anyhow!("hmac key: {e}"))?;
    mac.update(to_sign.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    let sig_enc = urlencoding::encode(&sig);
    Ok(format!(
        "SharedAccessSignature sr={resource_enc}&sig={sig_enc}&se={expiry}&skn={key_name}"
    ))
}

fn rand_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..16).map(|_| format!("{:02x}", rng.gen::<u8>())).collect()
}

/// A local byte target: a connected unix or tcp stream, type-erased.
enum Local {
    Tcp(TcpStream),
    Unix(UnixStream),
}

async fn connect_local(target: &str) -> Result<Local> {
    if let Some(path) = target.strip_prefix("unix-listen:") {
        // Listen on a unix socket and accept exactly one connection (the
        // local peer, e.g. `waypipe server`, dials in). Removes the need
        // for an intermediary socat in the sandbox.
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).with_context(|| format!("bind unix-listen:{path}"))?;
        eprintln!("[relay-bridge] waiting for local connection on unix:{path}");
        let (stream, _) = listener.accept().await.with_context(|| format!("accept unix-listen:{path}"))?;
        Ok(Local::Unix(stream))
    } else if let Some(path) = target.strip_prefix("unix:") {
        Ok(Local::Unix(UnixStream::connect(path).await.with_context(|| format!("connect unix:{path}"))?))
    } else if let Some(addr) = target.strip_prefix("tcp:") {
        Ok(Local::Tcp(TcpStream::connect(addr).await.with_context(|| format!("connect tcp:{addr}"))?))
    } else {
        // bare host:port defaults to tcp
        Ok(Local::Tcp(TcpStream::connect(target).await.with_context(|| format!("connect {target}"))?))
    }
}

/// Pump bytes between a websocket and a local stream until either closes.
async fn pump<S>(ws: S, local: Local) -> Result<()>
where
    S: StreamExt<Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    match local {
        Local::Tcp(s) => pump_io(ws, s).await,
        Local::Unix(s) => pump_io(ws, s).await,
    }
}

async fn pump_io<S, T>(mut ws: S, mut io: T) -> Result<()>
where
    S: StreamExt<Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
    T: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        tokio::select! {
            // Local -> relay
            n = io.read(&mut buf) => {
                let n = n.context("local read")?;
                if n == 0 {
                    let _ = ws.send(Message::Close(None)).await;
                    return Ok(());
                }
                ws.send(Message::Binary(buf[..n].to_vec())).await.context("ws send")?;
            }
            // Relay -> local
            msg = ws.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        io.write_all(&data).await.context("local write")?;
                    }
                    Some(Ok(Message::Text(_))) => { /* control text on rendezvous: ignore */ }
                    Some(Ok(Message::Ping(p))) => { ws.send(Message::Pong(p)).await.ok(); }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => {
                        return Ok(());
                    }
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(e)) => return Err(anyhow!("ws error: {e}")),
                }
            }
        }
    }
}

async fn run_send(cli: &Cli, target: &str, connector: &Connector) -> Result<()> {
    let token = sas_token(&cli.namespace, &cli.entity, &cli.key_name, &cli.key, cli.token_ttl)?;
    let url = format!(
        "wss://{}/$hc/{}?sb-hc-action=connect&sb-hc-id={}&sb-hc-token={}",
        cli.namespace,
        urlencoding::encode(&cli.entity),
        rand_id(),
        urlencoding::encode(&token),
    );
    eprintln!("[relay-bridge] sender connecting to relay for entity {}", cli.entity);
    let ws = ws_connect(&url, connector).await.context("relay sender connect")?;
    eprintln!("[relay-bridge] sender connected; bridging to {target}");
    let local = connect_local(target).await?;
    pump(ws, local).await
}

async fn run_listen(cli: &Cli, target: &str, connector: &Connector) -> Result<()> {
    // The Azure Relay control channel is periodically closed by the service
    // (idle timeout / rebalancing). A real listener reconnects; mirror that
    // so the host side stays available across the demo's lifetime.
    loop {
        if let Err(e) = listen_control_once(cli, target, connector).await {
            eprintln!("[relay-bridge] control channel ended: {e:#}; reconnecting in 1s");
        } else {
            eprintln!("[relay-bridge] control channel closed; reconnecting in 1s");
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

async fn listen_control_once(cli: &Cli, target: &str, connector: &Connector) -> Result<()> {
    let token = sas_token(&cli.namespace, &cli.entity, &cli.key_name, &cli.key, cli.token_ttl)?;
    let url = format!(
        "wss://{}/$hc/{}?sb-hc-action=listen&sb-hc-token={}",
        cli.namespace,
        urlencoding::encode(&cli.entity),
        urlencoding::encode(&token),
    );
    eprintln!("[relay-bridge] listener opening control channel for entity {}", cli.entity);
    let mut control = ws_connect(&url, connector).await.context("relay listen connect")?;
    eprintln!("[relay-bridge] listener ready; waiting for sender connections");

    while let Some(msg) = control.next().await {
        match msg.context("control channel")? {
            Message::Text(text) => {
                let v: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(addr) = v.get("accept").and_then(|a| a.get("address")).and_then(|s| s.as_str()) {
                    let address = addr.to_string();
                    let target = target.to_string();
                    let connector = connector.clone();
                    eprintln!("[relay-bridge] accept -> opening rendezvous");
                    tokio::spawn(async move {
                        if let Err(e) = accept_one(&address, &target, connector).await {
                            eprintln!("[relay-bridge] rendezvous error: {e:#}");
                        }
                    });
                }
            }
            Message::Ping(p) => { control.send(Message::Pong(p)).await.ok(); }
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    }
    Ok(())
}

async fn accept_one(address: &str, target: &str, connector: Connector) -> Result<()> {
    let ws = ws_connect(address, &connector).await.context("rendezvous connect")?;
    let local = connect_local(target).await?;
    eprintln!("[relay-bridge] rendezvous up; bridging to {target}");
    pump(ws, local).await
}

#[tokio::main]
async fn main() -> Result<()> {
    // rustls 0.23 requires an explicit process-level crypto provider.
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow!("failed to install rustls ring crypto provider"))?;
    let cli = Cli::parse();
    let connector = build_connector(cli.ca_file.as_deref())?;
    match &cli.mode {
        Mode::Send { target } => run_send(&cli, target, &connector).await,
        Mode::Listen { target } => run_listen(&cli, target, &connector).await,
    }
}
