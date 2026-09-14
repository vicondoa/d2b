//! Host-global GPU authority admission contracts.
//!
//! Core creates the opaque values in this module after resolving the trusted
//! device inventory. The Provider can compare those values and retain the
//! resulting lease, but it cannot derive one from a path, selector, or
//! process identifier.

use core::fmt;

use d2b_contracts_resource::v3::{
    ResourceGeneration, ResourceRef, ResourceUid, device::DeviceArbitration,
};

use crate::process::GpuProcessRole;

/// Core-derived identity for one physical GPU or render node backing.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GpuBackingToken([u8; 32]);

impl GpuBackingToken {
    /// Construct a backing token at the trusted Core boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Whether the token is the forbidden all-zero identity.
    pub fn is_zero(&self) -> bool {
        self.0 == [0; 32]
    }

    /// Borrow the token for another trusted adapter comparison.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for GpuBackingToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GpuBackingToken(<redacted>)")
    }
}

/// Core-derived platform identity for one GPU effect.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GpuPlatformToken([u8; 32]);

impl GpuPlatformToken {
    /// Construct a platform token at the trusted Core boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Whether the token is the forbidden all-zero identity.
    pub fn is_zero(&self) -> bool {
        self.0 == [0; 32]
    }
}

impl fmt::Debug for GpuPlatformToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GpuPlatformToken(<redacted>)")
    }
}

/// Core-assigned worker principal.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GpuPrincipalToken([u8; 32]);

impl GpuPrincipalToken {
    /// Construct a principal token at the trusted Core boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Whether the token is the forbidden all-zero identity.
    pub fn is_zero(&self) -> bool {
        self.0 == [0; 32]
    }
}

impl fmt::Debug for GpuPrincipalToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GpuPrincipalToken(<redacted>)")
    }
}

/// Opaque proof that a Device owner is authorized to hold GPU authority.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuOwnerProof {
    zone_ref: ResourceRef,
    holder_ref: ResourceRef,
    device_uid: ResourceUid,
    host_uid: ResourceUid,
    generation: ResourceGeneration,
}

impl GpuOwnerProof {
    /// Bind a proof to an exact Zone, holder, Device, Host, and generation.
    pub fn new(
        zone_ref: ResourceRef,
        holder_ref: ResourceRef,
        device_uid: ResourceUid,
        host_uid: ResourceUid,
        generation: ResourceGeneration,
    ) -> Result<Self, GpuAuthorityError> {
        if zone_ref.resource_type().as_str() != "Zone"
            || !matches!(holder_ref.resource_type().as_str(), "Guest" | "Host")
        {
            return Err(GpuAuthorityError::WrongPrincipal);
        }
        Ok(Self {
            zone_ref,
            holder_ref,
            device_uid,
            host_uid,
            generation,
        })
    }

    /// Borrow the exact Zone reference.
    pub const fn zone_ref(&self) -> &ResourceRef {
        &self.zone_ref
    }

    /// Borrow the exact holder reference.
    pub const fn holder_ref(&self) -> &ResourceRef {
        &self.holder_ref
    }

    /// Borrow the Device UID.
    pub const fn device_uid(&self) -> &ResourceUid {
        &self.device_uid
    }

    /// Borrow the Host UID.
    pub const fn host_uid(&self) -> &ResourceUid {
        &self.host_uid
    }

    /// Return the admitted resource generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }
}

impl fmt::Debug for GpuOwnerProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GpuOwnerProof(<redacted>)")
    }
}

/// Core-issued GPU authority admission.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuAuthorityAdmission {
    owner: GpuOwnerProof,
    backing: GpuBackingToken,
    platform: GpuPlatformToken,
    arbitration: DeviceArbitration,
    max_holders: u32,
    render_node_only: bool,
    gpu_principal: GpuPrincipalToken,
    video_principal: Option<GpuPrincipalToken>,
}

