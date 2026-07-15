use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

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
pub struct EndpointConnectionMetadata {
    pub operation_id: OperationId,
    pub handle_id: HandleId,
    pub binding_id: TransportBindingId,
    pub identity: Fingerprint,
    pub generation: Generation,
    pub kind: LocalTransportKind,
    pub capabilities: TransportCapabilityProfile,
    pub reachability: ReachabilityEvidence,
}

/// Resource owned by one successfully connected endpoint.
///
/// `close` must perform only nonblocking release work. The wrapper invokes it
/// exactly once, including when a connect result is rejected or cancelled
/// before the provider can publish a handle.
pub trait EndpointConnectionResource: Send + Sync {
    fn close(&self);
}

struct OwnedEndpointConnectionInner {
    resource: Arc<dyn EndpointConnectionResource>,
    closed: AtomicBool,
}

impl OwnedEndpointConnectionInner {
    fn close_once(&self) -> EndpointCloseState {
        if self
            .closed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.resource.close();
            EndpointCloseState::Closed
        } else {
            EndpointCloseState::AlreadyClosed
        }
    }
}

impl Drop for OwnedEndpointConnectionInner {
    fn drop(&mut self) {
        let _ = self.close_once();
    }
}

/// Cloneable RAII ownership token for a connected endpoint.
#[derive(Clone)]
pub struct OwnedEndpointConnection(Arc<OwnedEndpointConnectionInner>);

impl OwnedEndpointConnection {
    pub fn new(resource: Arc<dyn EndpointConnectionResource>) -> Self {
        Self(Arc::new(OwnedEndpointConnectionInner {
            resource,
            closed: AtomicBool::new(false),
        }))
    }

    pub fn close(&self) -> EndpointCloseState {
        self.0.close_once()
    }

    pub fn is_open(&self) -> bool {
        !self.0.closed.load(Ordering::Acquire)
    }
}

impl fmt::Debug for OwnedEndpointConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedEndpointConnection")
            .field("open", &self.is_open())
            .finish_non_exhaustive()
    }
}

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
    owned: OwnedEndpointConnection,
}

impl EndpointConnection {
    pub fn new(metadata: EndpointConnectionMetadata, owned: OwnedEndpointConnection) -> Self {
        Self {
            operation_id: metadata.operation_id,
            handle_id: metadata.handle_id,
            binding_id: metadata.binding_id,
            identity: metadata.identity,
            generation: metadata.generation,
            kind: metadata.kind,
            capabilities: metadata.capabilities,
            reachability: metadata.reachability,
            owned,
        }
    }

    pub fn owned(&self) -> &OwnedEndpointConnection {
        &self.owned
    }
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
        connection: &OwnedEndpointConnection,
    ) -> Result<EndpointObservation, EndpointPortError>;

    async fn close(
        &self,
        request: EndpointCloseRequest,
        connection: &OwnedEndpointConnection,
    ) -> Result<EndpointCloseResult, EndpointPortError>;
}
