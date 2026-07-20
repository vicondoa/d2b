use std::fmt;

use async_trait::async_trait;
use d2b_contracts::v2_component_session::{Locality, TransportClass};
use d2b_session::{OwnedTransport, TransportDescriptor, TransportError, TransportPacket};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_vsock::{VMADDR_CID_ANY, VsockAddr, VsockListener, VsockStream};

pub struct FramedVsockTransport<S> {
    stream: S,
    closed: bool,
}

impl<S> FramedVsockTransport<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            closed: false,
        }
    }
}

impl<S> fmt::Debug for FramedVsockTransport<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FramedVsockTransport")
            .field("closed", &self.closed)
            .finish()
    }
}

#[async_trait]
impl<S> OwnedTransport for FramedVsockTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    fn descriptor(&self) -> TransportDescriptor {
        TransportDescriptor {
            class: TransportClass::NativeVsock,
            locality: Locality::GuestLocal,
            packet_atomic: false,
            supports_attachments: false,
        }
    }

    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected);
        }
        let mut length = [0_u8; 2];
        self.stream.read_exact(&mut length).await.map_err(map_io)?;
        let length = usize::from(u16::from_be_bytes(length));
        if length == 0 || length > protected_limit {
            return Err(TransportError::LimitExceeded);
        }
        let mut bytes = vec![0_u8; length];
        self.stream.read_exact(&mut bytes).await.map_err(map_io)?;
        Ok(TransportPacket::new(bytes))
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Disconnected);
        }
        let (bytes, attachments) = packet.into_parts();
        if !attachments.is_empty() {
            return Err(TransportError::InvalidAttachment);
        }
        if bytes.is_empty() || bytes.len() > usize::from(u16::MAX) {
            return Err(TransportError::LimitExceeded);
        }
        self.stream
            .write_all(&(bytes.len() as u16).to_be_bytes())
            .await
            .map_err(map_io)?;
        self.stream.write_all(&bytes).await.map_err(map_io)?;
        self.stream.flush().await.map_err(map_io)
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        if !self.closed {
            self.closed = true;
            self.stream.shutdown().await.map_err(map_io)?;
        }
        Ok(())
    }
}

pub type NativeVsockTransport = FramedVsockTransport<VsockStream>;

impl FramedVsockTransport<VsockStream> {
    pub async fn connect(cid: u32, port: u32) -> Result<Self, TransportError> {
        if cid <= 2 || port == 0 {
            return Err(TransportError::Other);
        }
        VsockStream::connect(VsockAddr::new(cid, port))
            .await
            .map(Self::new)
            .map_err(map_io)
    }
}

pub struct NativeVsockListener {
    listener: VsockListener,
    port: u32,
}

impl NativeVsockListener {
    pub fn bind(port: u32) -> Result<Self, TransportError> {
        if port == 0 {
            return Err(TransportError::Other);
        }
        VsockListener::bind(VsockAddr::new(VMADDR_CID_ANY, port))
            .map(|listener| Self { listener, port })
            .map_err(map_io)
    }

    pub async fn accept(
        &mut self,
        expected_cid: u32,
    ) -> Result<NativeVsockTransport, TransportError> {
        if expected_cid <= 2 {
            return Err(TransportError::Other);
        }
        accept_expected(&mut self.listener, expected_cid)
            .await
            .map(FramedVsockTransport::new)
    }

    pub const fn port(&self) -> u32 {
        self.port
    }
}

#[async_trait]
trait AcceptOne {
    type Stream: Send;

    async fn accept_one(&mut self) -> Result<(Self::Stream, u32, u32), TransportError>;
}

#[async_trait]
impl AcceptOne for VsockListener {
    type Stream = VsockStream;

    async fn accept_one(&mut self) -> Result<(Self::Stream, u32, u32), TransportError> {
        let (stream, peer) = self.accept().await.map_err(map_io)?;
        Ok((stream, peer.cid(), peer.port()))
    }
}

async fn accept_expected<A>(
    listener: &mut A,
    expected_cid: u32,
) -> Result<A::Stream, TransportError>
where
    A: AcceptOne + Send,
{
    loop {
        let (stream, cid, port) = listener.accept_one().await?;
        if cid == expected_cid && port != 0 {
            return Ok(stream);
        }
        drop(stream);
    }
}

