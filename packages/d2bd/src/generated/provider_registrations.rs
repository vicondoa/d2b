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
        provider_ref: "network-local",
        services: &["network.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "process",
        services: &["process.d2bus.org/effects"],
    },
    ProviderRegistration {
        provider_ref: "volume",
        services: &["volume.d2bus.org/effects"],
    },
];
