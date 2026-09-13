//! The well-known resource type names the v3 resource plane recognizes.

use d2b_resource_runtime::identity::ResourceTypeName;

/// One well-known resource type name.
///
/// The name is stored as a `&'static str` so every constant is
/// `const`-constructible: [`ResourceTypeName`] wraps an owned `String` and
/// cannot be built in a const context, while a declaration table has to be a
/// plain constant. [`WellKnownType::to_resource_type_name`] converts into the
/// runtime's owned name at the call site that needs one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WellKnownType(&'static str);

impl WellKnownType {
    /// A configured launcher process.
    pub const PROCESS: Self = Self("Process");
    /// A one-shot process created for a single invocation.
    pub const EPHEMERAL_PROCESS: Self = Self("EphemeralProcess");
    /// A workload or nested guest machine.
    pub const GUEST: Self = Self("Guest");
    /// A durable storage volume.
    pub const VOLUME: Self = Self("Volume");
    /// The binding that attaches a volume to a workload.
    pub const VOLUME_BINDING: Self = Self("VolumeBinding");
    /// A reachable transport endpoint.
    pub const ENDPOINT: Self = Self("Endpoint");
    /// The physical host target.
    pub const HOST: Self = Self("Host");
    /// A host user.
    pub const USER: Self = Self("User");
    /// A NixOS activation generation owned by the activation provider.
    pub const NIXOS_GENERATION: Self = Self("activation-nixos.d2bus.org.NixosGeneration");
    /// A telemetry service instance.
    pub const TELEMETRY_SERVICE: Self = Self("telemetry.d2bus.org.TelemetryService");
    /// The binding that attaches a telemetry service to a resource.
    pub const TELEMETRY_BINDING: Self = Self("telemetry.d2bus.org.TelemetryBinding");
    /// A managed credential material set.
    pub const CREDENTIAL: Self = Self("Credential");
    /// A network interface resource.
    pub const NETWORK: Self = Self("Network");
    /// A host device backing.
    pub const DEVICE: Self = Self("Device");
    /// A USB/IP service instance.
    pub const USB_SERVICE: Self = Self("usb.d2bus.org.UsbService");
    /// The binding that attaches a USB backing to a guest.
    pub const USB_BINDING: Self = Self("usb.d2bus.org.UsbBinding");
    /// A security-key service instance.
    pub const SECURITY_KEY_SERVICE: Self = Self("security-key.d2bus.org.SecurityKeyService");
    /// The binding that attaches a security key to a guest.
    pub const SECURITY_KEY_BINDING: Self = Self("security-key.d2bus.org.SecurityKeyBinding");
    /// The Wayland display policy of a zone.
    pub const WAYLAND_POLICY: Self = Self("display-wayland.d2bus.org.WaylandPolicy");
    /// A Wayland display session.
    pub const WAYLAND_SESSION: Self = Self("display-wayland.d2bus.org.WaylandSession");
    /// An audio service instance.
    pub const AUDIO_SERVICE: Self = Self("audio.d2bus.org.AudioService");
    /// The binding that attaches an audio backing to a guest.
    pub const AUDIO_BINDING: Self = Self("audio.d2bus.org.AudioBinding");
    /// A pool of persistent shell sessions.
    pub const SHELL_POOL: Self = Self("shell-terminal.d2bus.org.ShellPool");
    /// A persistent shell session.
    pub const SHELL_SESSION: Self = Self("shell-terminal.d2bus.org.ShellSession");
    /// A zone.
    pub const ZONE: Self = Self("Zone");
    /// A link between two zones.
    pub const ZONE_LINK: Self = Self("ZoneLink");
    /// A provider identity.
    pub const PROVIDER: Self = Self("Provider");
    /// An authority role.
    pub const ROLE: Self = Self("Role");
    /// A role binding.
    pub const ROLE_BINDING: Self = Self("RoleBinding");
    /// A quota.
    pub const QUOTA: Self = Self("Quota");
    /// An emergency policy.
    pub const EMERGENCY_POLICY: Self = Self("EmergencyPolicy");
    /// A resource exported to another zone.
    pub const RESOURCE_EXPORT: Self = Self("ResourceExport");
    /// A resource imported from another zone.
    pub const RESOURCE_IMPORT: Self = Self("ResourceImport");
    /// A declared launch shape a `Process` instance references.
    pub const COMMAND: Self = Self("Command");
    /// A committed broker operation with its handler reference.
    pub const OPERATION: Self = Self("Operation");
    /// A committed seccomp posture a role references.
    pub const SECCOMP_PROFILE: Self = Self("SeccompProfile");