impl GpuAuthorityAdmission {
    /// Construct an admission before any device or process effect.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner: GpuOwnerProof,
        backing: GpuBackingToken,
        platform: GpuPlatformToken,
        arbitration: DeviceArbitration,
        max_holders: u32,
        render_node_only: bool,
        gpu_principal: GpuPrincipalToken,
    ) -> Result<Self, GpuAuthorityError> {
        if backing.is_zero() || platform.is_zero() || gpu_principal.is_zero() {
            return Err(GpuAuthorityError::StaleDeviceIdentity);
        }
        if !(1..=16).contains(&max_holders)
            || (arbitration == DeviceArbitration::Exclusive && max_holders != 1)
            || (arbitration == DeviceArbitration::Shared && !render_node_only)
            || (arbitration == DeviceArbitration::Exclusive && render_node_only && max_holders != 1)
        {
            return Err(GpuAuthorityError::ArbitrationViolation);
        }
        Ok(Self {
            owner,
            backing,
            platform,
            arbitration,
            max_holders,
            render_node_only,
            gpu_principal,
            video_principal: None,
        })
    }

    /// Attach the distinct Core-assigned video principal.
    pub fn with_video_principal(
        mut self,
        video_principal: GpuPrincipalToken,
    ) -> Result<Self, GpuAuthorityError> {
        if video_principal.is_zero() || video_principal == self.gpu_principal {
            return Err(GpuAuthorityError::PrincipalNotSeparated);
        }
        self.video_principal = Some(video_principal);
        Ok(self)
    }

    /// Borrow the exact owner proof.
    pub const fn owner(&self) -> &GpuOwnerProof {
        &self.owner
    }

    /// Borrow the opaque backing identity.
    pub const fn backing(&self) -> &GpuBackingToken {
        &self.backing
    }

    /// Borrow the opaque platform identity.
    pub const fn platform(&self) -> &GpuPlatformToken {
        &self.platform
    }

    /// Return the requested arbitration.
    pub const fn arbitration(&self) -> DeviceArbitration {
        self.arbitration
    }

    /// Return the signed holder ceiling.
    pub const fn max_holders(&self) -> u32 {
        self.max_holders
    }

    /// Whether this is render-node-only authority.
    pub const fn render_node_only(&self) -> bool {
        self.render_node_only
    }

    /// Borrow the GPU worker principal.
    pub const fn gpu_principal(&self) -> &GpuPrincipalToken {
        &self.gpu_principal
    }

    /// Borrow the distinct video worker principal, when configured.
    pub const fn video_principal(&self) -> Option<&GpuPrincipalToken> {
        self.video_principal.as_ref()
    }
}

impl fmt::Debug for GpuAuthorityAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuAuthorityAdmission")
            .field("arbitration", &self.arbitration)
            .field("max_holders", &self.max_holders)
            .field("render_node_only", &self.render_node_only)
            .field("has_video_principal", &self.video_principal.is_some())
            .finish()
    }
}

/// Opaque Host-global GPU lease.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuAuthorityLease([u8; 16]);

impl GpuAuthorityLease {
    /// Construct a lease at the trusted authority adapter boundary.
    pub const fn from_core(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Borrow the opaque lease token at the daemon adapter boundary.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Debug for GpuAuthorityLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GpuAuthorityLease(<redacted>)")
    }
}

/// Opaque identity of one broker-supervised GPU worker.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuProcessIdentity {
    process_token: [u8; 16],
    role: GpuProcessRole,
    principal: GpuPrincipalToken,
    platform: GpuPlatformToken,
    generation: ResourceGeneration,
}

impl GpuProcessIdentity {
    /// Construct a verified process identity at the broker boundary.
    pub const fn from_core(
        process_token: [u8; 16],
        role: GpuProcessRole,
        principal: GpuPrincipalToken,
        platform: GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> Self {
        Self {
            process_token,
            role,
            principal,
            platform,
            generation,
        }
    }

    /// Return the worker role.
    pub const fn role(&self) -> GpuProcessRole {
        self.role
    }

    /// Borrow the worker principal.
    pub const fn principal(&self) -> &GpuPrincipalToken {
        &self.principal
    }

    /// Borrow the platform identity.
    pub const fn platform(&self) -> &GpuPlatformToken {
        &self.platform
    }

    /// Return the resource generation bound to this worker.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }
}

impl fmt::Debug for GpuProcessIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuProcessIdentity")
            .field("role", &self.role)
            .field("generation", &self.generation)
            .finish()
    }
}

