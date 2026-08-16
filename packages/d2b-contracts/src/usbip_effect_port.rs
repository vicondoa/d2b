//! Typed semantic effect ports for the USBIP Provider.
//!
//! The Provider receives opaque identities and leases only.  Core resolves
//! those values to signed bundle rows and the daemon adapts the semantic
//! requests to the broker.  No bus id, path, address, argv, descriptor, or
//! broker wire type crosses this boundary.

use core::fmt;

use crate::v3::{ResourceBundleGenerationId, ResourceUid};

/// Opaque Device identity accepted from Core.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceUid(ResourceUid);

impl DeviceUid {
    /// Construct a Device identity at the trusted Core boundary.
    pub const fn from_core(value: ResourceUid) -> Self {
        Self(value)
    }

    /// Borrow the opaque identity for adapter routing.
    pub const fn as_resource_uid(&self) -> &ResourceUid {
        &self.0
    }
}

impl fmt::Debug for DeviceUid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeviceUid(<redacted>)")
    }
}

/// Opaque Network identity accepted from Core.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetworkUid(ResourceUid);

impl NetworkUid {
    /// Construct a Network identity at the trusted Core boundary.
    pub const fn from_core(value: ResourceUid) -> Self {
        Self(value)
    }

    /// Borrow the opaque identity for adapter routing.
    pub const fn as_resource_uid(&self) -> &ResourceUid {
        &self.0
    }
}

impl fmt::Debug for NetworkUid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NetworkUid(<redacted>)")
    }
}

/// Opaque Binding identity accepted from Core.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UsbBindingUid(ResourceUid);

impl UsbBindingUid {
    /// Construct a Binding identity at the trusted Core boundary.
    pub const fn from_core(value: ResourceUid) -> Self {
        Self(value)
    }

    /// Borrow the opaque identity for adapter routing.
    pub const fn as_resource_uid(&self) -> &ResourceUid {
        &self.0
    }
}

impl fmt::Debug for UsbBindingUid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UsbBindingUid(<redacted>)")
    }
}

/// Core-derived physical USB backing identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalUsbBacking([u8; 32]);

impl PhysicalUsbBacking {
    /// Construct a backing identity at the trusted Core boundary.
    pub const fn from_core(value: [u8; 32]) -> Self {
        Self(value)
    }

    /// Borrow the opaque digest for authority indexing.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for PhysicalUsbBacking {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PhysicalUsbBacking(<redacted>)")
    }
}

/// Opaque authority lease retained until the corresponding effect closes.
#[derive(Clone, PartialEq, Eq)]
pub struct LeaseToken([u8; 16]);

impl LeaseToken {
    /// Construct a lease at the trusted adapter boundary.
    pub const fn from_adapter(value: [u8; 16]) -> Self {
        Self(value)
    }
}

impl fmt::Debug for LeaseToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LeaseToken(<redacted>)")
    }
}

/// Opaque ownership token for one firewall projection.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallToken([u8; 16]);

impl FirewallToken {
    /// Construct a token at the trusted adapter boundary.
    pub const fn from_adapter(value: [u8; 16]) -> Self {
        Self(value)
    }
}

impl fmt::Debug for FirewallToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FirewallToken(<redacted>)")
    }
}

/// Opaque digest of the installed firewall projection.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallDigest([u8; 32]);

impl FirewallDigest {
    /// Construct a digest at the trusted adapter boundary.
    pub const fn from_adapter(value: [u8; 32]) -> Self {
        Self(value)
    }
}

impl fmt::Debug for FirewallDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FirewallDigest(<redacted>)")
    }
}

/// Immutable generation fence for one firewall projection.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallGenerationFence(ResourceBundleGenerationId);

impl FirewallGenerationFence {
    /// Bind a projection to the installed bundle generation observed by Core.
    pub const fn from_core(value: ResourceBundleGenerationId) -> Self {
        Self(value)
    }

    /// Borrow the expected installed generation for broker dispatch.
    pub const fn generation(&self) -> &ResourceBundleGenerationId {
        &self.0
    }
}

impl fmt::Debug for FirewallGenerationFence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FirewallGenerationFence(<redacted>)")
    }
}

/// Closed firewall projection direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallAction {
    /// Install or converge the owned projection.
    Apply,
    /// Remove only the owned projection.
    Remove,
}

/// Exact ownership-scoped firewall projection.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallProjection {
    network: NetworkUid,
    binding: UsbBindingUid,
    expected: FirewallGenerationFence,
    action: FirewallAction,
}

impl FirewallProjection {
    /// Construct a projection after Core has resolved its exact scope.
    pub const fn new(
        network: NetworkUid,
        binding: UsbBindingUid,
        expected: FirewallGenerationFence,
        action: FirewallAction,
    ) -> Self {
        Self {
            network,
            binding,
            expected,
            action,
        }
    }