impl fmt::Debug for NativeVsockListener {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeVsockListener")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

fn map_io(error: std::io::Error) -> TransportError {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::UnexpectedEof
        | ErrorKind::BrokenPipe
        | ErrorKind::ConnectionAborted
        | ErrorKind::ConnectionReset
        | ErrorKind::NotConnected => TransportError::Disconnected,
        ErrorKind::WouldBlock => TransportError::WouldBlock,
        _ => TransportError::Other,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use super::*;

    #[tokio::test]
    async fn in_memory_vsock_adapter_is_framed_and_rejects_attachments() {
        let (left, right) = tokio::io::duplex(512);
        let mut sender = FramedVsockTransport::new(left);
        let mut receiver = FramedVsockTransport::new(right);
        sender
            .send(TransportPacket::new(b"guest-v2".to_vec()))
            .await
            .unwrap();
        assert_eq!(receiver.receive(64).await.unwrap().as_bytes(), b"guest-v2");
        assert_eq!(sender.descriptor().class, TransportClass::NativeVsock);
    }

    #[tokio::test]
    async fn in_memory_vsock_adapter_enforces_frame_limit_and_disconnect() {
        let (left, right) = tokio::io::duplex(512);
        let mut sender = FramedVsockTransport::new(left);
        let mut receiver = FramedVsockTransport::new(right);
        sender
            .send(TransportPacket::new(vec![1; 65]))
            .await
            .unwrap();
        assert_eq!(
            receiver.receive(64).await.unwrap_err(),
            TransportError::LimitExceeded
        );
        sender.close().await.unwrap();
        assert_eq!(
            sender
                .send(TransportPacket::new(vec![1]))
                .await
                .unwrap_err(),
            TransportError::Disconnected
        );
    }

    struct TrackedStream(Arc<AtomicUsize>);

    impl Drop for TrackedStream {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    struct FakeAcceptor {
        peers: VecDeque<(TrackedStream, u32, u32, Duration)>,
        wait_forever: bool,
    }

    #[async_trait]
    impl AcceptOne for FakeAcceptor {
        type Stream = TrackedStream;

        async fn accept_one(&mut self) -> Result<(Self::Stream, u32, u32), TransportError> {
            if let Some((stream, cid, port, delay)) = self.peers.pop_front() {
                tokio::time::sleep(delay).await;
                return Ok((stream, cid, port));
            }
            if self.wait_forever {
                std::future::pending().await
            } else {
                Err(TransportError::Disconnected)
            }
        }
    }

    #[tokio::test]
    async fn expected_cid_accept_discards_repeated_foreign_peers() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let mut acceptor = FakeAcceptor {
            peers: VecDeque::from([
                (TrackedStream(Arc::clone(&dropped)), 41, 100, Duration::ZERO),
                (TrackedStream(Arc::clone(&dropped)), 43, 101, Duration::ZERO),
                (TrackedStream(Arc::clone(&dropped)), 42, 102, Duration::ZERO),
            ]),
            wait_forever: false,
        };
        let expected = accept_expected(&mut acceptor, 42).await.unwrap();
        assert_eq!(dropped.load(Ordering::Acquire), 2);
        drop(expected);
        assert_eq!(dropped.load(Ordering::Acquire), 3);
    }

    #[tokio::test]
    async fn foreign_peer_does_not_reset_original_accept_deadline() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let mut acceptor = FakeAcceptor {
            peers: VecDeque::from([
                (
                    TrackedStream(Arc::clone(&dropped)),
                    41,
                    100,
                    Duration::from_millis(10),
                ),
                (
                    TrackedStream(Arc::clone(&dropped)),
                    42,
                    101,
                    Duration::from_millis(15),
                ),
            ]),
            wait_forever: false,
        };
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                accept_expected(&mut acceptor, 42)
            )
            .await
            .is_err()
        );
        assert_eq!(dropped.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn cancelling_accept_closes_foreign_peer_and_pending_listener() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let acceptor = FakeAcceptor {
            peers: VecDeque::from([(TrackedStream(Arc::clone(&dropped)), 41, 100, Duration::ZERO)]),
            wait_forever: true,
        };
        let task = tokio::spawn(async move {
            let mut acceptor = acceptor;
            accept_expected(&mut acceptor, 42).await
        });
        tokio::task::yield_now().await;
        task.abort();
        let _ = task.await;
        assert_eq!(dropped.load(Ordering::Acquire), 1);
    }
}
