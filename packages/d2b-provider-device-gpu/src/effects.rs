//! Opaque GPU effect-token and launch boundary.

use core::fmt;
use d2b_contracts::v3::ResourceUid;

use crate::process::GpuProcessRole;

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
}

impl GpuEffectError {
    /// Return the stable Device error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::DeviceQuotaExceeded => "device-broker-fd-quota-exceeded",
            Self::OpenRejected => "device-broker-inaccessible",
            Self::SpawnRejected => "device-worker-failed",
            Self::Transient => "transient",
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
}
