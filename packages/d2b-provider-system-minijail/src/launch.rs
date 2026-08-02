//! Minijail launch admission and mandatory platform gate.

use d2b_process_conformance::{LaunchTicket, ProcessConformanceError};

use crate::PROVIDER_NAME;

/// Linux placement requirements that cannot be downgraded by config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformGate {
    /// Kernel major.
    pub kernel_major: u16,
    /// Kernel minor.
    pub kernel_minor: u16,
    /// Whether the delegated leaf has a writable cgroup.kill.
    pub cgroup_kill_writable: bool,
}

impl PlatformGate {
    /// Check Linux 5.14 and cgroup.kill.
    pub const fn validate(self) -> Result<(), ProcessConformanceError> {
        if self.kernel_major < 5
            || (self.kernel_major == 5 && self.kernel_minor < 14)
            || !self.cgroup_kill_writable
        {
            Err(ProcessConformanceError::PlatformGateRejected)
        } else {
            Ok(())
        }
    }
}

/// Validate provider identity and platform evidence before spawn dispatch.
pub fn validate_launch_ticket(
    ticket: &LaunchTicket,
    gate: PlatformGate,
) -> Result<(), ProcessConformanceError> {
    if ticket.selected_provider().as_str() != PROVIDER_NAME
        || ticket.provider_ref().to_canonical_string() != "Provider/system-minijail"
    {
        return Err(ProcessConformanceError::ProviderMismatch);
    }
    gate.validate()
}