    /// Borrow the opaque Network identity.
    pub const fn network(&self) -> &NetworkUid {
        &self.network
    }

    /// Borrow the opaque Binding identity.
    pub const fn binding(&self) -> &UsbBindingUid {
        &self.binding
    }

    /// Borrow the installed-generation fence.
    pub const fn expected(&self) -> &FirewallGenerationFence {
        &self.expected
    }

    /// Return the closed action.
    pub const fn action(&self) -> FirewallAction {
        self.action
    }
}

impl fmt::Debug for FirewallProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FirewallProjection")
            .field("action", &self.action)
            .field("expected", &self.expected)
            .finish()
    }
}

/// Ownership-scoped observation of one firewall projection.
#[derive(Clone, PartialEq, Eq)]
pub struct FirewallObservation {
    matches_expected: bool,
    digest: FirewallDigest,
}

impl FirewallObservation {
    /// Construct an adapter-owned observation.
    pub const fn from_adapter(matches_expected: bool, digest: FirewallDigest) -> Self {
        Self {
            matches_expected,
            digest,
        }
    }

    /// Whether the projection matches the expected generation and ownership.
    pub const fn matches_expected(&self) -> bool {
        self.matches_expected
    }

    /// Borrow the opaque observed digest.
    pub const fn digest(&self) -> &FirewallDigest {
        &self.digest
    }
}

impl fmt::Debug for FirewallObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FirewallObservation")
            .field("matches_expected", &self.matches_expected)
            .field("digest", &self.digest)
            .finish()
    }
}

/// Kernel-backed USBIP authority class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelModuleClass {
    /// The host-side usbip-host module.
    UsbipHost,
}

/// Result of a Core-owned physical-device probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceProbeResult {
    /// The declared backing is present and may be claimed.
    Present,
    /// The backing is absent and no effect may start.
    Missing,
    /// The observed device is not the declared backing.
    Mismatch,
    /// Multiple observations matched and must be quarantined.
    Ambiguous,
}

/// Closed retry detail with no caller-controlled payload.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TransientDetail {
    /// The broker or host effect is temporarily unavailable.
    EffectUnavailable,
    /// The installed generation must be refreshed.
    StaleGeneration,
    /// The backing is being drained by another close operation.
    DrainInProgress,
}

impl fmt::Debug for TransientDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TransientDetail(<redacted>)")
    }
}

impl fmt::Display for TransientDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("transient")
    }
}

/// Closed USBIP effect failures.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UsbipEffectError {
    /// The resource and requested Zone differ.
    WrongZone,
    /// A physical USB backing is already owned.
    PhysicalBackingConflict,
    /// The host usbip module is already owned by an incompatible lifecycle.
    HostModuleConflict,
    /// A Network relay is already owned by another Service.
    RelayAuthorityConflict,
    /// The device is absent or does not match the signed backing.
    DeviceUnavailable,
    /// The host topology does not prove anti-spoofing.
    AntiSpoofFailed,
    /// The broker rejected the typed effect.
    EffectRejected,
    /// A foreign ownership marker blocked mutation.
    ForeignOwnership,
    /// The installed generation changed before the effect ran.
    FirewallGenerationMismatch,
    /// The operation may be retried without releasing authority.
    Transient(TransientDetail),
}

impl fmt::Debug for UsbipEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongZone => "UsbipEffectError::WrongZone",
            Self::PhysicalBackingConflict => "UsbipEffectError::PhysicalBackingConflict",
            Self::HostModuleConflict => "UsbipEffectError::HostModuleConflict",
            Self::RelayAuthorityConflict => "UsbipEffectError::RelayAuthorityConflict",
            Self::DeviceUnavailable => "UsbipEffectError::DeviceUnavailable",
            Self::AntiSpoofFailed => "UsbipEffectError::AntiSpoofFailed",
            Self::EffectRejected => "UsbipEffectError::EffectRejected",
            Self::ForeignOwnership => "UsbipEffectError::ForeignOwnership",
            Self::FirewallGenerationMismatch => "UsbipEffectError::FirewallGenerationMismatch",
            Self::Transient(_) => "UsbipEffectError::Transient(<redacted>)",
        })
    }
}

impl fmt::Display for UsbipEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongZone => "wrong-zone",
            Self::PhysicalBackingConflict => "physical-usb-backing-conflict",
            Self::HostModuleConflict => "usbip-host-authority-conflict",
            Self::RelayAuthorityConflict => "usbip-network-relay-authority-conflict",
            Self::DeviceUnavailable => "usb-device-unavailable",
            Self::AntiSpoofFailed => "usbip-anti-spoof-failed",
            Self::EffectRejected => "effect-rejected",
            Self::ForeignOwnership => "foreign-ownership",
            Self::FirewallGenerationMismatch => "firewall-generation-mismatch",
            Self::Transient(_) => "transient",
        })
    }
}

