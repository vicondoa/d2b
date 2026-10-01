// @generated
// Provenance:emitted from the per-crate `service-catalog.json` declarations
// by `cargo xtask check-provider-crate-layout --fix`;the layout check's
// drift gate regenerates this file byte-for-byte,and refuses a hand edit.

// Generated service-to-provider vocabulary for the zone-plane session
// contract. The bus is pinned provider-free,so it reads the provider refs
// here instead of depending on the owning crates' constants. The daemon's
// composition reads the same catalog rather than restating a hand table.

/// The fixed bootstrap Provider reference (the system-core Provider).
pub const BOOTSTRAP_PROVIDER_REF: &str = "Provider/system-core";

/// The fixed bootstrap Provider resource UID.
pub const BOOTSTRAP_PROVIDER_UID: &str = "11111111-1111-4111-8111-111111111111";

/// The provider reference one declared provider identity publishes.
pub fn provider_ref(identity: &str) -> Option<&'static str> {
    match identity {
        "clipboard-wayland" => Some("Provider/clipboard-wayland"),
        "config-nixos" => Some("Provider/config-nixos"),
        "display-wayland" => Some("Provider/display-wayland"),
        "notification-desktop" => Some("Provider/notification-desktop"),
        "shell-terminal" => Some("Provider/shell-terminal"),
        "system-core" => Some("Provider/system-core"),
        _ => None,
    }
}

/// The provider reference that serves one closed service package
/// on the zone-plane session contract, when a fixed provider serves it.
pub fn provider_ref_for_service(service: &str) -> Option<&'static str> {
    match service {
        "d2b.clipboard.bridge.v3" => Some("Provider/clipboard-wayland"),
        "d2b.clipboard.picker-coord.v3" => Some("Provider/clipboard-wayland"),
        "d2b.clipboard.v3" => Some("Provider/clipboard-wayland"),
        "d2b.config-nixos.v3" => Some("Provider/config-nixos"),
        "d2b.display.v3" => Some("Provider/display-wayland"),
        "d2b.notification.v3" => Some("Provider/notification-desktop"),
        _ => None,
    }
}
