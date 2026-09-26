//! Minijail launch admission and mandatory platform gate.

use d2b_process_conformance::ProcessConformanceError;

/// Linux placement requirements that cannot be downgraded by config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformGate {
    /// Kernel major.
    pub kernel_major: u16,
    /// Kernel minor.
    pub kernel_minor: u16,
    /// Whether the runtime cgroup exposes cgroup.kill.
    pub cgroup_kill_available: bool,
}

impl PlatformGate {
    /// Construct a gate from daemon-owned host observations.
    pub const fn from_observed(
        kernel_major: u16,
        kernel_minor: u16,
        cgroup_kill_available: bool,
    ) -> Self {
        Self {
            kernel_major,
            kernel_minor,
            cgroup_kill_available,
        }
    }

    /// Check Linux 5.14 and cgroup.kill.
    ///
    /// # Errors
    ///
    /// Returns `PlatformGateRejected` when the kernel is older than
    /// 5.14 or the runtime cgroup does not expose `cgroup.kill`.
    pub const fn validate(self) -> Result<(), ProcessConformanceError> {
        if self.kernel_major < 5
            || (self.kernel_major == 5 && self.kernel_minor < 14)
            || !self.cgroup_kill_available
        {
            Err(ProcessConformanceError::PlatformGateRejected)
        } else {
            Ok(())
        }
    }
}

/// Validate the mandatory platform gate before spawn dispatch.
///
/// Provider identity is checked by the controller itself
/// (`MinijailProcessProvider::validate`); this admission step only
/// verifies the platform evidence the daemon observed.
///
/// # Errors
///
/// Returns `PlatformGateRejected` when the platform gate fails.
pub fn validate_platform_gate(gate: PlatformGate) -> Result<(), ProcessConformanceError> {
    gate.validate()
}
