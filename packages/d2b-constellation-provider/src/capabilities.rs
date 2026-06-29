//! Typed capability descriptors (ADR 0032). Capabilities are structured
//! data, not comments. Each provider family advertises a descriptor;
//! callers route by required capability and fail closed when absent.

use d2b_constellation_core::{Capability, CapabilitySet};
use serde::{Deserialize, Serialize};

/// Capabilities advertised by a [`crate::RuntimeProvider`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapabilitySet {
    /// The underlying capability set.
    pub caps: CapabilitySet,
}

/// Capabilities advertised by a [`crate::WorkloadProvider`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadCapabilitySet {
    /// The underlying capability set.
    pub caps: CapabilitySet,
}

/// Capabilities advertised by a [`crate::DisplayProvider`]. Display
/// providers advertise window-forwarding, display-streaming, clipboard,
/// audio, and input independently, plus a latency class and whether the
/// transport supports reconnect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayCapabilitySet {
    /// The underlying capability set (window-forwarding/clipboard/audio/…).
    pub caps: CapabilitySet,
    /// Whether SHM buffer forwarding is supported.
    pub shm_buffers: bool,
    /// Whether dmabuf forwarding is available (false for ACA sandboxes).
    pub dmabuf: bool,
    /// Whether the display transport supports reconnect.
    pub reconnect: bool,
}

/// Capabilities advertised by a [`crate::NodeProvider`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCapabilitySet {
    /// The underlying capability set.
    pub caps: CapabilitySet,
    /// Host substrate family, when this descriptor comes from a host substrate
    /// provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub substrate: Option<HostSubstrateKind>,
    /// Non-secret substrate version (for example Ubuntu `24.04`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub substrate_version: Option<String>,
    /// Whether unprivileged user namespaces appear usable for the substrate.
    #[serde(default)]
    pub userns_available: bool,
    /// Whether vhost acceleration appears available for the substrate.
    #[serde(default)]
    pub vhost_acceleration: bool,
    /// Low-cardinality LSM label (`landlock`, `apparmor`, `selinux`, `none`,
    /// or `unknown`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lsm: Option<String>,
}

/// Host substrate family advertised by a [`crate::HostSubstrateProvider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostSubstrateKind {
    /// NixOS module-managed host.
    NixOs,
    /// Ubuntu or Ubuntu-derived generic Linux host.
    Ubuntu,
    /// Generic Linux host without a more specific adapter.
    GenericLinux,
}

macro_rules! caps_accessor {
    ($t:ty) => {
        impl $t {
            /// True iff the capability is advertised.
            pub fn has(&self, cap: Capability) -> bool {
                self.caps.has(cap)
            }
        }
    };
}

caps_accessor!(RuntimeCapabilitySet);
caps_accessor!(WorkloadCapabilitySet);
caps_accessor!(DisplayCapabilitySet);
caps_accessor!(NodeCapabilitySet);

// ---- Console capability descriptor (ADR 0041) --------------------------------

/// Capabilities advertised by a console provider (ADR 0041). Console providers
/// expose a VM serial/PTY stream backed by a persistent ring-buffer drainer.
///
/// Distinct from [`DisplayCapabilitySet`]: display forwarding does not imply
/// console access, and console does not imply Wayland/display-streaming.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleCapabilitySet {
    /// Backing console transport kind.
    pub kind: ConsoleSupportKind,
    /// Whether the ring-buffer drainer persists across client disconnects so
    /// console output is never lost when no operator is attached.
    pub drainer_persistent: bool,
    /// Whether the console session can be re-attached after a client
    /// disconnect without losing ring-buffer history.
    pub reconnect: bool,
}

/// The transport backing a console provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConsoleSupportKind {
    /// No console support.
    #[default]
    None,
    /// Cloud Hypervisor `--serial socket=...` or equivalent fd backend managed
    /// by the broker, with a daemon-side persistent drainer. Only valid when
    /// proven non-blocking and attach-safe.
    LocalHypervisorSocket,
    /// qemu-media broker-owned fd-backed chardev or PTY/fd-store design.
    /// A qemu-created path socket is not the default (weakens socket-permission
    /// posture and can race with stale socket cleanup).
    QemuFdBacked,
    /// ACA sandbox guestd-compatible agent over the provider peer transport.
    /// No local socket or broker fd; no relay URL or resource ID exposed.
    ProviderGuestd,
}

