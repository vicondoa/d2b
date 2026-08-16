//! Opaque effect-port contracts for native AF_VSOCK operations.

use crate::errors::VsockEffectError;
use async_trait::async_trait;
use std::{fmt, time::Instant};
use tokio::io::{AsyncRead, AsyncWrite};

/// Opaque endpoint resolution identity supplied by the child Zone core.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct OpaqueEndpointId(String);

impl OpaqueEndpointId {
    /// Parse one allocator-issued endpoint identity.
    pub fn parse(value: impl Into<String>) -> Result<Self, VsockEffectError> {
        let value = value.into();
        if valid_opaque_id(&value) {
            Ok(Self(value))
        } else {
            Err(VsockEffectError::EffectRejected)
        }
    }

    /// Construct an opaque identity at a trusted Core adapter boundary.
    pub fn from_core(value: impl Into<String>) -> Result<Self, VsockEffectError> {
        Self::parse(value)
    }

    /// Borrow the opaque value for the injected effect port.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OpaqueEndpointId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueEndpointId(<redacted>)")
    }
}

impl fmt::Display for OpaqueEndpointId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("opaque-endpoint")
    }
}

/// Opaque port-binding identity supplied by the child Zone core.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct OpaqueBindingId(String);

impl OpaqueBindingId {
    /// Parse one allocator-issued binding identity.
    pub fn parse(value: impl Into<String>) -> Result<Self, VsockEffectError> {
        let value = value.into();
        if valid_opaque_id(&value) {
            Ok(Self(value))
        } else {
            Err(VsockEffectError::EffectRejected)
        }
    }

    /// Construct an opaque identity at a trusted Core adapter boundary.
    pub fn from_core(value: impl Into<String>) -> Result<Self, VsockEffectError> {
        Self::parse(value)
    }

    /// Borrow the opaque value for the injected effect port.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OpaqueBindingId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueBindingId(<redacted>)")
    }
}

impl fmt::Display for OpaqueBindingId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("opaque-binding")
    }
}

/// The side of a ZoneLink transport that is being opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportRole {
    /// Connect to the selected parent route endpoint.
    Initiator,
    /// Accept from the selected parent route endpoint.
    Responder,
}

/// Injected native-vsock effect boundary.
#[async_trait]
pub trait VsockEffectPort: Send + Sync + 'static {
    /// The opaque byte stream returned by the Core adapter.
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    /// Open or accept one allocator-selected endpoint.
    async fn open(
        &self,
        endpoint_id: &OpaqueEndpointId,
        binding_id: &OpaqueBindingId,
        role: TransportRole,
        deadline: Instant,
    ) -> Result<Self::Stream, VsockEffectError>;

    /// Close one stream after the bridge has stopped.
    async fn close(&self, stream: Self::Stream) -> Result<(), VsockEffectError>;
}

fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
