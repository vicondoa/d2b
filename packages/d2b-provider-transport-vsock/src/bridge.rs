//! Named-stream bridge lifecycle.

use crate::graph_binding::AdmittedTransportRoute;
use async_trait::async_trait;
use d2b_contracts_resource::v3::BindingKey;
use std::error::Error;
use std::sync::atomic::AtomicBool;
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::{
    io::{AsyncRead, AsyncWrite, copy_bidirectional},
    sync::{Notify, watch},
};

/// Opaque named stream identity returned to the child core.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NamedStreamId(u64);

impl NamedStreamId {
    /// Construct a stream identity at the ComponentSession boundary.
    pub const fn from_core(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Debug for NamedStreamId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NamedStreamId(<redacted>)")
    }
}

/// Opaque transport handle owned by one Provider service.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportHandle(u64);

impl TransportHandle {
    /// Construct a handle at a trusted test or Core boundary.
    pub const fn from_core(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Debug for TransportHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TransportHandle(<redacted>)")
    }
}

/// Port used by the Provider to create and close ComponentSession named
/// streams. It carries no raw file descriptor or socket path.
#[async_trait]
pub trait NamedStreamPort: Send + Sync + 'static {
    /// The byte stream connected to the ComponentSession named stream.
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    /// Open one named stream.
    ///
    /// # Errors
    ///
    /// Returns [`NamedStreamError::Capacity`] when the stream table is
    /// full and [`NamedStreamError::Disconnected`] when the session is no
    /// longer available.
    async fn open_named_stream(&self) -> Result<(NamedStreamId, Self::Stream), NamedStreamError>;

    /// Close one named stream.
    ///
    /// # Errors
    ///
    /// Returns [`NamedStreamError::Disconnected`] when the session is no
    /// longer available.
    async fn close_named_stream(&self, stream: NamedStreamId) -> Result<(), NamedStreamError>;
}

/// Named-stream allocation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedStreamError {
    /// The ComponentSession stream table is full.
    Capacity,
    /// The session is no longer available.
    Disconnected,
}

impl fmt::Display for NamedStreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Capacity => "named-stream-capacity",
            Self::Disconnected => "named-stream-disconnected",
        })
    }
}

impl std::error::Error for NamedStreamError {}

/// Closed bridge completion reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeExit {
    /// The peer closed the byte stream.
    PeerClosed,
    /// The owner requested closure.
    OwnerClosed,
    /// The bridge encountered an I/O error.
    IoError,
}

/// Bounded bridge counters.
#[derive(Debug, Default)]
pub struct BridgeStats {
    bytes_from_vsock: AtomicU64,
    bytes_to_vsock: AtomicU64,
}

impl BridgeStats {
    /// Return bytes received from the vsock side.
    pub fn bytes_from_vsock(&self) -> u64 {
        self.bytes_from_vsock.load(Ordering::Relaxed)
    }

    /// Return bytes sent to the vsock side.
    pub fn bytes_to_vsock(&self) -> u64 {
        self.bytes_to_vsock.load(Ordering::Relaxed)
    }

    fn record(&self, from_vsock: u64, to_vsock: u64) {
        self.bytes_from_vsock
            .fetch_add(from_vsock, Ordering::Relaxed);
        self.bytes_to_vsock.fetch_add(to_vsock, Ordering::Relaxed);
    }
}

/// A cancel signal and completion notification for one bridge task.
#[derive(Clone)]
pub struct BridgeControl {
    stop: watch::Sender<bool>,
    completed: Arc<Notify>,
    done: Arc<AtomicBool>,
}

impl BridgeControl {
    /// Create a bridge control pair.
    pub fn new() -> (Self, watch::Receiver<bool>) {
        let (stop, receiver) = watch::channel(false);
        (
            Self {
                stop,
                completed: Arc::new(Notify::new()),
                done: Arc::new(AtomicBool::new(false)),
            },
            receiver,
        )
    }

    /// Request bridge shutdown.
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }

    /// Mark the bridge as finished.
    pub(crate) fn mark_completed(&self) {
        self.done.store(true, Ordering::Release);
        self.completed.notify_waiters();
    }

    /// Wait for the bridge task to finish.
    pub async fn wait(&self) {
        if self.done.load(Ordering::Acquire) {
            return;
        }
        let notified = self.completed.notified();
        if self.done.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

/// Run a byte bridge until one side closes or the owner requests shutdown.
pub async fn run_bridge<L, R>(
    mut left: L,
    mut right: R,
    mut stop: watch::Receiver<bool>,
    stats: Arc<BridgeStats>,
    endpoint_id: &crate::service::OpaqueEndpointId,
    binding_id: &crate::service::OpaqueBindingId,
) -> (L, R, BridgeExit)
where
    L: AsyncRead + AsyncWrite + Unpin,
    R: AsyncRead + AsyncWrite + Unpin,
{
    let result = tokio::select! {
        copied = copy_bidirectional(&mut left, &mut right) => {
            match copied {
                Ok((from_left, from_right)) => {
                    stats.record(from_left, from_right);
                    BridgeExit::PeerClosed
                }
                Err(_) => {
                    tracing::debug!(
                        provider = "transport-vsock",
                        endpoint = %endpoint_id,
                        binding = %binding_id,
                        "bridge copy failed with an IO error"
                    );
                    BridgeExit::IoError
                }
            }
        }
        changed = stop.changed() => {
            if changed.is_ok() && *stop.borrow() {
                BridgeExit::OwnerClosed
            } else {
                BridgeExit::IoError
            }
        }
    };
    (left, right, result)
}

/// The closed set of privileged transport control operations.
///
/// Every one is an in-process call into the service surface. None is a frame,
/// and none is reachable from the bytes a named stream carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportControlOperation {
    /// Open a transport on an already admitted route.
    Open,
    /// Close a transport on an already admitted route.
    Close,
    /// Observe a transport on an already admitted route.
    Observe,
}

