//! `nixling-constellation-transport` (ADR 0032): the transport-trait home
//! plus an in-memory **loopback** [`TransportProvider`] used for
//! conformance. The loopback connects a sender side to a listener side
//! over `tokio::io::duplex` — no real socket is opened — so the
//! byte-carrying transport/mux contract can be exercised hermetically.
//!
//! Dependency direction: depends only on `nixling-constellation-core` +
//! `nixling-constellation-provider` + `async-trait`/`tokio`. It MUST NOT
//! depend on a protocol codec, a real transport, or any host-only
//! broker/daemon internals (enforced by `tests/unit/meta/w0-dep-direction.sh`).

use async_trait::async_trait;
use nixling_constellation_core::ErrorKind;
use nixling_constellation_core::{NodeId, ProviderId};
use nixling_constellation_provider::error::{ProviderError, ProviderResult};
use nixling_constellation_provider::provider::{TransportListener, TransportProvider};
use nixling_constellation_provider::types::{
    NodeRegistration, SafeLabel, TransportSession, TransportTarget,
};
use tokio::sync::{mpsc, Mutex};

/// Default in-memory duplex buffer size for a loopback session.
const LOOPBACK_BUF: usize = 64 * 1024;

/// An in-memory loopback transport. Each `connect` creates a duplex pair,
/// hands one end back to the caller, and queues the other end for the
/// listener's `accept`. Single listener; multiple concurrent connects are
/// supported (each is an independent duplex pair).
pub struct LoopbackTransport {
    id: ProviderId,
    tx: mpsc::Sender<TransportSession>,
    rx: Mutex<Option<mpsc::Receiver<TransportSession>>>,
}

impl LoopbackTransport {
    /// Build a loopback transport.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(16);
        Self {
            id: ProviderId::parse("loopback").expect("valid provider id"),
            tx,
            rx: Mutex::new(Some(rx)),
        }
    }
}

impl Default for LoopbackTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TransportProvider for LoopbackTransport {
    fn transport_id(&self) -> ProviderId {
        self.id.clone()
    }

    async fn connect(&self, _target: TransportTarget) -> ProviderResult<TransportSession> {
        let (near, far) = tokio::io::duplex(LOOPBACK_BUF);
        // Queue the far end for the listener; hand the near end to the caller.
        self.tx
            .send(TransportSession::new(
                SafeLabel::new("loopback-accept"),
                Box::new(far),
            ))
            .await
            .map_err(|_| {
                ProviderError::new(ErrorKind::RelayUnavailable, "loopback listener is gone")
            })?;
        Ok(TransportSession::new(
            SafeLabel::new("loopback-connect"),
            Box::new(near),
        ))
    }

    async fn listen(
        &self,
        registration: NodeRegistration,
    ) -> ProviderResult<Box<dyn TransportListener>> {
        let rx = self.rx.lock().await.take().ok_or_else(|| {
            ProviderError::new(ErrorKind::RelayUnavailable, "listener already taken")
        })?;
        Ok(Box::new(LoopbackListener {
            node: registration.node,
            rx: Mutex::new(rx),
        }))
    }
}

/// The accept side of a [`LoopbackTransport`].
struct LoopbackListener {
    node: NodeId,
    rx: Mutex<mpsc::Receiver<TransportSession>>,
}

#[async_trait]
impl TransportListener for LoopbackListener {
    fn node(&self) -> NodeId {
        self.node.clone()
    }

    async fn accept(&self) -> ProviderResult<TransportSession> {
        self.rx
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| ProviderError::new(ErrorKind::RelayUnavailable, "loopback closed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn loopback_connects_and_round_trips_bytes() {
        let transport = LoopbackTransport::new();
        let listener = transport
            .listen(NodeRegistration {
                node: NodeId::parse("gw").unwrap(),
            })
            .await
            .unwrap();

        let target = TransportTarget {
            endpoint: "loopback".to_string(),
        };
        // Connect (sender) and accept (listener) concurrently.
        let (conn, acc) = tokio::join!(transport.connect(target), listener.accept());
        let mut sender = conn.unwrap();
        let mut accepted = acc.unwrap();

        sender.stream_mut().write_all(b"hello-relay").await.unwrap();
        let mut buf = [0u8; 11];
        accepted.stream_mut().read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello-relay");

        // The reverse direction works too (bidirectional duplex).
        accepted.stream_mut().write_all(b"ack").await.unwrap();
        let mut back = [0u8; 3];
        sender.stream_mut().read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"ack");
    }

    #[tokio::test]
    async fn second_listen_fails_closed() {
        let transport = LoopbackTransport::new();
        let _l = transport
            .listen(NodeRegistration {
                node: NodeId::parse("gw").unwrap(),
            })
            .await
            .unwrap();
        // A loopback has a single accept queue; a second listener fails closed.
        let second = transport
            .listen(NodeRegistration {
                node: NodeId::parse("gw").unwrap(),
            })
            .await;
        assert!(second.is_err());
    }
}
