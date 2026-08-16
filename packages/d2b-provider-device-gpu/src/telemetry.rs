//! Closed GPU telemetry labels.

use crate::GpuEffectError;

/// Closed GPU controller operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuOperation {
    /// Reserve Host-global authority.
    ReserveAuthority,
    /// Probe the physical device.
    Probe,
    /// Start a GPU worker.
    StartGpu,
    /// Start a video worker.
    StartVideo,
    /// Adopt a worker after restart.
    Adopt,
    /// Close a worker.
    Close,
    /// Release Host-global authority.
    ReleaseAuthority,
}

impl GpuOperation {
    /// Return the stable operation label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReserveAuthority => "reserve-authority",
            Self::Probe => "probe",
            Self::StartGpu => "start-gpu",
            Self::StartVideo => "start-video",
            Self::Adopt => "adopt",
            Self::Close => "close",
            Self::ReleaseAuthority => "release-authority",
        }
    }
}

/// Closed GPU operation outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuOutcome {
    /// The operation converged.
    Success,
    /// The operation can be retried.
    Retry,
    /// The operation was refused fail closed.
    Blocked,
}

impl GpuOutcome {
    /// Return the stable outcome label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Retry => "retry",
            Self::Blocked => "blocked",
        }
    }
}

/// Bounded metric label projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuMetricLabels {
    /// Fixed Provider label.
    pub provider: &'static str,
    /// Fixed component label.
    pub component: &'static str,
    /// Closed operation label.
    pub operation: &'static str,
    /// Closed outcome label.
    pub outcome: &'static str,
    /// Stable error slug or `none`.
    pub error: &'static str,
    /// Closed realization mode.
    pub mode: &'static str,
    /// Closed video-sidecar state.
    pub video_sidecar: &'static str,
    /// Closed arbitration state.
    pub arbitration: &'static str,
}

impl GpuMetricLabels {
    /// Build labels without accepting resource, Zone, selector, or process
    /// identity values.
    pub const fn new(
        operation: GpuOperation,
        outcome: GpuOutcome,
        error: Option<GpuEffectError>,
        mode: &'static str,
        video_sidecar: bool,
        arbitration: &'static str,
    ) -> Self {
        Self {
            provider: "device-gpu",
            component: "device-controller",
            operation: operation.as_str(),
            outcome: outcome.as_str(),
            error: match error {
                Some(error) => error.code(),
                None => "none",
            },
            mode,
            video_sidecar: if video_sidecar { "enabled" } else { "disabled" },
            arbitration,
        }
    }
}