/// What stream-carried bytes turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarriageClass {
    /// The bytes name no control operation: they are carriage.
    Data,
    /// The bytes name a control operation but carry no authority.
    ControlShaped,
}

const OPEN_DISCRIMINANT: &[u8] = b"open";
const CLOSE_DISCRIMINANT: &[u8] = b"close";
const OBSERVE_DISCRIMINANT: &[u8] = b"observe";

fn scan_for_operation(bytes: &[u8]) -> Option<TransportControlOperation> {
    let candidates = [
        (OPEN_DISCRIMINANT, TransportControlOperation::Open),
        (CLOSE_DISCRIMINANT, TransportControlOperation::Close),
        (OBSERVE_DISCRIMINANT, TransportControlOperation::Observe),
    ];
    for (name, operation) in candidates {
        if bytes.windows(name.len()).any(|window| window == name) {
            return Some(operation);
        }
    }
    None
}

/// Classify carried bytes by scanning them for a control discriminant.
///
/// The scan is the whole point: carriage that names a privileged operation is
/// recognisable as such and is still refused, because recognisable is not
/// the same as authorized. The classification never admits anything.
pub fn classify_carriage(bytes: &[u8]) -> CarriageClass {
    if scan_for_operation(bytes).is_some() {
        CarriageClass::ControlShaped
    } else {
        CarriageClass::Data
    }
}

/// Why stream-carried bytes could not become a control-plane request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPlaneInjectionRefusal {
    /// Carriage carries no operation discriminant: it is data by construction.
    NoOperationDiscriminant,
    /// The bytes are shaped like a control request but carry no route token.
    NotAControlRequest,
    /// No admitted route backs this attempt.
    RouteNotAdmitted,
}

impl ControlPlaneInjectionRefusal {
    /// Return the closed, stable refusal code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NoOperationDiscriminant => "no-operation-discriminant",
            Self::NotAControlRequest => "not-a-control-request",
            Self::RouteNotAdmitted => "route-not-admitted",
        }
    }
}

impl fmt::Display for ControlPlaneInjectionRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ControlPlaneInjectionRefusal {}

/// The opaque authorization a privileged control request must carry.
///
/// It is derived from a live admitted route and deliberately has no public
/// constructor, no accessor, no `Clone`, and no deserializer: the only way to
/// hold one is to be holding an admitted route, and the only way to use it is
/// to spend it on one in-process call. The bound relationship is retained
/// rather than read, because reading it would turn the token into evidence
/// about a route the holder does not own.
pub struct ControlRouteToken {
    _route: BindingKey,
}

impl ControlRouteToken {
    pub(crate) fn for_route(route: BindingKey) -> Self {
        Self { _route: route }
    }
}

impl fmt::Debug for ControlRouteToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ControlRouteToken(OPAQUE)")
    }
}

/// A privileged control request.
///
/// Constructible only against a live admitted route, because issuing one
/// requires the route token that only
/// [`AdmittedTransportRoute::control_token`] can mint and that this type
/// consumes.
#[derive(Debug)]
pub struct ControlPlaneRequest {
    operation: TransportControlOperation,
    _token: ControlRouteToken,
    handle: TransportHandle,
}

impl ControlPlaneRequest {
    /// Issue one privileged control request against a live route.
    pub const fn issue(
        operation: TransportControlOperation,
        token: ControlRouteToken,
        handle: TransportHandle,
    ) -> Self {
        Self {
            operation,
            _token: token,
            handle,
        }
    }

    /// Return the operation the request performs.
    pub const fn operation(&self) -> TransportControlOperation {
        self.operation
    }

    /// Return the transport handle the request targets.
    pub const fn handle(&self) -> TransportHandle {
        self.handle
    }

    /// The one function stream-carried bytes could reach if a transport ever
    /// parsed its data plane for control.
    ///
    /// It scans `bytes` for a control-operation discriminant and always
    /// refuses: carriage can name an operation, but it cannot carry the route
    /// token that would make the operation privileged, and no byte sequence is
    /// a deserializer for [`ControlRouteToken`]. This is the only plausible
    /// injection seam on the bridge, and it is closed.
    ///
    /// # Errors
    ///
    /// Returns `RouteNotAdmitted` when no admitted route backs the attempt,
    /// `NoOperationDiscriminant` when the bytes name no control operation, and
    /// `NotAControlRequest` when they do name one without a route token.
    pub fn from_carriage(
        route: Option<&AdmittedTransportRoute>,
        bytes: &[u8],
    ) -> Result<Self, ControlPlaneInjectionRefusal> {
        let Some(_route) = route else {
            return Err(ControlPlaneInjectionRefusal::RouteNotAdmitted);
        };
        if scan_for_operation(bytes).is_none() {
            return Err(ControlPlaneInjectionRefusal::NoOperationDiscriminant);
        }
        Err(ControlPlaneInjectionRefusal::NotAControlRequest)
    }
}