impl std::error::Error for UsbipEffectError {}

/// Successful firewall mutation.
#[derive(Clone, PartialEq, Eq)]
pub enum FirewallConfirmation {
    /// Apply confirmed with an ownership token and digest.
    Applied {
        /// Token retained until Remove confirms closure.
        token: FirewallToken,
        /// Digest of the installed projection.
        digest: FirewallDigest,
    },
    /// Remove confirmed.
    Removed,
    /// Remove confirmed the projection was already absent.
    ValidatedAbsent,
}

impl fmt::Debug for FirewallConfirmation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Applied { .. } => "FirewallConfirmation::Applied(<redacted>)",
            Self::Removed => "FirewallConfirmation::Removed",
            Self::ValidatedAbsent => "FirewallConfirmation::ValidatedAbsent",
        })
    }
}

/// Verified identity of a broker-spawned attach process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachProcessIdentity {
    /// Kernel pid used only for pidfd lookup inside the daemon.
    pub pid: u32,
    /// Kernel start time used to reject pid reuse.
    pub start_time: u64,
}

/// Typed Core-to-Provider Service effect boundary.
pub trait UsbipEffectPort {
    /// Reserve the physical USB backing before any module, bind, or relay effect.
    fn claim_physical_backing(
        &mut self,
        device: &DeviceUid,
        backing: &PhysicalUsbBacking,
    ) -> Result<LeaseToken, UsbipEffectError>;

    /// Reserve the host module authority before any attach effect.
    fn claim_host_module(
        &mut self,
        class: KernelModuleClass,
    ) -> Result<LeaseToken, UsbipEffectError>;

    /// Reserve the one shared relay authority for a Network.
    fn claim_relay(&mut self, network: &NetworkUid) -> Result<LeaseToken, UsbipEffectError>;

    /// Apply or remove one exact Network/Binding projection.
    fn mutate_firewall(
        &mut self,
        projection: &FirewallProjection,
        token: Option<&FirewallToken>,
    ) -> Result<FirewallConfirmation, UsbipEffectError>;

    /// Observe one exact projection without mutating it.
    fn observe_firewall(
        &mut self,
        projection: &FirewallProjection,
        token: &FirewallToken,
    ) -> Result<FirewallObservation, UsbipEffectError>;

    /// Unbind the exact physical device after every Binding process closed.
    fn unbind(&mut self, device: &DeviceUid, physical: &LeaseToken)
    -> Result<(), UsbipEffectError>;

    /// Release relay authority after the projection is confirmed removed.
    fn release_relay(
        &mut self,
        network: &NetworkUid,
        lease: LeaseToken,
    ) -> Result<(), UsbipEffectError>;

    /// Release physical and module authority after unbind and process close.
    fn release_authority(
        &mut self,
        device: &DeviceUid,
        physical: LeaseToken,
        module: LeaseToken,
    ) -> Result<(), UsbipEffectError>;
}

/// Typed Core-to-Provider Binding effect boundary.
pub trait UsbipGuestEffectPort {
    /// Start the private Binding proxy after Service admission.
    fn start_proxy(
        &mut self,
        binding: &UsbBindingUid,
        service_lease: &LeaseToken,
    ) -> Result<LeaseToken, UsbipEffectError>;

    /// Attach through the daemon's brokered SpawnRunner path.
    fn spawn_attach(
        &mut self,
        binding: &UsbBindingUid,
        proxy: &LeaseToken,
    ) -> Result<AttachProcessIdentity, UsbipEffectError>;

    /// Verify a persisted attach runner by pidfd/start-time identity.
    fn observe_attach(
        &mut self,
        binding: &UsbBindingUid,
        identity: AttachProcessIdentity,
    ) -> Result<DeviceProbeResult, UsbipEffectError>;

    /// Detach the Guest before closing the attach process.
    fn detach_guest(
        &mut self,
        binding: &UsbBindingUid,
        proxy: &LeaseToken,
    ) -> Result<(), UsbipEffectError>;

    /// Close only the exact Binding-owned runner.
    fn close_attach(
        &mut self,
        binding: &UsbBindingUid,
        identity: AttachProcessIdentity,
    ) -> Result<(), UsbipEffectError>;

    /// Close only the exact Binding-owned proxy.
    fn close_proxy(
        &mut self,
        binding: &UsbBindingUid,
        proxy: LeaseToken,
    ) -> Result<(), UsbipEffectError>;
}
