//! Audio-pipewire Provider manifest facts.

use serde::Serialize;

/// Fixed manifest projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioManifest {
    /// Artifact identity.
    pub artifact_id: &'static str,
    /// Provider-neutral ResourceTypes.
    pub resource_types: [&'static str; 2],
    /// Static controller and user-session components.
    pub components: [&'static str; 3],
    /// Audio has no Provider-owned state Volume.
    pub declares_state_volume: bool,
}

impl AudioManifest {
    /// Return the canonical manifest.
    pub const fn canonical() -> Self {
        Self {
            artifact_id: "audio-pipewire",
            resource_types: [
                "audio.d2bus.org.AudioService",
                "audio.d2bus.org.AudioBinding",
            ],
            components: [
                "audio-controller",
                "audio-pipewire-mediator",
                "guest-audio-agent",
            ],
            declares_state_volume: false,
        }
    }
}
