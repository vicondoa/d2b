//! Named stream model (ADR 0032). Streams are named, typed, bounded, and
//! authorized. The mux discipline (frame caps, backpressure, fairness,
//! cancellation) lives in the stream-mux implementation; this module is
//! the codec-neutral descriptor surface.

use crate::capability::Capability;
use crate::ids::{PrincipalId, StreamId};
use crate::realm::RealmPath;
use serde::{Deserialize, Serialize};

/// The kind of a named stream. Each kind maps to a required capability.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum StreamKind {
    /// Request/response operations and events.
    Control,
    /// Terminal bytes, resize, exit metadata.
    Pty,
    /// Separate stdout/stderr in non-TTY mode.
    Stdio,
    /// Durable execution logs with resume cursors.
    Logs,
    /// Bounded chunks, hashes, explicit destination policy.
    FileCopy,
    /// One stream per connection; never a generic network bridge.
    PortForward,
    /// Display/window forwarding (late-stage capability).
    Display,
    /// Explicit opt-in, realm-gated clipboard.
    Clipboard,
    /// Audio playback (host → workload).
    AudioPlayback,
    /// Audio capture (workload → host); a distinct capability from playback.
    AudioCapture,
    /// Named HID device operations.
    DeviceHid,
    /// Named USB device operations; a distinct capability from HID.
    DeviceUsb,
}

impl StreamKind {
    /// The capability a peer must advertise to open this stream kind.
    /// `Display` requires `WindowForwarding`; clipboard/audio/device are
    /// independent so display cannot smuggle them, and audio
    /// playback/capture and HID/USB are split so the required capability
    /// is exact.
    pub fn required_capability(self) -> Capability {
        match self {
            StreamKind::Control => Capability::Lifecycle,
            StreamKind::Pty => Capability::Pty,
            StreamKind::Stdio => Capability::Exec,
            StreamKind::Logs => Capability::Logs,
            StreamKind::FileCopy => Capability::FileCopy,
            StreamKind::PortForward => Capability::PortForward,
            StreamKind::Display => Capability::WindowForwarding,
            StreamKind::Clipboard => Capability::Clipboard,
            StreamKind::AudioPlayback => Capability::AudioPlayback,
            StreamKind::AudioCapture => Capability::AudioCapture,
            StreamKind::DeviceHid => Capability::Hid,
            StreamKind::DeviceUsb => Capability::Usb,
        }
    }
}

/// A request to open a named stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamDescriptor {
    /// Stream id within the peer session.
    pub id: StreamId,
    /// Stream kind.
    pub kind: StreamKind,
}

/// The logical sub-channel a [`crate::frame::StreamData`] chunk belongs to.
///
/// Most stream kinds carry a single `Primary` channel. A `Stdio` stream in
/// non-TTY mode multiplexes `Stdout` and `Stderr` over one stream so the
/// two are ordered against each other yet still distinguishable — a peer
/// MUST NOT have to open two streams (and two authz contexts) just to split
/// stdout from stderr. The channel is a closed enum; an unknown channel is
/// rejected at decode (fail-closed).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum StreamChannel {
    /// The default single channel for a stream kind.
    #[default]
    Primary,
    /// Standard output (non-TTY `Stdio` streams).
    Stdout,
    /// Standard error (non-TTY `Stdio` streams).
    Stderr,
}

/// Why a stream was closed. Carried on [`crate::frame::StreamClose`] so the
/// peer can distinguish an orderly end-of-stream from a cancellation,
/// timeout, or error without inspecting payload bytes. Closed enum; an
/// unknown reason is rejected at decode (fail-closed).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum StreamCloseReason {
    /// Orderly end-of-stream: all data was delivered.
    Completed,
    /// The stream was cancelled by a peer (e.g. `ExecCancel`).
    Cancelled,
    /// The stream exceeded its inactivity/operation deadline.
    TimedOut,
    /// The stream ended because of an error (details travel in a separate
    /// typed-error frame, never in this reason).
    Errored,
    /// The remote end of the stream went away (process/relay/gateway gone).
    PeerGone,
}

/// The authorization context evaluated before a stream is opened. The
/// `capability` is always derived from the stream kind via
/// [`StreamAuthz::for_kind`] so a caller cannot pair a stream kind with a
/// weaker capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamAuthz {
    /// Authenticated principal (never a relay credential).
    pub principal: PrincipalId,
    /// Realm the stream belongs to (full path; supports nested realms).
    pub realm: RealmPath,
    /// Capability required for the stream kind (derived from the kind).
    pub capability: Capability,
}

impl StreamAuthz {
    /// Build an authorization context whose capability is derived from
    /// the stream kind (so the capability can never be downgraded).
    pub fn for_kind(principal: PrincipalId, realm: RealmPath, kind: StreamKind) -> Self {
        Self {
            principal,
            realm,
            capability: kind.required_capability(),
        }
    }

    /// True iff `capability` matches the capability `kind` requires. The
    /// mux/router MUST reject a stream open where this is false.
    pub fn matches_kind(&self, kind: StreamKind) -> bool {
        self.capability == kind.required_capability()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_requires_window_forwarding_not_clipboard() {
        assert_eq!(
            StreamKind::Display.required_capability(),
            Capability::WindowForwarding
        );
        assert_eq!(
            StreamKind::Clipboard.required_capability(),
            Capability::Clipboard
        );
    }

    #[test]
    fn audio_and_device_capabilities_are_exact() {
        assert_eq!(
            StreamKind::AudioPlayback.required_capability(),
            Capability::AudioPlayback
        );
        assert_eq!(
            StreamKind::AudioCapture.required_capability(),
            Capability::AudioCapture
        );
        assert_eq!(StreamKind::DeviceHid.required_capability(), Capability::Hid);
        assert_eq!(StreamKind::DeviceUsb.required_capability(), Capability::Usb);
    }

    #[test]
    fn stream_authz_capability_is_derived_from_kind() {
        let p = PrincipalId::parse("principal-1").unwrap();
        let realm = RealmPath::local();
        let authz = StreamAuthz::for_kind(p, realm, StreamKind::Display);
        assert!(authz.matches_kind(StreamKind::Display));
        assert!(!authz.matches_kind(StreamKind::Clipboard));
    }
}
