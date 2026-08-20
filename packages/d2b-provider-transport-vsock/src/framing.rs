//! Bounded length-prefixed byte framing for vsock streams.

use crate::{TransportError, limits::MAX_FRAME_BYTES};
use async_trait::async_trait;
use d2b_contracts_zone_session::v3::component_session::{Locality, TransportClass};
use d2b_session::{
    Cancellation, OwnedTransport, TransportDescriptor, TransportPacket, TransportReader,
    TransportWriter,
};
use std::fmt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const FRAME_HEADER_BYTES: usize = 2;

/// Provider-facing descriptor for a native vsock transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VsockTransportDescriptor {
    /// The ComponentSession transport class.
    pub class: TransportClass,
    /// The endpoint locality.
    pub locality: Locality,
    /// Whether a frame is packet atomic.
    pub packet_atomic: bool,
    /// Whether descriptor attachments are supported.
    pub supports_attachments: bool,
}

impl Default for VsockTransportDescriptor {
    fn default() -> Self {
        Self {
            class: TransportClass::NativeVsock,
            locality: Locality::GuestLocal,
            packet_atomic: false,
            supports_attachments: false,
        }
    }
}

/// A bounded two-byte length-prefixed transport.
pub struct FramedVsockTransport<S> {
    stream: S,
    max_frame_bytes: usize,
    closed: bool,
}

impl<S> FramedVsockTransport<S> {
    /// Wrap a stream with the ComponentSession maximum frame size.
    pub fn new(stream: S) -> Self {
        Self::with_limit(stream, MAX_FRAME_BYTES)
    }

    /// Wrap a stream with a smaller test or process-local frame bound.
    pub fn with_limit(stream: S, max_frame_bytes: usize) -> Self {
        Self {
            stream,
            max_frame_bytes: max_frame_bytes.min(MAX_FRAME_BYTES),
            closed: false,
        }
    }

    /// Return the provider-facing descriptor.
    pub const fn vsock_descriptor(&self) -> VsockTransportDescriptor {
        VsockTransportDescriptor {
            class: TransportClass::NativeVsock,
            locality: Locality::GuestLocal,
            packet_atomic: false,
            supports_attachments: false,
        }
    }

    /// Read one complete framed record.
    pub async fn read_frame(&mut self) -> Result<Vec<u8>, TransportError>
    where
        S: AsyncRead + Unpin,
    {
        if self.closed {
            return Err(TransportError::Closed);
        }
        let mut header = [0_u8; FRAME_HEADER_BYTES];
        read_exact_classified(&mut self.stream, &mut header, false).await?;
        let declared = usize::from(u16::from_be_bytes(header));
        if declared == 0 {
            self.closed = true;
            return Err(TransportError::InvalidFrame);
        }
        if declared > self.max_frame_bytes {
            self.closed = true;
            return Err(TransportError::FrameTooLarge);
        }
        let mut body = vec![0_u8; declared];
        read_exact_classified(&mut self.stream, &mut body, true).await?;
        Ok(body)
    }

    /// Write one complete framed record.
    pub async fn write_frame(&mut self, bytes: &[u8]) -> Result<(), TransportError>
    where
        S: AsyncWrite + Unpin,
    {
        if self.closed {
            return Err(TransportError::Closed);
        }
        if bytes.is_empty() {
            return Err(TransportError::InvalidFrame);
        }
        if bytes.len() > self.max_frame_bytes || bytes.len() > u16::MAX as usize {
            return Err(TransportError::FrameTooLarge);
        }
        let length = u16::try_from(bytes.len()).map_err(|_| TransportError::FrameTooLarge)?;
        self.stream
            .write_all(&length.to_be_bytes())
            .await
            .map_err(|_| {
                self.closed = true;
                TransportError::Io
            })?;
        self.stream.write_all(bytes).await.map_err(|_| {
            self.closed = true;
            TransportError::Io
        })?;
        self.stream.flush().await.map_err(|_| {
            self.closed = true;
            TransportError::Io
        })
    }

    /// Consume the transport and return its stream.
    pub fn into_inner(self) -> S {
        self.stream
    }
}

impl<S> fmt::Debug for FramedVsockTransport<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FramedVsockTransport")
            .field("max_frame_bytes", &self.max_frame_bytes)
            .field("closed", &self.closed)
            .finish()
    }
}

