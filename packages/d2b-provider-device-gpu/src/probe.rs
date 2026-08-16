//! Opaque DRM probe and bounded observe scheduling.

use core::fmt;

use crate::authority::{GpuBackingToken, GpuPlatformToken};

/// Maximum GPU observe interval in seconds.
pub const MAX_OBSERVE_INTERVAL_SECS: u64 = 60;
/// Minimum GPU observe interval in seconds.
pub const MIN_OBSERVE_INTERVAL_SECS: u64 = 10;
/// Default GPU observe interval in seconds.
pub const DEFAULT_OBSERVE_INTERVAL_SECS: u64 = 30;

/// Stable DRM inventory selector.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuDeviceSelector {
    label: String,
    pci_slot: Option<String>,
}

impl GpuDeviceSelector {
    /// Construct a selector without accepting a host path or bus address.
    pub fn new(
        label: impl Into<String>,
        pci_slot: Option<impl Into<String>>,
    ) -> Result<Self, GpuProbeError> {
        let label = label.into();
        if label.is_empty()
            || label.len() > 63
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || !label.as_bytes()[0].is_ascii_lowercase()
        {
            return Err(GpuProbeError::InvalidSelector);
        }
        let pci_slot = pci_slot.map(Into::into);
        if pci_slot.as_deref().is_some_and(|slot| {
            slot.is_empty()
                || slot.len() > 31
                || slot
                    .bytes()
                    .any(|byte| !byte.is_ascii_graphic() || byte == b'/')
        }) {
            return Err(GpuProbeError::InvalidSelector);
        }
        Ok(Self { label, pci_slot })
    }

    /// Borrow the stable selector label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Borrow the optional PCI filter.
    pub fn pci_slot(&self) -> Option<&str> {
        self.pci_slot.as_deref()
    }
}

impl fmt::Debug for GpuDeviceSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuDeviceSelector")
            .field("has_pci_slot", &self.pci_slot.is_some())
            .finish()
    }
}

/// Result returned by the Core-owned probe adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuProbeResult {
    backing: GpuBackingToken,
    platform: GpuPlatformToken,
    physical_present: bool,
    render_node_available: bool,
}

impl GpuProbeResult {
    /// Construct a present GPU observation at the Core boundary.
    pub fn present(
        backing: GpuBackingToken,
        platform: GpuPlatformToken,
        render_node_available: bool,
    ) -> Result<Self, GpuProbeError> {
        if backing.is_zero() || platform.is_zero() {
            return Err(GpuProbeError::StaleIdentity);
        }
        Ok(Self {
            backing,
            platform,
            physical_present: true,
            render_node_available,
        })
    }

    /// Construct a bounded absent observation retaining no physical identity.
    pub const fn absent() -> Self {
        Self {
            backing: GpuBackingToken::from_core([0; 32]),
            platform: GpuPlatformToken::from_core([0; 32]),
            physical_present: false,
            render_node_available: false,
        }
    }

    /// Borrow the opaque backing identity.
    pub const fn backing(&self) -> &GpuBackingToken {
        &self.backing
    }

    /// Borrow the opaque platform identity.
    pub const fn platform(&self) -> &GpuPlatformToken {
        &self.platform
    }

    /// Whether the physical DRM device is present.
    pub const fn physical_present(&self) -> bool {
        self.physical_present
    }

    /// Whether a render node is available.
    pub const fn render_node_available(&self) -> bool {
        self.render_node_available
    }
}

impl fmt::Debug for GpuProbeResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuProbeResult")
            .field("physical_present", &self.physical_present)
            .field("render_node_available", &self.render_node_available)
            .finish()
    }
}

/// Closed probe failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuProbeError {
    /// Selector shape is invalid.
    InvalidSelector,
    /// A platform or backing identity could not be proven.
    StaleIdentity,
    /// The observe interval is outside the signed bounds.
    ObserveIntervalOutOfRange,
    /// The Core probe adapter is unavailable.
    Unavailable,
}

impl GpuProbeError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidSelector => "gpu-selector-invalid",
            Self::StaleIdentity => "gpu-device-identity-stale",
            Self::ObserveIntervalOutOfRange => "gpu-observe-interval-out-of-range",
            Self::Unavailable => "gpu-effect-unavailable",
        }
    }
}

impl fmt::Display for GpuProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for GpuProbeError {}

/// Typed probe port implemented by Core.
pub trait GpuProbePort {
    /// Probe a trusted DRM selector.
    fn probe_drm_device(
        &mut self,
        selector: &GpuDeviceSelector,
    ) -> Result<GpuProbeResult, GpuProbeError>;
}

/// Result of applying three-strike physical probe semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuProbeDisposition {
    /// The device is present and usable.
    Ready,
    /// The first or second failure is not enough to declare absence.
    Unknown,
    /// Three consecutive failures crossed the fail-closed threshold.
    Degraded,
}

/// Bounded three-strike probe tracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuProbeTracker {
    failures: u8,
    interval_secs: u64,
}

impl GpuProbeTracker {
    /// Construct a tracker with the default observe interval.
    pub const fn new() -> Self {
        Self {
            failures: 0,
            interval_secs: DEFAULT_OBSERVE_INTERVAL_SECS,
        }
    }

    /// Construct a tracker with an explicitly bounded observe interval.
    pub fn with_interval(interval_secs: u64) -> Result<Self, GpuProbeError> {
        if !(MIN_OBSERVE_INTERVAL_SECS..=MAX_OBSERVE_INTERVAL_SECS).contains(&interval_secs) {
            return Err(GpuProbeError::ObserveIntervalOutOfRange);
        }
        Ok(Self {
            failures: 0,
            interval_secs,
        })
    }

    /// Return the configured observe interval.
    pub const fn interval_secs(&self) -> u64 {
        self.interval_secs
    }

    /// Return the consecutive failure count.
    pub const fn failures(&self) -> u8 {
        self.failures
    }

    /// Record one successful probe.
    pub fn record_success(&mut self) -> GpuProbeDisposition {
        self.failures = 0;
        GpuProbeDisposition::Ready
    }

    /// Record one failed probe using the three-strike contract.
    pub fn record_failure(&mut self) -> GpuProbeDisposition {
        self.failures = self.failures.saturating_add(1).min(3);
        if self.failures >= 3 {
            GpuProbeDisposition::Degraded
        } else {
            GpuProbeDisposition::Unknown
        }
    }
}

impl Default for GpuProbeTracker {
    fn default() -> Self {
        Self::new()
    }
}
