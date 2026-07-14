//! Compatibility seam for the pending `d2b-session` provider transport API.
//! Once that crate exports these types, this module becomes direct re-exports
//! and the server implementation remains unchanged.

use std::{error::Error, fmt};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{EndpointRole, ServicePackage},
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{AuthorizedProviderScope, Generation, PrincipalRef, ProviderMethod},
};
use d2b_provider::CancellationToken;
#[derive(Clone, PartialEq, Eq)]
pub struct AuthenticatedSessionState {
    pub local_provider_id: ProviderId,
    pub local_provider_type: ProviderType,
    pub local_provider_generation: Generation,
    pub local_role: EndpointRole,
    pub peer_role: EndpointRole,
    pub service: ServicePackage,
    pub session_generation: u64,
    pub principal: PrincipalRef,
    pub authorized_scope: AuthorizedProviderScope,
}

impl fmt::Debug for AuthenticatedSessionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedSessionState")
            .field("local_provider_type", &self.local_provider_type)
            .field("local_provider_generation", &self.local_provider_generation)
            .field("local_role", &self.local_role)
            .field("peer_role", &self.peer_role)
            .field("service", &self.service)
            .field("session_generation", &self.session_generation)
            .finish_non_exhaustive()
    }
}

pub struct OwnedAttachment {
    index: u32,
    payload: Vec<u8>,
}

impl OwnedAttachment {
    pub fn new(index: u32, payload: Vec<u8>) -> Self {
        Self { index, payload }
    }

    pub const fn index(&self) -> u32 {
        self.index
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

impl fmt::Debug for OwnedAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedAttachment")
            .field("index", &self.index)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedProviderMethod {
    Health(ProviderType),
    Capabilities(ProviderType),
    Invoke(ProviderMethod),
}

pub struct TransportPacket {
    pub request_id: [u8; 16],
    pub method: ClosedProviderMethod,
    pub payload: Vec<u8>,
    pub attachments: Vec<OwnedAttachment>,
}

impl fmt::Debug for TransportPacket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransportPacket")
            .field("method", &self.method)
            .field("payload_bytes", &self.payload.len())
            .field("attachment_count", &self.attachments.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDriverError {
    Disconnected,
    Unauthenticated,
    GenerationMismatch,
    AttachmentMismatch,
    Cancelled,
    Protocol,
}

impl fmt::Display for SessionDriverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Disconnected => "component session disconnected",
            Self::Unauthenticated => "component session is not authenticated",
            Self::GenerationMismatch => "component session generation mismatch",
            Self::AttachmentMismatch => "component session attachment mismatch",
            Self::Cancelled => "component session request cancelled",
            Self::Protocol => "component session protocol violation",
        })
    }
}

impl Error for SessionDriverError {}

#[async_trait]
pub trait ComponentSessionDriver: Send + Sync {
    fn authenticated_state(&self) -> Result<AuthenticatedSessionState, SessionDriverError>;

    fn cancellation(&self, request_id: [u8; 16]) -> CancellationToken;

    fn monotonic_remaining_nanos(&self, request_id: [u8; 16]) -> Result<u64, SessionDriverError>;

    async fn take_attachments(
        &self,
        request_id: [u8; 16],
        indexes: &[u32],
    ) -> Result<Vec<OwnedAttachment>, SessionDriverError>;

    async fn receive_packet(&self) -> Result<TransportPacket, SessionDriverError>;

    async fn send_packet(&self, packet: TransportPacket) -> Result<(), SessionDriverError>;
}
