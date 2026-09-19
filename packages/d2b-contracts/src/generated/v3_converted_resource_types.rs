// @generated
// Provenance:emitted from the per-crate `resource-types.json` declarations
// by `cargo xtask check-provider-crate-layout --fix`;the layout check's
// authority drift gate regenerates this file byte-for-byte,and refuses a
// hand edit.

/// The resource types the v3 resource runtime owns end to end (R35/F1
/// exclusive per-type partition): served only by the per-zone manager plane.
pub const V3_CONVERTED_RESOURCE_TYPES: [&str; 36] = [
    "Process",
    "EphemeralProcess",
    "Guest",
    "Volume",
    "VolumeBinding",
    "Endpoint",
    "Host",
    "User",
    "activation-nixos.d2bus.org.NixosGeneration",
    "telemetry.d2bus.org.TelemetryService",
    "telemetry.d2bus.org.TelemetryBinding",
    "Credential",
    "Network",
    "Device",
    "usb.d2bus.org.UsbService",
    "usb.d2bus.org.UsbBinding",
    "security-key.d2bus.org.SecurityKeyService",
    "security-key.d2bus.org.SecurityKeyBinding",
    "display-wayland.d2bus.org.WaylandPolicy",
    "display-wayland.d2bus.org.WaylandSession",
    "audio.d2bus.org.AudioService",
    "audio.d2bus.org.AudioBinding",
    "shell-terminal.d2bus.org.ShellPool",
    "shell-terminal.d2bus.org.ShellSession",
    "Zone",
    "ZoneLink",
    "Provider",
    "Role",
    "RoleBinding",
    "Quota",
    "EmergencyPolicy",
    "ResourceExport",
    "ResourceImport",
    "Command",
    "Operation",
    "SeccompProfile",
];
