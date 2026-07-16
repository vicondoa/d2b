use std::{
    fmt,
    os::fd::OwnedFd,
    sync::{Arc, Mutex},
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

/// Actual connected descriptor transferred once to `ComponentSession`.
pub struct OwnedLocalTransport {
    kind: LocalTransportKind,
    capabilities: TransportCapabilityProfile,
    descriptor: OwnedFd,
}

impl OwnedLocalTransport {
    pub fn from_connected(kind: LocalTransportKind, descriptor: OwnedFd) -> Self {
        Self {
            kind,
            capabilities: kind.capability_profile(),
            descriptor,
        }
    }

    pub const fn kind(&self) -> LocalTransportKind {
        self.kind
    }

    pub const fn capabilities(&self) -> TransportCapabilityProfile {
        self.capabilities
    }

    pub fn into_owned_fd(self) -> OwnedFd {
        self.descriptor
    }
}

impl fmt::Debug for OwnedLocalTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedLocalTransport")
            .field("kind", &self.kind)
            .field("capabilities", &self.capabilities)
            .field("descriptor", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportHandoffError {
    UnknownHandle,
    AlreadyClaimed,
    Closed,
}

impl fmt::Display for TransportHandoffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnknownHandle => "transport handoff handle unknown",
            Self::AlreadyClaimed => "transport handoff already claimed",
            Self::Closed => "transport handoff closed",
        })
    }
}

impl std::error::Error for TransportHandoffError {}

enum OwnedConnectionState {
    Available(OwnedLocalTransport),
    Claimed,
    Closed,
}

struct OwnedEndpointConnectionInner {
    state: Mutex<OwnedConnectionState>,
}

impl OwnedEndpointConnectionInner {
    fn close_once(&self) -> EndpointCloseState {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match std::mem::replace(&mut *state, OwnedConnectionState::Closed) {
            OwnedConnectionState::Available(transport) => {
                drop(transport);
                EndpointCloseState::Closed
            }
            OwnedConnectionState::Claimed | OwnedConnectionState::Closed => {
                EndpointCloseState::AlreadyClosed
            }
        }
    }

    fn take_transport(&self) -> Result<OwnedLocalTransport, TransportHandoffError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match std::mem::replace(&mut *state, OwnedConnectionState::Claimed) {
            OwnedConnectionState::Available(transport) => Ok(transport),
            OwnedConnectionState::Claimed => {
                *state = OwnedConnectionState::Claimed;
                Err(TransportHandoffError::AlreadyClaimed)
            }
            OwnedConnectionState::Closed => {
                *state = OwnedConnectionState::Closed;
                Err(TransportHandoffError::Closed)
            }
        }
    }

    fn is_open(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !matches!(&*state, OwnedConnectionState::Closed)
    }

    fn is_claimed(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        matches!(&*state, OwnedConnectionState::Claimed)
    }

    fn is_claimable(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        matches!(&*state, OwnedConnectionState::Available(_))
    }
}

impl fmt::Debug for OwnedEndpointConnectionInner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedEndpointConnectionInner")
            .field("open", &self.is_open())
            .field("claimed", &self.is_claimed())
            .finish_non_exhaustive()
    }
}

/// RAII ownership token for a connected endpoint.
pub struct OwnedEndpointConnection(Arc<OwnedEndpointConnectionInner>);

impl OwnedEndpointConnection {
    fn new(transport: OwnedLocalTransport) -> Self {
        Self(Arc::new(OwnedEndpointConnectionInner {
            state: Mutex::new(OwnedConnectionState::Available(transport)),
        }))
    }

    pub fn close(&self) -> EndpointCloseState {
        self.0.close_once()
    }

    pub fn is_open(&self) -> bool {
        self.0.is_open()
    }

    pub(crate) fn take_transport(&self) -> Result<OwnedLocalTransport, TransportHandoffError> {
        self.0.take_transport()
    }

    pub(crate) fn is_claimable(&self) -> bool {
        self.0.is_claimable()
    }

    fn share(&self) -> Self {
        Self(self.0.clone())
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
    pub fn new(
        metadata: EndpointConnectionMetadata,
        transport: OwnedLocalTransport,
    ) -> Result<Self, EndpointPortError> {
        if metadata.kind != transport.kind() || metadata.capabilities != transport.capabilities() {
            return Err(EndpointPortError::InvariantViolation);
        }
        Ok(Self {
            operation_id: metadata.operation_id,
            handle_id: metadata.handle_id,
            binding_id: metadata.binding_id,
            identity: metadata.identity,
            generation: metadata.generation,
            kind: metadata.kind,
            capabilities: metadata.capabilities,
            reachability: metadata.reachability,
            owned: OwnedEndpointConnection::new(transport),
        })
    }

    pub fn owned(&self) -> &OwnedEndpointConnection {
        &self.owned
    }

    pub(crate) fn snapshot(&self) -> Self {
        Self {
            operation_id: self.operation_id.clone(),
            handle_id: self.handle_id.clone(),
            binding_id: self.binding_id.clone(),
            identity: self.identity.clone(),
            generation: self.generation,
            kind: self.kind,
            capabilities: self.capabilities,
            reachability: self.reachability,
            owned: self.owned.share(),
        }
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
