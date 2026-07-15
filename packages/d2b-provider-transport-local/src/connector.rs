use std::{fmt, time::Duration};

use async_trait::async_trait;
use d2b_contracts::v2_provider::{
    Fingerprint, Generation, HandleId, OperationId, TransportBindingId,
};

use crate::{EndpointSource, LocalTransportKind, TransportCapabilityProfile};

/// Evidence returned by a local connector. It never denotes authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReachabilityEvidence {
    ReachableOnly,
}

/// Closed request passed to the injected endpoint port.
#[derive(Clone)]
pub struct EndpointConnectRequest {
    pub operation_id: OperationId,
    pub handle_id: HandleId,
    pub binding_id: TransportBindingId,
    pub endpoint: EndpointSource,
    pub expected_identity: Fingerprint,
    pub expected_generation: Generation,
    pub kind: LocalTransportKind,
    pub capabilities: TransportCapabilityProfile,
    pub deadline: Duration,
}

impl fmt::Debug for EndpointConnectRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointConnectRequest")
            .field("kind", &self.kind)
            .field("capabilities", &self.capabilities)
            .field("endpoint", &self.endpoint)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

/// Successful connection evidence retained by the provider.
#[derive(Clone)]
pub struct EndpointConnection {
    pub operation_id: OperationId,
    pub handle_id: HandleId,
    pub binding_id: TransportBindingId,
    pub identity: Fingerprint,
    pub generation: Generation,
    pub kind: LocalTransportKind,
    pub capabilities: TransportCapabilityProfile,
    pub reachability: ReachabilityEvidence,
}

impl fmt::Debug for EndpointConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointConnection")
            .field("generation", &self.generation)
            .field("kind", &self.kind)
            .field("capabilities", &self.capabilities)
            .field("reachability", &self.reachability)
            .finish_non_exhaustive()
    }
}

/// Closed inspection request for one provider-owned connection.
#[derive(Clone)]
pub struct EndpointInspectRequest {
    pub operation_id: OperationId,
    pub handle_id: HandleId,
    pub binding_id: TransportBindingId,
    pub expected_identity: Fingerprint,
    pub expected_generation: Generation,
    pub kind: LocalTransportKind,
    pub capabilities: TransportCapabilityProfile,
    pub deadline: Duration,
}

impl fmt::Debug for EndpointInspectRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointInspectRequest")
            .field("kind", &self.kind)
            .field("capabilities", &self.capabilities)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointObservationState {
    Connected,
    Closed,
    Unavailable,
}

/// Bounded endpoint observation. Identity and generation are always echoed so
/// the provider can reject stale or substituted resources.
#[derive(Clone)]
pub struct EndpointObservation {
    pub operation_id: OperationId,
    pub handle_id: HandleId,
    pub binding_id: TransportBindingId,
    pub identity: Fingerprint,
    pub generation: Generation,
    pub kind: LocalTransportKind,
    pub capabilities: TransportCapabilityProfile,
    pub state: EndpointObservationState,
}

impl fmt::Debug for EndpointObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointObservation")
            .field("generation", &self.generation)
            .field("kind", &self.kind)
            .field("capabilities", &self.capabilities)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// Closed request to release one endpoint connection.
#[derive(Clone)]
pub struct EndpointCloseRequest {
    pub operation_id: OperationId,
    pub handle_id: HandleId,
    pub binding_id: TransportBindingId,
    pub expected_identity: Fingerprint,
    pub expected_generation: Generation,
    pub kind: LocalTransportKind,
    pub deadline: Duration,
}

impl fmt::Debug for EndpointCloseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointCloseRequest")
            .field("kind", &self.kind)
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointCloseState {
    Closed,
    AlreadyClosed,
}

#[derive(Clone)]
pub struct EndpointCloseResult {
    pub operation_id: OperationId,
    pub handle_id: HandleId,
    pub binding_id: TransportBindingId,
    pub identity: Fingerprint,
    pub generation: Generation,
    pub state: EndpointCloseState,
}

impl fmt::Debug for EndpointCloseResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndpointCloseResult")
            .field("generation", &self.generation)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// Closed endpoint-port failure classes. Authentication failures are
/// intentionally absent: authentication belongs to `ComponentSession`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointPortError {
    Cancelled,
    DeadlineExpired,
    Unavailable,
    BoundExceeded,
    IdentityMismatch,
    GenerationMismatch,
    InvariantViolation,
}

impl fmt::Display for EndpointPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Cancelled => "endpoint operation cancelled",
            Self::DeadlineExpired => "endpoint operation deadline expired",
            Self::Unavailable => "endpoint unavailable",
            Self::BoundExceeded => "endpoint bound exceeded",
            Self::IdentityMismatch => "endpoint identity mismatch",
            Self::GenerationMismatch => "endpoint generation mismatch",
            Self::InvariantViolation => "endpoint invariant violation",
        })
    }
}

impl std::error::Error for EndpointPortError {}

/// Asynchronous endpoint seam implemented by the owner of pre-opened sockets,
/// bundle resolution, or allocator leases.
///
/// Implementations must be cancellation-safe when their future is dropped and
/// must not treat successful connection as d2b authentication or authorization.
#[async_trait]
pub trait LocalEndpointPort: Send + Sync {
    async fn connect(
        &self,
        request: EndpointConnectRequest,
    ) -> Result<EndpointConnection, EndpointPortError>;

    async fn inspect(
        &self,
        request: EndpointInspectRequest,
    ) -> Result<EndpointObservation, EndpointPortError>;

    async fn close(
        &self,
        request: EndpointCloseRequest,
    ) -> Result<EndpointCloseResult, EndpointPortError>;
}