impl ConsoleCapabilitySet {
    /// Cloud Hypervisor NixOS VM: local hypervisor socket backend with a
    /// persistent daemon drainer and re-attach support.
    pub fn local_hypervisor(drainer_persistent: bool) -> Self {
        Self {
            kind: ConsoleSupportKind::LocalHypervisorSocket,
            drainer_persistent,
            reconnect: true,
        }
    }

    /// qemu-media VM: broker-owned fd backend. Drainer is persistent when the
    /// broker holds the fd; re-attach support depends on the backend design.
    pub fn qemu_fd_backed(drainer_persistent: bool, reconnect: bool) -> Self {
        Self {
            kind: ConsoleSupportKind::QemuFdBacked,
            drainer_persistent,
            reconnect,
        }
    }

    /// ACA sandbox via guestd-compatible agent over the provider peer
    /// transport. Drainer and reconnect semantics are provider-defined.
    pub fn provider_guestd(drainer_persistent: bool, reconnect: bool) -> Self {
        Self {
            kind: ConsoleSupportKind::ProviderGuestd,
            drainer_persistent,
            reconnect,
        }
    }
}

// ---- Audio capability descriptor (ADR 0041) ----------------------------------

/// Capabilities advertised by an audio provider (ADR 0041).
///
/// Host and guest enforcement are deliberately independent so that display
/// forwarding cannot smuggle audio enforcement, and so that qemu-media VMs
/// can advertise host-only enforcement without implying guestd availability.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioCapabilitySet {
    /// Host-side audio enforcement mechanism.
    pub host_enforcement: AudioHostEnforcement,
    /// Guest-side audio enforcement mechanism (via guestd or unsupported).
    pub guest_enforcement: AudioGuestEnforcement,
}

/// Host-side audio enforcement mechanism for a provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioHostEnforcement {
    /// No host-side audio enforcement (provider-managed sandboxes, unsupported
    /// providers, or VMs without a vhost-user-sound sidecar).
    #[default]
    None,
    /// PipeWire policy through the vhost-user-sound sidecar (Cloud Hypervisor
    /// NixOS VMs with `audio.enable = true`).
    PipeWireVhostUserSound,
    /// qemu audio backend declared in the qemu-media config.
    QemuAudioBackend,
}

/// Guest-side audio enforcement mechanism for a provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioGuestEnforcement {
    /// Guest-side enforcement is not supported (qemu-media VMs, local-only
    /// providers, or provider sandboxes without a running guestd agent).
    #[default]
    Unsupported,
    /// guestd-capable audio policy via the authenticated guest-control
    /// transport (Cloud Hypervisor NixOS VMs) or provider peer transport
    /// (ACA sandboxes with a guestd-compatible agent).
    GuestdCapable,
}

impl AudioCapabilitySet {
    /// Cloud Hypervisor NixOS VM: PipeWire vhost-user-sound host enforcement
    /// plus guestd guest enforcement.
    pub fn local_hypervisor_full() -> Self {
        Self {
            host_enforcement: AudioHostEnforcement::PipeWireVhostUserSound,
            guest_enforcement: AudioGuestEnforcement::GuestdCapable,
        }
    }

    /// qemu-media VM: declared qemu audio backend host enforcement only;
    /// guest enforcement unsupported.
    pub fn qemu_media_host_only() -> Self {
        Self {
            host_enforcement: AudioHostEnforcement::QemuAudioBackend,
            guest_enforcement: AudioGuestEnforcement::Unsupported,
        }
    }

    /// ACA sandbox: no local host enforcement; guest enforcement via the
    /// guestd-compatible sandbox agent over the provider peer transport.
    pub fn aca_sandbox_guest_only() -> Self {
        Self {
            host_enforcement: AudioHostEnforcement::None,
            guest_enforcement: AudioGuestEnforcement::GuestdCapable,
        }
    }
}

impl DisplayCapabilitySet {
    /// Local Wayland/window-forwarding provider shape. Clipboard, audio, HID,
    /// USB, GPU acceleration, and video are separate capabilities and are not
    /// implied by display.
    pub fn local_wayland() -> Self {
        Self {
            caps: CapabilitySet::from_caps([Capability::WindowForwarding]),
            shm_buffers: true,
            dmabuf: true,
            reconnect: false,
        }
    }

