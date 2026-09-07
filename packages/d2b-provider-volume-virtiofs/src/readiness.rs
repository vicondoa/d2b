//! Fail-closed readiness classification for the binding serving side.

use crate::error::VirtiofsBindingError;
use crate::port::BindingPhase;

/// Observation of the private binding socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketObservation {
    /// The socket exists and is listening.
    Ready,
    /// The socket is absent.
    Absent,
}

/// Observation of the guest mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestMountObservation {
    /// The guest reports the mount present.
    Ready,
    /// The mount is not observed inside the guest.
    Absent,
    /// The guest-side probe could not be completed.
    Unreachable,
}

/// Observation of the zero-length store-view marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreViewMarkerObservation {
    /// Whether the marker file exists.
    pub present: bool,
    /// Whether the marker file is zero-length.
    pub zero_length: bool,
}

/// Classify binding status from the two readiness probes.
pub const fn classify_readiness(
    socket: SocketObservation,
    guest: Option<GuestMountObservation>,
) -> (BindingPhase, Option<VirtiofsBindingError>) {
    match socket {
        SocketObservation::Absent => (
            BindingPhase::Pending,
            Some(VirtiofsBindingError::BindingNotReady),
        ),
        SocketObservation::Ready => match guest {
            Some(GuestMountObservation::Ready) => (BindingPhase::Ready, None),
            Some(GuestMountObservation::Unreachable) => (
                BindingPhase::Degraded,
                Some(VirtiofsBindingError::GuestMountNotReady),
            ),
            Some(GuestMountObservation::Absent) | None => (
                BindingPhase::Degraded,
                Some(VirtiofsBindingError::GuestMountNotReady),
            ),
        },
    }
}

/// Require a present, zero-length store-view marker before launch.
pub const fn require_store_view_marker(
    observation: StoreViewMarkerObservation,
) -> Result<(), VirtiofsBindingError> {
    if observation.present && observation.zero_length {
        Ok(())
    } else {
        Err(VirtiofsBindingError::StoreViewMarkerMissing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_fails_closed_on_absent_probes() {
        assert_eq!(
            classify_readiness(SocketObservation::Absent, None),
            (
                BindingPhase::Pending,
                Some(VirtiofsBindingError::BindingNotReady)
            )
        );
        assert_eq!(
            classify_readiness(SocketObservation::Ready, Some(GuestMountObservation::Ready)),
            (BindingPhase::Ready, None)
        );
    }

    #[test]
    fn a_missing_store_view_marker_never_launches() {
        assert_eq!(
            require_store_view_marker(StoreViewMarkerObservation {
                present: true,
                zero_length: false,
            }),
            Err(VirtiofsBindingError::StoreViewMarkerMissing)
        );
    }
}
