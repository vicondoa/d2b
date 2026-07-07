use async_trait::async_trait;
use d2b_realm_core::{ErrorKind, NodeId, ProviderId};
use d2b_realm_provider::error::{ProviderError, ProviderResult};
use d2b_realm_provider::provider::{TransportListener, TransportProvider};
use d2b_realm_provider::types::{NodeRegistration, SafeLabel, TransportSession, TransportTarget};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify};

const LOCAL_TCP_SCHEME: &str = "tcp+local://";

/// A loopback-only TCP transport adapter used to prove the transport
/// abstraction is not Azure-specific. It is intentionally local, plaintext,
/// credential-free, and safe for hermetic tests.
pub struct LocalTcpTransport {
    id: ProviderId,
    listener: Mutex<Option<TcpListener>>,
    local_addr: SocketAddr,
    closed: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
}

impl LocalTcpTransport {
    /// Bind a loopback TCP listener. Use port `0` in tests for an
    /// OS-assigned ephemeral port.
    pub async fn bind(bind_addr: SocketAddr) -> ProviderResult<Self> {
        validate_bind_addr(bind_addr)?;
        let listener = TcpListener::bind(bind_addr)
            .await
            .map_err(|err| local_tcp_io_error("bind", err.kind()))?;
        let local_addr = listener
            .local_addr()
            .map_err(|err| local_tcp_io_error("bind", err.kind()))?;
        Ok(Self {
            id: ProviderId::parse("local-tcp").expect("valid provider id"),
            listener: Mutex::new(Some(listener)),
            local_addr,
            closed: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(Notify::new()),
        })
    }

    /// Bind `127.0.0.1:0`.
    pub async fn bind_loopback_v4() -> ProviderResult<Self> {
        Self::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await
    }

    /// The OS-selected local address.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// A URI-like transport target for this listener.
    pub fn target(&self) -> TransportTarget {
        TransportTarget {
            endpoint: format!("{LOCAL_TCP_SCHEME}{}", self.local_addr),
        }
    }

    /// Close the local TCP listener. Future connects and accepts fail with a
    /// typed transport error.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.shutdown.notify_waiters();
    }
}

#[async_trait]
impl TransportProvider for LocalTcpTransport {
    fn transport_id(&self) -> ProviderId {
        self.id.clone()
    }

    async fn connect(&self, target: TransportTarget) -> ProviderResult<TransportSession> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ProviderError::new(
                ErrorKind::RelayUnavailable,
                "local-tcp-closed",
            ));
        }
        let addr = parse_target(&target.endpoint)?;
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|err| local_tcp_io_error("connect", err.kind()))?;
        Ok(TransportSession::new(
            SafeLabel::new("local-tcp-connect"),
            Box::new(stream),
        ))
    }

    async fn listen(
        &self,
        registration: NodeRegistration,
    ) -> ProviderResult<Box<dyn TransportListener>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ProviderError::new(
                ErrorKind::RelayUnavailable,
                "local-tcp-closed",
            ));
        }
        let listener = self.listener.lock().await.take().ok_or_else(|| {
            ProviderError::new(ErrorKind::RelayUnavailable, "local-tcp-listener-taken")
        })?;
        Ok(Box::new(LocalTcpListener {
            node: registration.node,
            listener,
            closed: Arc::clone(&self.closed),
            shutdown: Arc::clone(&self.shutdown),
        }))
    }
}

struct LocalTcpListener {
    node: NodeId,
    listener: TcpListener,
    closed: Arc<AtomicBool>,
    shutdown: Arc<Notify>,
}

#[async_trait]
impl TransportListener for LocalTcpListener {
    fn node(&self) -> NodeId {
        self.node.clone()
    }

    async fn accept(&self) -> ProviderResult<TransportSession> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ProviderError::new(
                ErrorKind::RelayUnavailable,
                "local-tcp-closed",
            ));
        }
        let accepted = tokio::select! {
            _ = self.shutdown.notified() => {
                return Err(ProviderError::new(ErrorKind::RelayUnavailable, "local-tcp-closed"));
            }
            accepted = self.listener.accept() => accepted,
        };
        let (stream, _) = accepted.map_err(|err| local_tcp_io_error("accept", err.kind()))?;
        Ok(TransportSession::new(
            SafeLabel::new("local-tcp-accept"),
            Box::new(stream),
        ))
    }
}

fn validate_bind_addr(addr: SocketAddr) -> ProviderResult<()> {
    if !addr.ip().is_loopback() || addr.ip().is_unspecified() {
        return Err(ProviderError::new(
            ErrorKind::InvalidTarget,
            "local-tcp-bind-address-denied",
        ));
    }
    if addr.port() != 0 && addr.port() < 1024 {
        return Err(ProviderError::new(
            ErrorKind::InvalidTarget,
            "local-tcp-privileged-port-denied",
        ));
    }
    Ok(())
}

fn parse_target(endpoint: &str) -> ProviderResult<SocketAddr> {
    let raw = endpoint
        .strip_prefix(LOCAL_TCP_SCHEME)
        .ok_or_else(|| ProviderError::new(ErrorKind::InvalidTarget, "local-tcp-target-invalid"))?;
    let addr = raw
        .parse::<SocketAddr>()
        .map_err(|_| ProviderError::new(ErrorKind::InvalidTarget, "local-tcp-target-invalid"))?;
    validate_connect_addr(addr)?;
    Ok(addr)
}

