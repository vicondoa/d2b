//! Per-provider capability matrix types (ADR 0041).
//!
//! Tracks what audio and console enforcement mechanisms a given runtime
//! provider supports. Callers use these descriptors to select the correct
//! enforcement path before dispatching an audio or console op. Types are kept
//! separate from [`crate::audio_policy`] so the policy state module stays
//! focused on the per-VM on-disk state contract.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ── Audio capability matrix ──────────────────────────────────────────────────

/// Host-side audio enforcement mechanism for a provider.
///
/// Separate from [`AudioGuestEnforcementKind`] so that host and guest
/// capabilities can evolve independently, and so "guestd enforced" is
/// never a valid description of host-side audio behaviour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AudioHostEnforcementKind {
    /// No host-side enforcement (provider-managed sandboxes, unsupported
    /// providers, or VMs without a vhost-user-sound sidecar).
    #[default]
    None,
    /// PipeWire policy enforced through the vhost-user-sound sidecar (Cloud
    /// Hypervisor NixOS VMs with `audio.enable = true`).
    PipeWireVhostUserSound,
    /// qemu audio backend declared in the qemu-media config.
    QemuAudioBackend,
}

/// Guest-side audio enforcement mechanism for a provider.
///
/// Separate from [`AudioHostEnforcementKind`] so that guest enforcement via
/// guestd cannot be conflated with host-side PipeWire or qemu enforcement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AudioGuestEnforcementKind {
    /// Guest-side enforcement is not supported (qemu-media VMs, local-only
    /// providers, or provider sandboxes without a running guestd agent).
    #[default]
    Unsupported,
    /// guestd-capable audio policy via the authenticated guest-control
    /// transport (Cloud Hypervisor NixOS VMs) or provider peer transport
    /// (ACA sandboxes with a guestd-compatible agent).
    GuestdCapable,
}

/// Per-provider audio capability row, used by the daemon to select the
/// correct enforcement path before dispatching an audio op.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioProviderCapability {
    /// Host-side audio enforcement mechanism for this provider.
    pub host_enforcement: AudioHostEnforcementKind,
    /// Guest-side audio enforcement mechanism for this provider.
    pub guest_enforcement: AudioGuestEnforcementKind,
    /// Whether the provider requires a local audio-state file on the host.
    pub needs_local_state_file: bool,
}

impl AudioProviderCapability {
    /// Capability row for Cloud Hypervisor NixOS VMs: PipeWire vhost-user-sound
    /// host enforcement plus guestd guest enforcement.
    pub fn cloud_hypervisor_nixos() -> Self {
        Self {
            host_enforcement: AudioHostEnforcementKind::PipeWireVhostUserSound,
            guest_enforcement: AudioGuestEnforcementKind::GuestdCapable,
            needs_local_state_file: true,
        }
    }

    /// Capability row for qemu-media VMs: declared qemu audio backend on the
    /// host only; guest enforcement unsupported.
    pub fn qemu_media() -> Self {
        Self {
            host_enforcement: AudioHostEnforcementKind::QemuAudioBackend,
            guest_enforcement: AudioGuestEnforcementKind::Unsupported,
            needs_local_state_file: true,
        }
    }

    /// Capability row for ACA sandbox targets: no local host enforcement;
    /// guest enforcement via the guestd-compatible sandbox agent over the
    /// provider peer transport.
    pub fn aca_sandbox() -> Self {
        Self {
            host_enforcement: AudioHostEnforcementKind::None,
            guest_enforcement: AudioGuestEnforcementKind::GuestdCapable,
            needs_local_state_file: false,
        }
    }
}

// ── Console capability matrix ────────────────────────────────────────────────

/// Console access backend kind for a provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ConsoleBackendKind {
    /// No console support.
    #[default]
    None,
    /// Local hypervisor console backend (Cloud Hypervisor serial socket or
    /// broker-owned fd).
    LocalHypervisor,
    /// Provider relay transport (ACA sandbox via ADR 0032 guestd route).
    ProviderRelay,
}

/// Per-provider console capability row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleProviderCapability {
    /// How the console stream is established for this provider.
    pub backend: ConsoleBackendKind,
    /// Whether a persistent drain keeps the ring buffer populated even when
    /// no client is attached.
    pub persistent_drain: bool,
}

impl ConsoleProviderCapability {
    /// Capability row for Cloud Hypervisor NixOS VMs.
    pub fn cloud_hypervisor_nixos() -> Self {
        Self {
            backend: ConsoleBackendKind::LocalHypervisor,
            persistent_drain: true,
        }
    }

    /// Capability row for qemu-media VMs.
    pub fn qemu_media() -> Self {
        Self {
            backend: ConsoleBackendKind::LocalHypervisor,
            persistent_drain: true,
        }
    }

    /// Capability row for ACA sandbox targets.
    pub fn aca_sandbox() -> Self {
        Self {
            backend: ConsoleBackendKind::ProviderRelay,
            persistent_drain: false,
        }
    }
}
