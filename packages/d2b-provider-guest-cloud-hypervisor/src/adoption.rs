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

/// Bounded observation returned by the Process Provider during recovery.
#[derive(Clone, PartialEq, Eq)]
pub enum ProcessObservation {
    /// No matching process was observed.
    Absent,
    /// One or more candidates were observed before pidfd acquisition.
    Candidates(Vec<ProcessIdentity>),
    /// The Process Provider could not complete a bounded observation.
    Unavailable,
}

impl ProcessObservation {
    /// Construct a bounded candidate observation.
    pub fn candidates(
        candidates: impl IntoIterator<Item = ProcessIdentity>,
    ) -> Result<Self, AdoptionObservationError> {
        let candidates = candidates.into_iter().collect::<Vec<_>>();
        if candidates.is_empty() || candidates.len() > 8 {
            return Err(AdoptionObservationError::CandidateCount);
        }
        Ok(Self::Candidates(candidates))
    }

    /// Borrow the observed candidates, when the observation is complete.
    pub fn candidates_ref(&self) -> Option<&[ProcessIdentity]> {
        match self {
            Self::Candidates(candidates) => Some(candidates),
            Self::Absent | Self::Unavailable => None,
        }
    }
}

impl fmt::Debug for ProcessObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => formatter.write_str("ProcessObservation::Absent"),
            Self::Unavailable => formatter.write_str("ProcessObservation::Unavailable"),
            Self::Candidates(candidates) => formatter
                .debug_struct("ProcessObservation::Candidates")
                .field("candidate_count", &candidates.len())
                .finish(),
        }
    }
}

/// Failure while constructing a bounded adoption observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoptionObservationError {
    /// The observation contained no candidates or exceeded its bound.
    CandidateCount,
}

impl fmt::Display for AdoptionObservationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("cloud-hypervisor-adoption-candidate-count-invalid")
    }
}

impl std::error::Error for AdoptionObservationError {}

/// Bounded adoption result exposed to the Guest controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessAdoptionStatus {
    /// The Process Provider has already established current status.
    Current,
    /// The Process Provider verified and adopted the exact process locally.
    Adopted,
    /// No process realization remains.
    Absent,
    /// Identity was stale or ambiguous and is quarantined.
    Quarantined,
    /// The Process Provider could not complete a safe observation.
    Unavailable,
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

/// Select a process only when the complete identity tuple is exact and unique.
///
/// A missing expected identity, an unavailable observation, a stale candidate,
/// and every ambiguous candidate set are quarantined. No caller may turn this
/// result into a signal or name-only stop.
pub fn adopt_exact(
    expected: Option<&ProcessIdentity>,
    observation: &ProcessObservation,
) -> AdoptionOutcome {
    match observation {
        ProcessObservation::Absent => AdoptionOutcome::Absent,
        ProcessObservation::Unavailable => AdoptionOutcome::Quarantined,
        ProcessObservation::Candidates(candidates) => {
            if candidates.len() == 1 && expected == Some(&candidates[0]) {
                AdoptionOutcome::Adopted
            } else {
                AdoptionOutcome::Quarantined
            }
        }
    }
}

/// Verify the complete identity tuple before pidfd acquisition.
pub fn verify_identity(expected: &ProcessIdentity, observed: &ProcessIdentity) -> AdoptionOutcome {
    if expected == observed {
        AdoptionOutcome::Adopted
    } else {
        AdoptionOutcome::Quarantined
    }
}