fn validate_connect_addr(addr: SocketAddr) -> ProviderResult<()> {
    if !addr.ip().is_loopback() || addr.ip().is_unspecified() || addr.port() < 1024 {
        return Err(ProviderError::new(
            ErrorKind::InvalidTarget,
            "local-tcp-target-denied",
        ));
    }
    Ok(())
}

fn local_tcp_io_error(stage: &'static str, kind: std::io::ErrorKind) -> ProviderError {
    ProviderError::new(
        ErrorKind::RelayUnavailable,
        format!("local-tcp-{stage}-failed:{}", io_reason(kind)),
    )
}

fn io_reason(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::AddrInUse => "address-in-use",
        std::io::ErrorKind::AddrNotAvailable => "address-not-available",
        std::io::ErrorKind::ConnectionRefused => "connection-refused",
        std::io::ErrorKind::ConnectionReset => "connection-reset",
        std::io::ErrorKind::ConnectionAborted => "connection-aborted",
        std::io::ErrorKind::TimedOut => "timeout",
        std::io::ErrorKind::PermissionDenied => "permission-denied",
        std::io::ErrorKind::BrokenPipe => "broken-pipe",
        std::io::ErrorKind::UnexpectedEof => "unexpected-eof",
        _ => "io-error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::{
        accepts_and_round_trips_with_target, concurrent_sessions_are_isolated_with_target,
    };
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn local_tcp_passes_round_trip_conformance() {
        let transport = LocalTcpTransport::bind_loopback_v4().await.unwrap();
        accepts_and_round_trips_with_target(&transport, transport.target())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn local_tcp_concurrent_sessions_are_isolated() {
        let transport = LocalTcpTransport::bind_loopback_v4().await.unwrap();
        concurrent_sessions_are_isolated_with_target(&transport, transport.target(), 4)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn local_tcp_rejects_unsafe_bind_addresses() {
        for addr in [
            SocketAddr::from(([0, 0, 0, 0], 0)),
            SocketAddr::from(([192, 0, 2, 10], 1500)),
            SocketAddr::from(([127, 0, 0, 1], 22)),
        ] {
            let err = match LocalTcpTransport::bind(addr).await {
                Ok(_) => panic!("unsafe bind address was accepted"),
                Err(err) => err,
            };
            assert_eq!(err.kind(), ErrorKind::InvalidTarget);
            let rendered = err.to_string();
            assert!(!rendered.contains(&addr.to_string()));
        }
    }

    #[tokio::test]
    async fn local_tcp_rejects_bad_targets_without_endpoint_leakage() {
        let transport = LocalTcpTransport::bind_loopback_v4().await.unwrap();
        for endpoint in [
            "tcp://127.0.0.1:12345",
            "tcp+local://0.0.0.0:12345",
            "tcp+local://192.0.2.10:12345",
            "tcp+local://127.0.0.1:22",
        ] {
            let err = transport
                .connect(TransportTarget {
                    endpoint: endpoint.to_owned(),
                })
                .await
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidTarget);
            assert!(!err.to_string().contains(endpoint));
        }
    }

    #[tokio::test]
    async fn local_tcp_connection_refused_is_typed_and_redacted() {
        let transport = LocalTcpTransport::bind_loopback_v4().await.unwrap();
        let addr = transport.local_addr();
        drop(transport);
        let target = TransportTarget {
            endpoint: format!("{LOCAL_TCP_SCHEME}{addr}"),
        };
        let connector = LocalTcpTransport::bind_loopback_v4().await.unwrap();
        let err = connector.connect(target).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::RelayUnavailable);
        assert!(err.to_string().contains("connection-refused"));
        assert!(!err.to_string().contains(&addr.to_string()));
    }

    #[tokio::test]
    async fn local_tcp_bind_error_is_categorized_and_redacted() {
        let first = LocalTcpTransport::bind_loopback_v4().await.unwrap();
        let addr = first.local_addr();
        let err = match LocalTcpTransport::bind(addr).await {
            Ok(_) => panic!("duplicate bind unexpectedly succeeded"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), ErrorKind::RelayUnavailable);
        assert!(err.to_string().contains("address-in-use"));
        assert!(!err.to_string().contains(&addr.to_string()));
    }

    #[tokio::test]
    async fn local_tcp_shutdown_wakes_pending_accept() {
        let transport = LocalTcpTransport::bind_loopback_v4().await.unwrap();
        let listener = transport
            .listen(NodeRegistration {
                node: NodeId::parse("gw").unwrap(),
            })
            .await
            .unwrap();
        let accept = tokio::spawn(async move { listener.accept().await });
        transport.close();
        let err = tokio::time::timeout(std::time::Duration::from_secs(1), accept)
            .await
            .expect("accept should wake")
            .unwrap()
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::RelayUnavailable);
    }

    #[tokio::test]
    async fn local_tcp_unexpected_eof_is_observable_to_reader() {
        let transport = LocalTcpTransport::bind_loopback_v4().await.unwrap();
        let listener = transport
            .listen(NodeRegistration {
                node: NodeId::parse("gw").unwrap(),
            })
            .await
            .unwrap();
        let (client, accepted) =
            tokio::try_join!(transport.connect(transport.target()), listener.accept()).unwrap();
        drop(accepted);
        let mut stream = client.into_stream();
        let mut buf = [0_u8; 1];
        assert_eq!(stream.read(&mut buf).await.unwrap(), 0);
    }
}
