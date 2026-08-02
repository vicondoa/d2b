//! Bounded socket and guest-mount readiness decisions.

use crate::error::VirtiofsExportError;
use crate::port::ExportPhase;

/// Observation of the private export socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketObservation {
    /// The socket exists and is listening.
    Ready,
    /// The socket is not present yet.
    Absent,
}

/// Observation returned by the guest-control mount probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestMountObservation {
    /// The guest reports the mount present.
    Ready,
    /// The guest reports the mount absent.
    Absent,
    /// The guest could not be reached.
    Unreachable,
}

/// Bounded readiness observation for a store-view marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreViewMarkerObservation {
    /// Marker file exists.
    pub present: bool,
    /// Marker file has zero length.
    pub zero_length: bool,
}

/// Classify Export status from the two readiness probes.
pub const fn classify_readiness(
    socket: SocketObservation,
    guest: Option<GuestMountObservation>,
) -> (ExportPhase, Option<VirtiofsExportError>) {
    match socket {
        SocketObservation::Absent => (
            ExportPhase::Pending,
            Some(VirtiofsExportError::ExportNotReady),
        ),
        SocketObservation::Ready => match guest {
            Some(GuestMountObservation::Ready) => (ExportPhase::Ready, None),
            Some(GuestMountObservation::Unreachable) => (
                ExportPhase::Degraded,
                Some(VirtiofsExportError::GuestMountNotReady),
            ),
            Some(GuestMountObservation::Absent) | None => (
                ExportPhase::Degraded,
                Some(VirtiofsExportError::GuestMountNotReady),
            ),
        },
    }
}

/// Require a valid zero-length store-view marker before launch.
pub const fn require_store_view_marker(
    observation: StoreViewMarkerObservation,
) -> Result<(), VirtiofsExportError> {
    if observation.present && observation.zero_length {
        Ok(())
    } else {
        Err(VirtiofsExportError::StoreViewMarkerMissing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_must_be_ready_before_guest_mount_is_probed() {
        assert_eq!(
            classify_readiness(SocketObservation::Absent, None),
            (
                ExportPhase::Pending,
                Some(VirtiofsExportError::ExportNotReady)
            )
        );
        assert_eq!(
            classify_readiness(SocketObservation::Ready, Some(GuestMountObservation::Ready)),
            (ExportPhase::Ready, None)
        );
    }

    #[test]
    fn store_view_marker_requires_zero_length_presence() {
        assert!(
            require_store_view_marker(StoreViewMarkerObservation {
                present: true,
                zero_length: true,
            })
            .is_ok()
        );
        assert_eq!(
            require_store_view_marker(StoreViewMarkerObservation {
                present: true,
                zero_length: false,
            }),
            Err(VirtiofsExportError::StoreViewMarkerMissing)
        );
    }
}
