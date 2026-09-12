//! Resource-API watch handoff from the manager's watch hub to the wire.
//!
//! The wire service returns a stream name and snapshot revision, while the
//! authenticated bus owns the actual delivery task. This module keeps that
//! handoff explicit: the backend registers with the manager's serialized
//! watch hub, retains the replay prefix, and hands the named stream to the
//! bus owner.

use std::sync::Arc;

use d2b_contracts_resource::v3::ZoneRevision;
use serde_json::json;

/// One immutable encoded watch delivery.
#[derive(Clone, PartialEq, Eq)]
pub struct WatchFrame {
    revision: ZoneRevision,
    payload: Arc<[u8]>,
}

impl core::fmt::Debug for WatchFrame {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WatchFrame")
            .field("revision", &self.revision)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

impl WatchFrame {
    /// Return the runtime revision represented by this frame.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Borrow the bounded canonical-JSON payload.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// Closed failures returned by a named-stream delivery sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchSinkError {
    /// The sink is waiting for transport credit.
    Backpressure,
    /// The authenticated destination or stream is gone.
    Closed,
    /// The sink cannot carry one complete watch frame.
    FrameTooLarge,
}

impl core::fmt::Display for WatchSinkError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Backpressure => "watch sink is backpressured",
            Self::Closed => "watch sink is closed",
            Self::FrameTooLarge => "watch frame exceeds sink bounds",
        })
    }
}

impl std::error::Error for WatchSinkError {}

/// Sink implemented by the authenticated bus named-stream adapter.
pub trait WatchSink: Send + Sync {
    fn send(&self, frame: WatchFrame) -> impl Future<Output = Result<(), WatchSinkError>> + Send;
}

/// Failure returned by the complete watch-to-sink handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchPumpError {
    Sink(WatchSinkError),
}

impl core::fmt::Display for WatchPumpError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Sink(error) => write!(formatter, "watch sink failed: {error}"),
        }
    }
}

impl std::error::Error for WatchPumpError {}

// ---------------------------------------------------------------------------
// Manager-plane watch handoff (U8, KTD8; R23-R24)
// ---------------------------------------------------------------------------

use d2b_resource_runtime::watch::{
    ChangeKind as HubChangeKind, ChangeSource as HubChangeSource,
    WatchDelivery as HubWatchDelivery, WatchStream as HubWatchStream,
};

/// A watch registration from the manager plane, held for the authenticated
/// bus adapter: retained replay events (in revision order) followed by the
/// hub's live delivery stream. Taken from the backend's
/// [`ManagerWatchStreams`] by receipt stream name.
///
/// Delivery maps the hub's runtime revisions onto the wire revision
/// (`epoch_seconds << 32 | sequence`, the U5 budget). There are no
/// acknowledgements: the hub's slow-subscriber rule raises the explicit
/// `Missed` marker, which terminates the stream and sends the client back
/// through LIST/relist (R24). The hub's change notices carry the changed key
/// only (there is no second plane whose owner edges a frame could relay).
pub struct ManagerWatch {
    replay: std::collections::VecDeque<d2b_resource_runtime::watch::ResourceChange>,
    stream: HubWatchStream,
}

impl core::fmt::Debug for ManagerWatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ManagerWatch(<redacted>)")
    }
}

impl ManagerWatch {
    /// Receive and encode one delivery: retained replay first (relist-safe),
    /// then live events, ending at the `Missed` marker.
    pub async fn recv_frame(&mut self) -> Option<WatchFrame> {
        if let Some(change) = self.replay.pop_front() {
            return Some(encode_change(&change));
        }
        match self.stream.recv().await {
            Some(HubWatchDelivery::Change(change)) => Some(encode_change(&change)),
            // Explicit missed-data marker: the client relists (R24). The
            // stream ends here by contract.
            Some(HubWatchDelivery::Missed { .. }) | None => None,
        }
    }

    /// Pump the watch into one bounded sink. Returns after the terminal
    /// marker; the caller relists and re-watches from a fresh snapshot.
    pub async fn pump_to<S: WatchSink>(&mut self, sink: &S) -> Result<(), WatchPumpError> {
        while let Some(frame) = self.recv_frame().await {
            sink.send(frame).await.map_err(WatchPumpError::Sink)?;
        }
        Ok(())
    }
}

/// Shared registry mapping receipt stream names to the manager-plane
/// delivery the backend registered through the manager's serialized watch
/// handoff.
#[derive(Default)]
pub struct ManagerWatchStreams(
    std::sync::Mutex<std::collections::HashMap<String, ManagerWatch>>,
);

impl core::fmt::Debug for ManagerWatchStreams {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ManagerWatchStreams(<redacted>)")
    }
}

impl ManagerWatchStreams {
    pub(crate) fn insert(
        &self,
        replay: Vec<d2b_resource_runtime::watch::ResourceChange>,
        stream: HubWatchStream,
    ) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let stream_name = format!("manager-watch-{}", NEXT.fetch_add(1, Ordering::Relaxed));
        self.0
            .lock()
            .expect("watch handoff registry lock")
            .insert(stream_name.clone(), ManagerWatch {
                replay: replay.into_iter().collect(),
                stream,
            });
        stream_name
    }

    /// Take the delivery for `stream_name` (one handoff per registration).
    pub fn take(&self, stream_name: &str) -> Option<ManagerWatch> {
        self.0
            .lock()
            .expect("watch handoff registry")
            .remove(stream_name)
    }
}

/// Encode one hub change as the wire frame: the wire revision carries the
/// mapped runtime revision, and the bounded payload names the changed key.
fn encode_change(change: &d2b_resource_runtime::watch::ResourceChange) -> WatchFrame {
    let revision = crate::manager_backend::wire_revision(change.revision);
    let payload = serde_json::to_vec(&json!({
        "revision": revision,
        "entries": [{
            "zone": change.key.zone,
            "type": change.key.type_name,
            "name": change.key.name,
            "kind": match change.kind {
                HubChangeKind::Upsert => "upsert",
                HubChangeKind::Delete => "delete",
            },
            "source": match change.source {
                HubChangeSource::Desired => "desired",
                HubChangeSource::RuntimeStatus => "runtime-status",
            },
        }],
    }))
    .unwrap_or_default();
    WatchFrame {
        revision: ZoneRevision::new(revision),
        payload: std::sync::Arc::from(payload),
    }
}
