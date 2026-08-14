//! Minijail launch admission and mandatory platform gate.

use d2b_process_conformance::{LaunchTicket, ProcessConformanceError};
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
};

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
    /// Construct a platform snapshot for hermetic conformance tests.
    pub const fn new_for_test(
        kernel_major: u16,
        kernel_minor: u16,
        cgroup_kill_writable: bool,
    ) -> Self {
        Self {
            kernel_major,
            kernel_minor,
            cgroup_kill_writable,
        }
    }

    /// Probe the kernel and the daemon's delegated cgroup leaf.
    ///
    /// Probe failures become a rejected snapshot rather than an ambient
    /// fallback. The daemon can therefore compose the Provider during
    /// startup and every launch still fails closed when the host posture is
    /// unavailable.
    pub fn detect() -> Self {
        let (kernel_major, kernel_minor) = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .and_then(|release| Self::parse_kernel_release(&release))
            .unwrap_or((0, 0));
        let cgroup_kill_writable = Self::current_cgroup_kill_path()
            .map(|path| Self::writable_file(&path))
            .unwrap_or(false);
        Self {
            kernel_major,
            kernel_minor,
            cgroup_kill_writable,
        }
    }

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

    fn parse_kernel_release(release: &str) -> Option<(u16, u16)> {
        let mut components = release.split('.');
        let major = components.next()?.parse().ok()?;
        let minor = components
            .next()
            .and_then(|component| {
                component
                    .split(|character: char| !character.is_ascii_digit())
                    .next()
            })
            .filter(|component| !component.is_empty())?
            .parse()
            .ok()?;
        Some((major, minor))
    }

    fn current_cgroup_kill_path() -> Option<PathBuf> {
        let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
        let relative = cgroup
            .lines()
            .find_map(|line| line.strip_prefix("0::"))?
            .trim();
        let relative = relative.trim_start_matches('/');
        let path = Path::new("/sys/fs/cgroup")
            .join(relative)
            .join("cgroup.kill");
        path.is_file().then_some(path)
    }

    fn writable_file(path: &Path) -> bool {
        OpenOptions::new().write(true).open(path).is_ok()
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
