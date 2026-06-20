//! The capability model (ADR 0032). Capabilities are **positive
//! assertions**: a node/provider advertises exactly what it supports, and
//! an absent capability means a typed refusal, never a silent fallback.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A named, independently-authorized capability. Display, clipboard,
/// audio, HID, and USB are deliberately distinct so display forwarding
/// cannot smuggle clipboard or device access.
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
pub enum Capability {
    /// Workload create/start/stop/inspect.
    Lifecycle,
    /// Command execution.
    Exec,
    /// Interactive pseudo-terminal.
    Pty,
    /// Durable execution logs with resume cursors.
    Logs,
    /// Bounded file copy.
    FileCopy,
    /// One stream per connection; never a generic network bridge.
    PortForward,
    /// virtio-vsock availability.
    Vsock,
    /// virtiofs share availability.
    Virtiofs,
    /// Semantic Wayland window/protocol forwarding.
    WindowForwarding,
    /// Encoded frame/video stream for environments without host Wayland.
    DisplayStreaming,
    /// Clipboard bridge (separate from display).
    Clipboard,
    /// Audio playback.
    AudioPlayback,
    /// Audio capture.
    AudioCapture,
    /// Named HID device operations.
    Hid,
    /// Named USB device operations.
    Usb,
    /// Local/runtime GPU acceleration (not automatically relay-exportable).
    GpuAccel,
    /// Snapshots.
    Snapshots,
    /// Device hotplug.
    Hotplug,
    /// Ephemeral provider-managed sessions.
    EphemeralSessions,
    /// Provider-managed isolation boundary (not host-owned KVM).
    ProviderManagedIsolation,
}

impl Capability {
    /// A short, stable, low-cardinality kebab-case code (for messages and
    /// audit; never a secret).
    pub fn code(self) -> &'static str {
        match self {
            Capability::Lifecycle => "lifecycle",
            Capability::Exec => "exec",
            Capability::Pty => "pty",
            Capability::Logs => "logs",
            Capability::FileCopy => "file-copy",
            Capability::PortForward => "port-forward",
            Capability::Vsock => "vsock",
            Capability::Virtiofs => "virtiofs",
            Capability::WindowForwarding => "window-forwarding",
            Capability::DisplayStreaming => "display-streaming",
            Capability::Clipboard => "clipboard",
            Capability::AudioPlayback => "audio-playback",
            Capability::AudioCapture => "audio-capture",
            Capability::Hid => "hid",
            Capability::Usb => "usb",
            Capability::GpuAccel => "gpu-accel",
            Capability::Snapshots => "snapshots",
            Capability::Hotplug => "hotplug",
            Capability::EphemeralSessions => "ephemeral-sessions",
            Capability::ProviderManagedIsolation => "provider-managed-isolation",
        }
    }
}

/// A set of advertised capabilities. Routing is by required capability;
/// callers fail closed when a required capability is absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct CapabilitySet(BTreeSet<Capability>);

impl CapabilitySet {
    /// An empty set (advertises nothing).
    pub fn empty() -> Self {
        Self(BTreeSet::new())
    }

    /// Build from an iterator of capabilities.
    pub fn from_caps<I: IntoIterator<Item = Capability>>(caps: I) -> Self {
        caps.into_iter().collect()
    }

    /// Add a capability (builder style).
    pub fn with(mut self, cap: Capability) -> Self {
        self.0.insert(cap);
        self
    }

    /// True iff the capability is advertised.
    pub fn has(&self, cap: Capability) -> bool {
        self.0.contains(&cap)
    }

    /// Iterate the advertised capabilities in a stable order.
    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.0.iter().copied()
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<I: IntoIterator<Item = Capability>>(caps: I) -> Self {
        Self(caps.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_capability_is_not_advertised() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        assert!(caps.has(Capability::Lifecycle));
        assert!(!caps.has(Capability::WindowForwarding));
        // display and clipboard are independent.
        let disp = CapabilitySet::from_iter([Capability::WindowForwarding]);
        assert!(disp.has(Capability::WindowForwarding));
        assert!(!disp.has(Capability::Clipboard));
    }
}
