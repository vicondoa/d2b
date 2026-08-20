//! Opaque GPU effect-token and launch boundary.

use core::fmt;
use d2b_contracts_zone_session::v3::ResourceUid;

use crate::{
    authority::{
        GpuAuthorityAdmission, GpuAuthorityLease, GpuClosureProof, GpuPlatformToken,
        GpuProcessIdentity, GpuProcessObservation,
    },
    probe::{GpuDeviceSelector, GpuProbeResult},
    process::GpuProcessRole,
    workers::{GpuWorkerSpec, VideoWorkerSpec},
};

/// One Core-derived GPU device effect token.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuEffectToken([u8; 32]);

impl GpuEffectToken {
    /// Construct a token at the Core adapter boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for GpuEffectToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GpuEffectToken(<redacted>)")
    }
}

/// Opaque set of broker-resolved device grants.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuEffectTokenSet {
    tokens: Vec<GpuEffectToken>,
}

impl GpuEffectTokenSet {
    /// Construct a bounded token set supplied by Core.
    pub fn from_core(tokens: Vec<GpuEffectToken>) -> Result<Self, GpuEffectError> {
        if tokens.is_empty() || tokens.len() > 8 {
            return Err(GpuEffectError::DeviceQuotaExceeded);
        }
        Ok(Self { tokens })
    }

    /// Return the number of opaque grants.
    pub const fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Return whether no opaque grants are present.
    pub const fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

impl fmt::Debug for GpuEffectTokenSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuEffectTokenSet")
            .field("count", &self.tokens.len())
            .finish()
    }
}

/// Opaque worker LaunchTicket.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuLaunchTicket([u8; 16]);

impl GpuLaunchTicket {
    /// Construct a ticket at the Core adapter boundary.
    pub const fn from_core(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for GpuLaunchTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GpuLaunchTicket(<redacted>)")
    }
}

/// Closed GPU effect failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuEffectError {
    /// More than eight device grants were requested.
    DeviceQuotaExceeded,
    /// Core refused the opaque device open.
    OpenRejected,
    /// Core refused a worker launch.
    SpawnRejected,
    /// A worker can be retried.
    Transient,
    /// The Core probe adapter is unavailable.
    ProbeUnavailable,
    /// Restart observation could not prove one exact process.
    ProcessObservationUnavailable,
    /// The request used a different worker principal.
    WrongPrincipal,
    /// The request used a different platform identity.
    PlatformMismatch,
    /// The request used a stale Device generation or backing identity.
    StaleDeviceIdentity,
    /// A Host-global claim conflicts with another owner.
    AuthorityConflict,
    /// A restart observation was ambiguous and is quarantined.
    Quarantined,
    /// A worker closure did not prove the owned process was gone.
    CloseUnconfirmed,
    /// The frozen GPU/video wire contract diverged.
    WireContractMismatch,
}

impl GpuEffectError {
    /// Return the stable Device error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::DeviceQuotaExceeded => "device-broker-fd-quota-exceeded",
            Self::OpenRejected => "device-broker-inaccessible",
            Self::SpawnRejected => "device-worker-failed",
            Self::Transient => "transient",
            Self::ProbeUnavailable => "gpu-effect-unavailable",
            Self::ProcessObservationUnavailable => "gpu-process-observation-unavailable",
            Self::WrongPrincipal => "gpu-process-principal-mismatch",
            Self::PlatformMismatch => "gpu-platform-mismatch",
            Self::StaleDeviceIdentity => "gpu-device-identity-stale",
            Self::AuthorityConflict => "device-claim-conflict",
            Self::Quarantined => "gpu-authority-quarantined",
            Self::CloseUnconfirmed => "gpu-worker-close-unconfirmed",
            Self::WireContractMismatch => "device-wire-contract-mismatch",
        }
    }
}

impl fmt::Display for GpuEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for GpuEffectError {}