    /// Provider-managed display-streaming shape. Used when display bytes cross
    /// an authorized provider/relay stream instead of a local Wayland socket.
    pub fn provider_streaming(reconnect: bool) -> Self {
        Self {
            caps: CapabilitySet::from_caps([Capability::DisplayStreaming]),
            shm_buffers: false,
            dmabuf: false,
            reconnect,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_capabilities_do_not_imply_adjacent_io() {
        let local = DisplayCapabilitySet::local_wayland();
        assert!(local.has(Capability::WindowForwarding));
        assert!(!local.has(Capability::Clipboard));
        assert!(!local.has(Capability::AudioPlayback));
        assert!(!local.has(Capability::Usb));
        assert!(!local.has(Capability::Hid));
        assert!(!local.has(Capability::GpuAccel));
        assert!(local.shm_buffers);
        assert!(local.dmabuf);
        assert!(!local.reconnect);

        let provider = DisplayCapabilitySet::provider_streaming(true);
        assert!(provider.has(Capability::DisplayStreaming));
        assert!(!provider.has(Capability::WindowForwarding));
        assert!(!provider.shm_buffers);
        assert!(!provider.dmabuf);
        assert!(provider.reconnect);
    }

    #[test]
    fn console_capability_set_constructors_produce_correct_kinds() {
        let ch = ConsoleCapabilitySet::local_hypervisor(true);
        assert_eq!(ch.kind, ConsoleSupportKind::LocalHypervisorSocket);
        assert!(ch.drainer_persistent);
        assert!(ch.reconnect);

        let qemu = ConsoleCapabilitySet::qemu_fd_backed(false, false);
        assert_eq!(qemu.kind, ConsoleSupportKind::QemuFdBacked);
        assert!(!qemu.drainer_persistent);
        assert!(!qemu.reconnect);

        let aca = ConsoleCapabilitySet::provider_guestd(true, true);
        assert_eq!(aca.kind, ConsoleSupportKind::ProviderGuestd);
        assert!(aca.drainer_persistent);
        assert!(aca.reconnect);

        let none = ConsoleCapabilitySet::default();
        assert_eq!(none.kind, ConsoleSupportKind::None);
        assert!(!none.drainer_persistent);
    }

    #[test]
    fn audio_capability_set_constructors_match_adr_0041_provider_matrix() {
        // Cloud Hypervisor: host PipeWire + guestd guest enforcement.
        let ch = AudioCapabilitySet::local_hypervisor_full();
        assert_eq!(
            ch.host_enforcement,
            AudioHostEnforcement::PipeWireVhostUserSound
        );
        assert_eq!(ch.guest_enforcement, AudioGuestEnforcement::GuestdCapable);

        // qemu-media: host qemu backend only; guestd unsupported.
        let qemu = AudioCapabilitySet::qemu_media_host_only();
        assert_eq!(
            qemu.host_enforcement,
            AudioHostEnforcement::QemuAudioBackend
        );
        assert_eq!(qemu.guest_enforcement, AudioGuestEnforcement::Unsupported);

        // ACA sandbox: no host enforcement; guestd guest enforcement.
        let aca = AudioCapabilitySet::aca_sandbox_guest_only();
        assert_eq!(aca.host_enforcement, AudioHostEnforcement::None);
        assert_eq!(aca.guest_enforcement, AudioGuestEnforcement::GuestdCapable);

        // Display forwarding must not imply audio enforcement.
        let display = DisplayCapabilitySet::local_wayland();
        assert!(!display.has(Capability::AudioPlayback));
        assert!(!display.has(Capability::AudioCapture));
    }

    #[test]
    fn console_and_audio_capability_sets_are_independent_of_display() {
        // ConsoleSupportKind::None is the default (fail-closed).
        let no_console = ConsoleCapabilitySet::default();
        assert_eq!(no_console.kind, ConsoleSupportKind::None);

        // AudioCapabilitySet::default() enforces nothing (fail-closed).
        let no_audio = AudioCapabilitySet::default();
        assert_eq!(no_audio.host_enforcement, AudioHostEnforcement::None);
        assert_eq!(
            no_audio.guest_enforcement,
            AudioGuestEnforcement::Unsupported
        );
    }
}