/// Broker proof that one exact worker process has closed.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuClosureProof {
    identity: GpuProcessIdentity,
}

impl GpuClosureProof {
    /// Construct a closure proof at the broker boundary.
    pub fn from_core(identity: GpuProcessIdentity) -> Self {
        Self { identity }
    }

    /// Borrow the closed process identity for exact lease matching.
    pub const fn identity(&self) -> &GpuProcessIdentity {
        &self.identity
    }
}

impl fmt::Debug for GpuClosureProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GpuClosureProof(<redacted>)")
    }
}

/// Observation used by restart adoption.
#[derive(Clone, PartialEq, Eq)]
pub enum GpuProcessObservation {
    /// Exactly one process matched the expected identity.
    Matching(GpuProcessIdentity),
    /// No process with the expected identity was found.
    Missing,
    /// The identity was reused or could not be verified.
    StaleIdentity,
    /// More than one process matched the expected identity.
    Ambiguous,
}

impl fmt::Debug for GpuProcessObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Matching(_) => "GpuProcessObservation::Matching",
            Self::Missing => "GpuProcessObservation::Missing",
            Self::StaleIdentity => "GpuProcessObservation::StaleIdentity",
            Self::Ambiguous => "GpuProcessObservation::Ambiguous",
        })
    }
}

/// Stable GPU authority errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuAuthorityError {
    /// A claim used a wrong holder or Zone reference.
    WrongPrincipal,
    /// GPU and video workers attempted to share one principal.
    PrincipalNotSeparated,
    /// A physical identity was zero or no longer current.
    StaleDeviceIdentity,
    /// The arbitration and render-node settings disagree.
    ArbitrationViolation,
    /// A full-device or render-node claim conflicts with an owner.
    ClaimConflict,
    /// The signed shared-holder ceiling was reached.
    MaxClaimsExceeded,
    /// The authority index has not completed restart rehydration.
    StartupRehydrationRequired,
    /// The same owner already holds the authority.
    DuplicateActiveReservation,
    /// A process observation used the wrong principal.
    ProcessPrincipalMismatch,
    /// A process observation used the wrong platform identity.
    PlatformMismatch,
    /// A process observation used an old resource generation.
    GenerationMismatch,
    /// The exact owner proof did not match the retained lease.
    OwnerProofMismatch,
    /// A close proof was missing or named another process.
    CloseUnconfirmed,
    /// A quarantined key cannot admit a new effect.
    Quarantined,
}

impl GpuAuthorityError {
    /// Return the stable, identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::WrongPrincipal => "gpu-authority-principal-denied",
            Self::PrincipalNotSeparated => "gpu-principal-not-separated",
            Self::StaleDeviceIdentity => "gpu-device-identity-stale",
            Self::ArbitrationViolation => "gpu-arbitration-violation",
            Self::ClaimConflict => "device-claim-conflict",
            Self::MaxClaimsExceeded => "device-claim-max-exceeded",
            Self::StartupRehydrationRequired => "authority-startup-rehydration-required",
            Self::DuplicateActiveReservation => "authority-duplicate-active-reservation",
            Self::ProcessPrincipalMismatch => "gpu-process-principal-mismatch",
            Self::PlatformMismatch => "gpu-platform-mismatch",
            Self::GenerationMismatch => "gpu-device-generation-stale",
            Self::OwnerProofMismatch => "gpu-authority-owner-proof-mismatch",
            Self::CloseUnconfirmed => "gpu-worker-close-unconfirmed",
            Self::Quarantined => "gpu-authority-quarantined",
        }
    }
}

impl fmt::Display for GpuAuthorityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for GpuAuthorityError {}
