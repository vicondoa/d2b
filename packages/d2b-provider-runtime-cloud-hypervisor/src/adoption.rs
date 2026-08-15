//! Restart-safe process adoption.

use std::fmt;

/// Verified process identity bindings.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    /// Process id observed before pidfd open.
    pub pid: u32,
    /// Kernel start-time ticks.
    pub start_time_ticks: u64,
    /// Digest of the owning cgroup.
    pub cgroup_digest: [u8; 32],
    /// Digest of the executable inode.
    pub executable_digest: [u8; 32],
    /// Digest of the signed process template.
    pub template_digest: [u8; 32],
    /// Resource generation.
    pub generation: u64,
}

impl fmt::Debug for ProcessIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProcessIdentity(<redacted>)")
    }
}

/// Result of a process adoption scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptionOutcome {
    /// No process exists.
    Absent,
    /// Exact identity was verified and may receive a pidfd.
    Adopted,
    /// Identity was ambiguous or stale.
    Quarantined,
}

/// Verify the complete identity tuple before pidfd acquisition.
pub fn verify_identity(expected: &ProcessIdentity, observed: &ProcessIdentity) -> AdoptionOutcome {
    if expected == observed {
        AdoptionOutcome::Adopted
    } else {
        AdoptionOutcome::Quarantined
    }
}