#[async_trait]
impl<S> OwnedTransport for FramedVsockTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    fn descriptor(&self) -> TransportDescriptor {
        let descriptor = self.vsock_descriptor();
        TransportDescriptor {
            class: descriptor.class,
            locality: descriptor.locality,
            packet_atomic: descriptor.packet_atomic,
            supports_attachments: descriptor.supports_attachments,
        }
    }

    fn into_split(self: Box<Self>) -> (Box<dyn TransportReader>, Box<dyn TransportWriter>) {
        let Self {
            stream,
            max_frame_bytes,
            closed,
        } = *self;
        let (reader, writer) = tokio::io::split(stream);
        (
            Box::new(VsockReader {
                stream: reader,
                max_frame_bytes,
                closed,
            }),
            Box::new(VsockWriter {
                stream: writer,
                max_frame_bytes,
                closed,
            }),
        )
    }

    fn set_write_cancellation(&mut self, _cancellation: Option<Cancellation>) {}

    async fn receive(
        &mut self,
        protected_limit: usize,
    ) -> Result<TransportPacket, d2b_session::TransportError> {
        let limit = protected_limit.min(self.max_frame_bytes);
        self.read_frame()
            .await
            .map(|bytes| {
                if bytes.len() > limit {
                    return Err(d2b_session::TransportError::LimitExceeded);
                }
                Ok(TransportPacket::new(bytes))
            })
            .unwrap_or_else(|error| Err(map_session_error(error)))
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), d2b_session::TransportError> {
        let (bytes, attachments) = packet.into_parts();
        if !attachments.is_empty() {
            return Err(d2b_session::TransportError::InvalidAttachment);
        }
        self.write_frame(&bytes).await.map_err(map_session_error)
    }

    async fn close(&mut self) -> Result<(), d2b_session::TransportError> {
        if !self.closed {
            self.closed = true;
            self.stream
                .shutdown()
                .await
                .map_err(|_| d2b_session::TransportError::Disconnected)?;
        }
        Ok(())
    }
}

struct VsockReader<R> {
    stream: R,
    max_frame_bytes: usize,
    closed: bool,
}

#[async_trait]
impl<R> TransportReader for VsockReader<R>
where
    R: AsyncRead + Unpin + Send,
{
    async fn receive(
        &mut self,
        protected_limit: usize,
    ) -> Result<TransportPacket, d2b_session::TransportError> {
        if self.closed {
            return Err(d2b_session::TransportError::Disconnected);
        }
        let mut header = [0_u8; FRAME_HEADER_BYTES];
        read_exact_classified(&mut self.stream, &mut header, false)
            .await
            .map_err(map_session_error)?;
        let declared = usize::from(u16::from_be_bytes(header));
        if declared == 0 || declared > self.max_frame_bytes || declared > protected_limit {
            self.closed = true;
            return Err(d2b_session::TransportError::LimitExceeded);
        }
        let mut body = vec![0_u8; declared];
        read_exact_classified(&mut self.stream, &mut body, true)
            .await
            .map_err(map_session_error)?;
        Ok(TransportPacket::new(body))
    }
}

struct VsockWriter<W> {
    stream: W,
    max_frame_bytes: usize,
    closed: bool,
}

#[async_trait]
impl<W> TransportWriter for VsockWriter<W>
where
    W: AsyncWrite + Unpin + Send,
{
    async fn send(&mut self, packet: TransportPacket) -> Result<(), d2b_session::TransportError> {
        if self.closed {
            return Err(d2b_session::TransportError::Disconnected);
        }
        let (bytes, attachments) = packet.into_parts();
        if !attachments.is_empty() {
            return Err(d2b_session::TransportError::InvalidAttachment);
        }
        if bytes.is_empty() || bytes.len() > self.max_frame_bytes {
            return Err(d2b_session::TransportError::LimitExceeded);
        }
        let length =
            u16::try_from(bytes.len()).map_err(|_| d2b_session::TransportError::LimitExceeded)?;
        self.stream
            .write_all(&length.to_be_bytes())
            .await
            .map_err(|_| d2b_session::TransportError::Disconnected)?;
        self.stream
            .write_all(&bytes)
            .await
            .map_err(|_| d2b_session::TransportError::Disconnected)?;
        self.stream
            .flush()
            .await
            .map_err(|_| d2b_session::TransportError::Disconnected)
    }

    async fn close(&mut self) -> Result<(), d2b_session::TransportError> {
        if !self.closed {
            self.closed = true;
            self.stream
                .shutdown()
                .await
                .map_err(|_| d2b_session::TransportError::Disconnected)?;
        }
        Ok(())
    }
}

async fn read_exact_classified<R: AsyncRead + Unpin>(
    stream: &mut R,
    output: &mut [u8],
    frame_started: bool,
) -> Result<(), TransportError> {
    let mut offset = 0;
    while offset < output.len() {
        let read = stream.read(&mut output[offset..]).await.map_err(|_| {
            if frame_started {
                TransportError::Truncated
            } else {
                TransportError::Io
            }
        })?;
        if read == 0 {
            return Err(if offset == 0 && !frame_started {
                TransportError::Disconnected
            } else {
                TransportError::Truncated
            });
        }
        offset += read;
    }
    Ok(())
}

fn map_session_error(error: TransportError) -> d2b_session::TransportError {
    match error {
        TransportError::Closed
        | TransportError::Disconnected
        | TransportError::Truncated
        | TransportError::Io => d2b_session::TransportError::Disconnected,
        TransportError::FrameTooLarge | TransportError::InvalidFrame => {
            d2b_session::TransportError::LimitExceeded
        }
        TransportError::AttachmentsNotSupported => d2b_session::TransportError::InvalidAttachment,
    }
}
