//! Named-stream bridge lifecycle.

use async_trait::async_trait;
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
    async fn open_named_stream(&self) -> Result<(NamedStreamId, Self::Stream), NamedStreamError>;

    /// Close one named stream.
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
pub struct BridgeControl {
    stop: watch::Sender<bool>,
    completed: Arc<Notify>,
}

impl BridgeControl {
    /// Create a bridge control pair.
    pub fn new() -> (Self, watch::Receiver<bool>) {
        let (stop, receiver) = watch::channel(false);
        (
            Self {
                stop,
                completed: Arc::new(Notify::new()),
            },
            receiver,
        )
    }

    /// Request bridge shutdown.
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }

    /// Wait for the bridge task to finish.
    pub async fn wait(&self) {
        self.completed.notified().await;
    }

    /// Clone the completion signal for the bridge task.
    pub(crate) fn completion(&self) -> Arc<Notify> {
        Arc::clone(&self.completed)
    }
}

/// Run a byte bridge until one side closes or the owner requests shutdown.
pub async fn run_bridge<L, R>(
    mut left: L,
    mut right: R,
    mut stop: watch::Receiver<bool>,
    stats: Arc<BridgeStats>,
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
                Err(_) => BridgeExit::IoError,
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
