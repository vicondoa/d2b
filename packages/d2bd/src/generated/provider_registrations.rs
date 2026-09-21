// @generated
// Provenance: emitted from the per-crate `registrations.json` declarations
// by `cargo xtask check-provider-crate-layout --fix`; the layout check's
// authority drift gate regenerates this file byte-for-byte, and refuses a
// hand edit.

/// One registered provider family row: the provider identity the daemon's
/// composition root composes and the effect-service ids the family declares.
pub(crate) struct ProviderRegistration {
    pub(crate) provider_ref: &'static str,
    pub(crate) services: &'static [&'static str],
}

/// The registered provider families, in declaration order.
pub(crate) const PROVIDER_REGISTRATIONS: &[ProviderRegistration] = &[
    ProviderRegistration {
        provider_ref: "activation-nixos",
        services: &["activation.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "audio-binding",
        services: &[],
    },
    ProviderRegistration {
        provider_ref: "audio-service",
        services: &[],
    },
    ProviderRegistration {
        provider_ref: "device",
        services: &["device.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "device-security-key",
        services: &["security-key.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "device-usbip",
        services: &["usbip.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "endpoint",
        services: &["endpoint.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "guest",
        services: &["guest.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "host",
        services: &["host.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "network-local",
        services: &["network.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "process",
        services: &["process.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "process-systemd",
        services: &["process-systemd.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "shell-pool",
        services: &[],
    },
    ProviderRegistration {
        provider_ref: "shell-session",
        services: &[],
    },
    ProviderRegistration {
        provider_ref: "user",
        services: &["user.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "volume-binding",
        services: &["volume-binding.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "wayland-policy",
        services: &["interaction.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "wayland-session",
        services: &[],
    },
];