/// Core effect port for GPU worker sets.
pub trait GpuEffectPort {
    /// Open the Core-resolved GPU device grants before clone.
    fn open_devices(
        &mut self,
        device_uid: &ResourceUid,
        tokens: &GpuEffectTokenSet,
    ) -> Result<GpuLaunchTicket, GpuEffectError>;
    /// Start one worker role with its opaque LaunchTicket.
    fn start(
        &mut self,
        role: GpuProcessRole,
        ticket: &GpuLaunchTicket,
    ) -> Result<(), GpuEffectError>;
    /// Stop one worker role during finalization.
    fn stop(&mut self, role: GpuProcessRole) -> Result<(), GpuEffectError>;

    /// Probe a Core-resolved DRM selector.
    fn probe_drm_device(
        &mut self,
        _selector: &GpuDeviceSelector,
    ) -> Result<GpuProbeResult, GpuEffectError> {
        Err(GpuEffectError::ProbeUnavailable)
    }

    /// Observe one worker identity after a daemon restart.
    fn observe_process(
        &mut self,
        _identity: &GpuProcessIdentity,
    ) -> Result<GpuProcessObservation, GpuEffectError> {
        Err(GpuEffectError::ProcessObservationUnavailable)
    }

    /// Close one exact worker and return a broker proof.
    fn close_process(
        &mut self,
        role: GpuProcessRole,
        identity: &GpuProcessIdentity,
    ) -> Result<GpuClosureProof, GpuEffectError> {
        self.stop(role)?;
        Ok(GpuClosureProof::from_core(identity.clone()))
    }
}

/// Extended lifecycle port used by the production Provider path.
///
/// The Core adapter implements this trait and owns the mapping to
/// `HostGlobalAuthorityIndex`, `OpenDevice`, `SpawnRunner`, `OpenPidfd`, and
/// close/release operations. The Provider only sees opaque identities.
pub trait GpuLifecycleEffectPort {
    /// Reserve Host-global GPU authority before any effect.
    fn reserve_authority(
        &mut self,
        admission: &GpuAuthorityAdmission,
    ) -> Result<GpuAuthorityLease, GpuEffectError>;

    /// Open Core-resolved device grants before worker spawn.
    fn open_authorized_devices(
        &mut self,
        admission: &GpuAuthorityAdmission,
        tokens: &GpuEffectTokenSet,
    ) -> Result<GpuLaunchTicket, GpuEffectError>;

    /// Start a GPU or render-node worker with its signed semantic spec.
    fn start_gpu_worker(
        &mut self,
        spec: &GpuWorkerSpec,
        ticket: &GpuLaunchTicket,
        principal: &crate::authority::GpuPrincipalToken,
        platform: &GpuPlatformToken,
        generation: d2b_contracts_zone_session::v3::ResourceGeneration,
    ) -> Result<GpuProcessIdentity, GpuEffectError>;

    /// Start the separate video worker.
    fn start_video_worker(
        &mut self,
        spec: &VideoWorkerSpec,
        ticket: &GpuLaunchTicket,
        principal: &crate::authority::GpuPrincipalToken,
        platform: &GpuPlatformToken,
        generation: d2b_contracts_zone_session::v3::ResourceGeneration,
    ) -> Result<GpuProcessIdentity, GpuEffectError>;

    /// Observe one exact worker after restart.
    fn observe_worker(
        &mut self,
        identity: &GpuProcessIdentity,
    ) -> Result<GpuProcessObservation, GpuEffectError>;

    /// Close one exact worker and return its closure proof.
    fn stop_worker(
        &mut self,
        identity: &GpuProcessIdentity,
    ) -> Result<GpuClosureProof, GpuEffectError>;

    /// Release Host-global authority only after worker closure.
    fn release_authority(
        &mut self,
        lease: GpuAuthorityLease,
        closures: &[GpuClosureProof],
    ) -> Result<(), GpuEffectError>;
}
