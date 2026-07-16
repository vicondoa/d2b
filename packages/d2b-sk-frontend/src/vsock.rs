//! Native-vsock transport adapter for ComponentSession.

use std::{net::Shutdown, time::Duration};

use async_trait::async_trait;
use d2b_provider_transport_local::LocalTransportKind;
use d2b_session::{OwnedTransport, TransportDescriptor, TransportError, TransportPacket};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_vsock::{VsockAddr, VsockStream};

pub const VSOCK_HOST_CID: u32 = 2;
pub const SK_VSOCK_PORT: u32 = 14320;

#[derive(Debug, Clone, Copy)]
pub struct BackoffParams {
    pub initial: Duration,
    pub max: Duration,
    pub factor: u32,
}

impl Default for BackoffParams {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(60),
            factor: 2,
        }
    }
}

impl BackoffParams {
    fn next(self, current: Duration) -> Duration {
        current
            .checked_mul(self.factor)
            .unwrap_or(self.max)
            .min(self.max)
    }
}

pub struct VsockTransport {
    stream: VsockStream,
}

impl VsockTransport {
    pub fn new(stream: VsockStream) -> Self {
        Self { stream }
    }

    async fn receive_packet(
        &mut self,
        protected_limit: usize,
    ) -> Result<TransportPacket, TransportError> {
        let mut length = [0; 4];
        self.stream
            .read_exact(&mut length)
            .await
            .map_err(map_read_error)?;
        let length = usize::try_from(u32::from_be_bytes(length))
            .map_err(|_| TransportError::LimitExceeded)?;
        if length == 0 || length > protected_limit {
            return Err(TransportError::LimitExceeded);
        }
        let mut bytes = vec![0; length];
        self.stream
            .read_exact(&mut bytes)
            .await
            .map_err(map_read_error)?;
        Ok(TransportPacket::new(bytes))
    }

    async fn send_packet(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        if !packet.attachments().is_empty() {
            return Err(TransportError::InvalidAttachment);
        }
        let (bytes, _) = packet.into_parts();
        let length = u32::try_from(bytes.len()).map_err(|_| TransportError::LimitExceeded)?;
        if length == 0 {
            return Err(TransportError::LimitExceeded);
        }
        self.stream
            .write_all(&length.to_be_bytes())
            .await
            .map_err(map_write_error)?;
        self.stream
            .write_all(&bytes)
            .await
            .map_err(map_write_error)?;
        self.stream.flush().await.map_err(map_write_error)
    }
}

#[async_trait]
impl OwnedTransport for VsockTransport {
    fn descriptor(&self) -> TransportDescriptor {
        let capability = LocalTransportKind::NativeVsock.capability_profile();
        TransportDescriptor {
            class: capability.transport_class,
            locality: capability.locality,
            packet_atomic: capability.packet_atomic,
            supports_attachments: false,
        }
    }

    async fn receive(&mut self, protected_limit: usize) -> Result<TransportPacket, TransportError> {
        self.receive_packet(protected_limit).await
    }

    async fn send(&mut self, packet: TransportPacket) -> Result<(), TransportError> {
        self.send_packet(packet).await
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        self.stream
            .shutdown(Shutdown::Both)
            .map_err(map_write_error)
    }
}

pub async fn connect_with_backoff(port: u32, params: BackoffParams) -> VsockTransport {
    let mut wait = params.initial;
    loop {
        tokio::time::sleep(wait).await;
        match VsockStream::connect(VsockAddr::new(VSOCK_HOST_CID, port)).await {
            Ok(stream) => return VsockTransport::new(stream),
            Err(_) => {
                eprintln!("[d2b-sk-frontend] transport-unavailable");
                wait = params.next(wait);
            }
        }
    }
}

fn map_read_error(error: std::io::Error) -> TransportError {
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof => TransportError::Truncated,
        std::io::ErrorKind::WouldBlock => TransportError::WouldBlock,
        std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::NotConnected => TransportError::Disconnected,
        _ => TransportError::Other,
    }
}

fn map_write_error(error: std::io::Error) -> TransportError {
    match error.kind() {
        std::io::ErrorKind::WouldBlock => TransportError::WouldBlock,
        std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::NotConnected => TransportError::Disconnected,
        _ => TransportError::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v2_component_session::{Locality, TransportClass};

    #[test]
    fn backoff_is_integer_bounded() {
        let params = BackoffParams {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(10),
            factor: 4,
        };
        assert_eq!(params.next(Duration::from_secs(1)), Duration::from_secs(4));
        assert_eq!(params.next(Duration::from_secs(4)), Duration::from_secs(10));
        assert_eq!(params.next(Duration::MAX), Duration::from_secs(10));
    }

    #[test]
    fn native_vsock_contract_disables_attachments() {
        let capability = LocalTransportKind::NativeVsock.capability_profile();
        assert_eq!(capability.transport_class, TransportClass::NativeVsock);
        assert_eq!(capability.locality, Locality::GuestLocal);
        assert!(!capability.packet_atomic);
    }
}
