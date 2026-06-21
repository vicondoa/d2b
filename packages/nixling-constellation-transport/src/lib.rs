//! `nixling-constellation-transport` (ADR 0032): the transport-trait home
//! plus an in-memory **loopback** [`TransportProvider`] used for
//! conformance. The loopback connects a sender side to a listener side
//! over `tokio::io::duplex` — no real socket is opened — so the
//! byte-carrying transport/mux contract can be exercised hermetically.
//!
//! Dependency direction: depends only on `nixling-constellation-core` +
//! `nixling-constellation-provider` + `async-trait`/`tokio`. It MUST NOT
//! depend on a protocol codec, a real transport, or any host-only
//! broker/daemon internals (enforced by the constellation
//! dependency-direction CI gate).

use async_trait::async_trait;
use nixling_constellation_core::ErrorKind;
use nixling_constellation_core::{NodeId, ProviderId};
use nixling_constellation_provider::error::{ProviderError, ProviderResult};
use nixling_constellation_provider::provider::{TransportListener, TransportProvider};
use nixling_constellation_provider::types::{
    NodeRegistration, SafeLabel, TransportSession, TransportTarget,
};
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Mutex, mpsc, mpsc::error::TrySendError};

/// Default in-memory duplex buffer size for a loopback session.
const LOOPBACK_BUF: usize = 64 * 1024;

/// An in-memory loopback transport. Each `connect` creates a duplex pair,
/// hands one end back to the caller, and queues the other end for the
/// listener's `accept`. Single listener; multiple concurrent connects are
/// supported (each is an independent duplex pair).
pub struct LoopbackTransport {
    id: ProviderId,
    tx: StdMutex<Option<mpsc::Sender<TransportSession>>>,
    rx: Mutex<Option<mpsc::Receiver<TransportSession>>>,
    closed: Arc<AtomicBool>,
}

impl LoopbackTransport {
    /// Build a loopback transport.
    pub fn new() -> Self {
        Self::with_queue_capacity(16)
    }

    /// Build a loopback transport with an explicit pending-session queue
    /// capacity. Capacity zero is rounded up to one so the transport never
    /// constructs an invalid Tokio channel.
    pub fn with_queue_capacity(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        Self {
            id: ProviderId::parse("loopback").expect("valid provider id"),
            tx: StdMutex::new(Some(tx)),
            rx: Mutex::new(Some(rx)),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Close the in-memory transport. Future connects and accepts report a
    /// typed relay-unavailable error.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        if let Ok(mut tx) = self.tx.lock() {
            tx.take();
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
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
        if self.is_closed() {
            return Err(ProviderError::new(
                ErrorKind::RelayUnavailable,
                "loopback transport is closed",
            ));
        }
        let (near, far) = tokio::io::duplex(LOOPBACK_BUF);
        // Queue the far end for the listener; hand the near end to the caller.
        let tx = self
            .tx
            .lock()
            .map_err(|_| ProviderError::new(ErrorKind::RelayUnavailable, "loopback closed"))?
            .clone()
            .ok_or_else(|| ProviderError::new(ErrorKind::RelayUnavailable, "loopback closed"))?;
        match tx.try_send(TransportSession::new(
            SafeLabel::new("loopback-accept"),
            Box::new(far),
        )) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err(ProviderError::new(
                    ErrorKind::Backpressure,
                    "loopback pending-session queue is full",
                ));
            }
            Err(TrySendError::Closed(_)) => {
                return Err(ProviderError::new(
                    ErrorKind::RelayUnavailable,
                    "loopback listener is gone",
                ));
            }
        }
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
            closed: Arc::clone(&self.closed),
        }))
    }
}

/// The accept side of a [`LoopbackTransport`].
struct LoopbackListener {
    node: NodeId,
    rx: Mutex<mpsc::Receiver<TransportSession>>,
    closed: Arc<AtomicBool>,
}

#[async_trait]
impl TransportListener for LoopbackListener {
    fn node(&self) -> NodeId {
        self.node.clone()
    }

    async fn accept(&self) -> ProviderResult<TransportSession> {
        if self.closed.load(Ordering::Acquire) {
            return Err(ProviderError::new(
                ErrorKind::RelayUnavailable,
                "loopback closed",
            ));
        }
        self.rx
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| ProviderError::new(ErrorKind::RelayUnavailable, "loopback closed"))
    }
}

/// Reusable transport conformance checks for in-process and future real
/// transport providers. These are hermetic and assert only the byte/session
/// contract exposed by [`TransportProvider`].
pub mod conformance {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn registration() -> NodeRegistration {
        NodeRegistration {
            node: NodeId::parse("gw").expect("valid conformance node id"),
        }
    }

    fn target() -> TransportTarget {
        TransportTarget {
            endpoint: "loopback".to_owned(),
        }
    }

