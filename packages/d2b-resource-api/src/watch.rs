//! Resource-API watch delivery contract.
//!
//! External watch frames are handed to the authenticated bus as encoded
//! named-stream payloads: the sink below is the bus-side delivery trait
//! ([`WatchSink`]), and [`WatchFrame`] is the immutable encoded delivery.
//!
//! The manager plane does not serve external WATCH in this phase:
//! [`crate::manager_backend::ManagerBackend::watch`] refuses with
//! `UnsupportedCapability` because the composition has no consumer that
//! takes a manager watch registration and writes it to the component stream
//! the bus opened - a receipt would name a stream nothing fills. There is
//! deliberately no producer-side handoff registry here: one without a
//! consumer is the defect, not the fix.

use std::sync::Arc;

use d2b_contracts_resource::v3::ZoneRevision;

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
