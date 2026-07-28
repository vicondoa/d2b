use std::fmt;

use async_trait::async_trait;
use d2b_contracts::v3::component_session::{Locality, TransportClass};
use d2b_session::{OwnedTransport, TransportDescriptor, TransportError, TransportPacket};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_vsock::{VMADDR_CID_ANY, VsockAddr, VsockListener, VsockStream};

pub struct FramedVsockTransport<S> {
    stream: S,
    receive_header: Vec<u8>,
    receive_body: Vec<u8>,
    receive_declared: Option<usize>,
    outbound: Option<(Vec<u8>, usize)>,
    closed: bool,
}

impl<S> FramedVsockTransport<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            receive_header: Vec::with_capacity(4),
            receive_body: Vec::new(),
            receive_declared: None,
            outbound: None,
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
        read_exact_persistent(&mut self.stream, &mut self.receive_header, 4, false).await?;
        let length = match self.receive_declared {
            Some(length) => length,
            None => {
                let length = usize::try_from(u32::from_be_bytes([
                    self.receive_header[0],
                    self.receive_header[1],
                    self.receive_header[2],
                    self.receive_header[3],
                ]))
                .map_err(|_| TransportError::LimitExceeded)?;
                self.receive_declared = Some(length);
                length
            }
        };
        if length == 0 || length > protected_limit {
            self.closed = true;
            return Err(TransportError::LimitExceeded);
        }
        if self.receive_body.capacity() < length {
            self.receive_body
                .try_reserve_exact(length - self.receive_body.len())
                .map_err(|_| TransportError::LimitExceeded)?;
        }
        read_exact_persistent(&mut self.stream, &mut self.receive_body, length, true).await?;
        let bytes = std::mem::take(&mut self.receive_body);
        self.receive_header.clear();
        self.receive_declared = None;
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
        if bytes.is_empty() {
            return Err(TransportError::LimitExceeded);
        }
        if self.outbound.is_none() {
            let length = u32::try_from(bytes.len()).map_err(|_| TransportError::LimitExceeded)?;
            let mut framed = Vec::with_capacity(4 + bytes.len());
            framed.extend_from_slice(&length.to_be_bytes());
            framed.extend_from_slice(&bytes);
            self.outbound = Some((framed, 0));
        } else if self
            .outbound
            .as_ref()
            .is_none_or(|(pending, _)| pending[4..] != bytes)
        {
            return Err(TransportError::Other);
        }
        let result = async {
            let (pending, offset) = self.outbound.as_mut().expect("outbound was initialized");
            while *offset < pending.len() {
                match self.stream.write(&pending[*offset..]).await {
                    Ok(0) => return Err(TransportError::Disconnected),
                    Ok(written) => *offset += written,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(map_io(error)),
                }
            }
            self.stream.flush().await.map_err(map_io)
        }
        .await;
        if result.is_err() {
            self.closed = true;
        } else {
            self.outbound = None;
        }
        result
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        if !self.closed {
            self.closed = true;
            self.stream.shutdown().await.map_err(map_io)?;
        }
        Ok(())
    }
}

async fn read_exact_persistent<S: AsyncRead + Unpin>(
    stream: &mut S,
    output: &mut Vec<u8>,
    target: usize,
    frame_started: bool,
) -> Result<(), TransportError> {
    while output.len() < target {
        let remaining = target
            .checked_sub(output.len())
            .ok_or(TransportError::LimitExceeded)?;
        let mut chunk = vec![0_u8; remaining];
        match stream.read(&mut chunk).await {
            Ok(0) if output.is_empty() && !frame_started => {
                return Err(TransportError::Disconnected);
            }
            Ok(0) => return Err(TransportError::Truncated),
            Ok(count) => output.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(map_io(error)),
        }
    }
    Ok(())
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
            .send(TransportPacket::new(b"guest-v3".to_vec()))
            .await
            .unwrap();
        assert_eq!(receiver.receive(64).await.unwrap().as_bytes(), b"guest-v3");
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

    #[tokio::test]
    async fn cancelled_receive_retains_partial_header_and_body() {
        let (mut writer, right) = tokio::io::duplex(512);
        let mut receiver = FramedVsockTransport::new(right);
        writer.write_all(&[0, 0]).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(5), receiver.receive(64))
                .await
                .is_err()
        );
        writer.write_all(&[0, 4, b'a', b'b']).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(5), receiver.receive(64))
                .await
                .is_err()
        );
        writer.write_all(b"cd").await.unwrap();
        assert_eq!(receiver.receive(64).await.unwrap().as_bytes(), b"abcd");
    }

    #[tokio::test]
    async fn full_header_zero_body_eof_is_truncated() {
        let (mut writer, reader) = tokio::io::duplex(16);
        writer.write_all(&3_u32.to_be_bytes()).await.unwrap();
        writer.shutdown().await.unwrap();
        let mut transport = FramedVsockTransport::new(reader);
        assert!(matches!(
            transport.receive(8).await,
            Err(TransportError::Truncated)
        ));
    }

    #[tokio::test]
    async fn cancelled_partial_send_resumes_the_same_frame() {
        let (left, right) = tokio::io::duplex(5);
        let mut sender = FramedVsockTransport::new(left);
        let mut receiver = FramedVsockTransport::new(right);
        let payload = b"partial-body".to_vec();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(5),
                sender.send(TransportPacket::new(payload.clone())),
            )
            .await
            .is_err()
        );
        let (sent, received) = tokio::join!(
            sender.send(TransportPacket::new(payload.clone())),
            receiver.receive(payload.len()),
        );
        sent.unwrap();
        assert_eq!(received.unwrap().as_bytes(), payload);
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