    /// Accept/connect one session and prove bytes are bidirectional.
    pub async fn accepts_and_round_trips(provider: &dyn TransportProvider) -> ProviderResult<()> {
        let listener = provider.listen(registration()).await?;
        let (mut sender, mut accepted) =
            tokio::try_join!(provider.connect(target()), listener.accept())?;
        sender
            .stream_mut()
            .write_all(b"hello")
            .await
            .map_err(|err| {
                ProviderError::new(ErrorKind::RelayUnavailable, format!("write failed: {err}"))
            })?;
        let mut buf = [0_u8; 5];
        accepted
            .stream_mut()
            .read_exact(&mut buf)
            .await
            .map_err(|err| {
                ProviderError::new(ErrorKind::RelayUnavailable, format!("read failed: {err}"))
            })?;
        if &buf != b"hello" {
            return Err(ProviderError::new(
                ErrorKind::MalformedFrame,
                "transport corrupted forward bytes",
            ));
        }
        accepted
            .stream_mut()
            .write_all(b"ack")
            .await
            .map_err(|err| {
                ProviderError::new(ErrorKind::RelayUnavailable, format!("write failed: {err}"))
            })?;
        let mut back = [0_u8; 3];
        sender
            .stream_mut()
            .read_exact(&mut back)
            .await
            .map_err(|err| {
                ProviderError::new(ErrorKind::RelayUnavailable, format!("read failed: {err}"))
            })?;
        if &back != b"ack" {
            return Err(ProviderError::new(
                ErrorKind::MalformedFrame,
                "transport corrupted reverse bytes",
            ));
        }
        Ok(())
    }

    /// Open several sessions on one listener and prove they do not cross-talk.
    pub async fn concurrent_sessions_are_isolated(
        provider: &dyn TransportProvider,
        count: usize,
    ) -> ProviderResult<()> {
        let listener = provider.listen(registration()).await?;
        let mut sessions = Vec::with_capacity(count);
        for index in 0..count {
            let (sender, accepted) =
                tokio::try_join!(provider.connect(target()), listener.accept())?;
            sessions.push((index, sender, accepted));
        }
        let mut tasks = Vec::with_capacity(count);
        for (index, mut sender, mut accepted) in sessions {
            tasks.push(tokio::spawn(async move {
                let byte = b'a'.wrapping_add(index as u8);
                sender
                    .stream_mut()
                    .write_all(&[byte])
                    .await
                    .map_err(|err| {
                        ProviderError::new(
                            ErrorKind::RelayUnavailable,
                            format!("write failed: {err}"),
                        )
                    })?;
                let mut buf = [0_u8; 1];
                accepted
                    .stream_mut()
                    .read_exact(&mut buf)
                    .await
                    .map_err(|err| {
                        ProviderError::new(
                            ErrorKind::RelayUnavailable,
                            format!("read failed: {err}"),
                        )
                    })?;
                if buf[0] != byte {
                    return Err(ProviderError::new(
                        ErrorKind::MalformedFrame,
                        "transport sessions crossed bytes",
                    ));
                }
                Ok(())
            }));
        }
        for task in tasks {
            task.await.map_err(|err| {
                ProviderError::new(ErrorKind::RelayUnavailable, format!("task failed: {err}"))
            })??;
        }
        Ok(())
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

    #[tokio::test]
    async fn loopback_passes_shared_conformance() {
        let transport = LoopbackTransport::new();
        conformance::accepts_and_round_trips(&transport)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn loopback_concurrent_sessions_are_isolated() {
        let transport = LoopbackTransport::new();
        conformance::concurrent_sessions_are_isolated(&transport, 4)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn loopback_queue_capacity_is_enforced() {
        let transport = LoopbackTransport::with_queue_capacity(1);
        let _listener = transport
            .listen(NodeRegistration {
                node: NodeId::parse("gw").unwrap(),
            })
            .await
            .unwrap();
        let first = transport
            .connect(TransportTarget {
                endpoint: "loopback".to_owned(),
            })
            .await;
        assert!(first.is_ok());
        let second = transport
            .connect(TransportTarget {
                endpoint: "loopback".to_owned(),
            })
            .await;
        assert_eq!(second.unwrap_err().kind(), ErrorKind::Backpressure);
    }

    #[tokio::test]
    async fn loopback_shutdown_reports_typed_errors() {
        let transport = LoopbackTransport::new();
        let listener = transport
            .listen(NodeRegistration {
                node: NodeId::parse("gw").unwrap(),
            })
            .await
            .unwrap();
        transport.close();
        assert!(listener.accept().await.is_err());
        assert!(
            transport
                .connect(TransportTarget {
                    endpoint: "loopback".to_owned(),
                })
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn loopback_shutdown_wakes_pending_accept() {
        let transport = LoopbackTransport::new();
        let listener = transport
            .listen(NodeRegistration {
                node: NodeId::parse("gw").unwrap(),
            })
            .await
            .unwrap();
        let accept = tokio::spawn(async move { listener.accept().await });
        transport.close();
        assert!(accept.await.unwrap().is_err());
    }
}