    /// Every well-known type, in the order of the v3 converted-type authority
    /// list.
    pub const ALL: &'static [Self] = &[
        Self::PROCESS,
        Self::EPHEMERAL_PROCESS,
        Self::GUEST,
        Self::VOLUME,
        Self::VOLUME_BINDING,
        Self::ENDPOINT,
        Self::HOST,
        Self::USER,
        Self::NIXOS_GENERATION,
        Self::TELEMETRY_SERVICE,
        Self::TELEMETRY_BINDING,
        Self::CREDENTIAL,
        Self::NETWORK,
        Self::DEVICE,
        Self::USB_SERVICE,
        Self::USB_BINDING,
        Self::SECURITY_KEY_SERVICE,
        Self::SECURITY_KEY_BINDING,
        Self::WAYLAND_POLICY,
        Self::WAYLAND_SESSION,
        Self::AUDIO_SERVICE,
        Self::AUDIO_BINDING,
        Self::SHELL_POOL,
        Self::SHELL_SESSION,
        Self::ZONE,
        Self::ZONE_LINK,
        Self::PROVIDER,
        Self::ROLE,
        Self::ROLE_BINDING,
        Self::QUOTA,
        Self::EMERGENCY_POLICY,
        Self::RESOURCE_EXPORT,
        Self::RESOURCE_IMPORT,
        Self::COMMAND,
        Self::OPERATION,
        Self::SECCOMP_PROFILE,
    ];

    /// Convert to the runtime's owned resource type name.
    pub fn to_resource_type_name(&self) -> ResourceTypeName {
        ResourceTypeName::new(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::WellKnownType;
    use d2b_contracts_resource::v3::V3_CONVERTED_RESOURCE_TYPES;
    use d2b_resource_runtime::identity::ResourceTypeName;

    /// No two entries name the same resource type.
    #[test]
    fn every_well_known_type_is_distinct() {
        for (index, entry) in WellKnownType::ALL.iter().enumerate() {
            for other in &WellKnownType::ALL[index + 1..] {
                assert_ne!(entry, other, "duplicate well-known type in ALL");
            }
        }
    }

    /// `ALL` is the authority list: same length, same names, same order.
    #[test]
    fn all_covers_the_converted_type_authority_list() {
        assert_eq!(WellKnownType::ALL.len(), 36);
        assert_eq!(WellKnownType::ALL.len(), V3_CONVERTED_RESOURCE_TYPES.len());
        for (entry, authority) in WellKnownType::ALL.iter().zip(V3_CONVERTED_RESOURCE_TYPES) {
            assert_eq!(entry.to_resource_type_name().as_str(), authority);
        }
    }

    /// A spot check that the stored name survives the conversion into the
    /// runtime's owned name unchanged.
    #[test]
    fn names_round_trip_through_the_runtime_name() {
        assert_eq!(
            WellKnownType::PROCESS.to_resource_type_name(),
            ResourceTypeName::new("Process")
        );
        assert_eq!(
            WellKnownType::NIXOS_GENERATION.to_resource_type_name().as_str(),
            "activation-nixos.d2bus.org.NixosGeneration"
        );
    }
}
