//! Restart-safe QEMU process identity evidence.

use sha2::{Digest, Sha256};

/// Verified process identity tuple.
#[derive(Clone, PartialEq, Eq)]
pub struct ProcessIdentity {
    /// Process id evidence.
    pub pid: u32,
    /// Kernel start-time evidence.
    pub start_time_ticks: u64,
    /// Owning cgroup digest.
    pub cgroup_digest: [u8; 32],
    /// Executable digest.
    pub executable_digest: [u8; 32],
    /// Signed template digest.
    pub template_digest: [u8; 32],
    /// Resource generation.
    pub generation: u64,
}

impl ProcessIdentity {
    /// Construct deterministic identity evidence for hermetic tests.
    pub fn for_test(value: &str) -> Self {
        let digest = Sha256::digest(value.as_bytes());
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(&digest);
        Self {
            pid: 42,
            start_time_ticks: 7,
            cgroup_digest: bytes,
            executable_digest: bytes,
            template_digest: bytes,
            generation: 1,
        }
    }
}

impl core::fmt::Debug for ProcessIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ProcessIdentity(<redacted>)")
    }
}

/// Result of comparing an observed process identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptionOutcome {
    /// The process matches the durable identity.
    Adopted,
    /// The process cannot be safely reused.
    Quarantined,
}

/// Verify every process identity binding before opening a pidfd.
pub fn verify_identity(expected: &ProcessIdentity, observed: &ProcessIdentity) -> AdoptionOutcome {
    if expected == observed {
        AdoptionOutcome::Adopted
    } else {
        AdoptionOutcome::Quarantined
    }
}
