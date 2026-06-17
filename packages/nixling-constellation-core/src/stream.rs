//! Named stream model (ADR 0032). Streams are named, typed, bounded, and
//! authorized. The mux discipline (frame caps, backpressure, fairness,
//! cancellation) lives in the stream-mux implementation; this module is
//! the codec-neutral descriptor surface.

use crate::capability::Capability;
use crate::ids::{PrincipalId, RealmId, StreamId};
use serde::{Deserialize, Serialize};

/// The kind of a named stream. Each kind maps to a required capability.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
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
    /// Audio playback/capture.
    Audio,
    /// USB/HID-like operations through named policy only.
    Device,
}

impl StreamKind {
    /// The capability a peer must advertise to open this stream kind.
    /// `Display` requires `WindowForwarding`; clipboard/audio/device are
    /// independent so display cannot smuggle them.
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
            StreamKind::Audio => Capability::AudioPlayback,
            StreamKind::Device => Capability::Hid,
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

/// The authorization context evaluated before a stream is opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamAuthz {
    /// Authenticated principal (never a relay credential).
    pub principal: PrincipalId,
    /// Realm the stream belongs to.
    pub realm: RealmId,
    /// Capability required for the stream kind.
    pub capability: Capability,
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
}
