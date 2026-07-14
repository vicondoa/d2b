use std::{error::Error, fmt};

use async_trait::async_trait;
use d2b_contracts::v2_component_session::{Locality, TransportClass};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportDescriptor {
    pub class: TransportClass,
    pub locality: Locality,
    pub packet_atomic: bool,
    pub supports_attachments: bool,
}

pub struct TransportPacket {
    bytes: Vec<u8>,
}

impl TransportPacket {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl fmt::Debug for TransportPacket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransportPacket")
            .field("bytes", &"<redacted>")
            .field("len", &self.bytes.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    Disconnected,
    WouldBlock,
    Truncated,
    LimitExceeded,
    InvalidAttachment,
    Other,
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Disconnected => "transport-disconnected",
            Self::WouldBlock => "transport-would-block",
            Self::Truncated => "transport-truncated",
            Self::LimitExceeded => "transport-limit-exceeded",
            Self::InvalidAttachment => "transport-invalid-attachment",
            Self::Other => "transport-error",
        })
    }
}

impl Error for TransportError {}

#[async_trait]
pub trait OwnedTransport: Send {
    fn descriptor(&self) -> TransportDescriptor;

    async fn receive(
        &mut self,
        protected_limit: usize,
    ) -> std::result::Result<TransportPacket, TransportError>;

    async fn send(&mut self, packet: TransportPacket) -> std::result::Result<(), TransportError>;

    async fn close(&mut self) -> std::result::Result<(), TransportError>;
}
