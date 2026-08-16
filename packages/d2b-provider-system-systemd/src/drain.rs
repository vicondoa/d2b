//! Typed graceful-stop and finalization ordering.

use d2b_process_conformance::WaitReapOwner;

/// Drain stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainStage {
    /// Exact-main stop has been requested.
    GracefulStopRequested,
    /// Systemd supplied the terminal transition.
    ManagerTerminal,
    /// The anchored leaf is empty.
    LeafEmpty,
    /// Finalizer may be cleared.
    Complete,
}

/// Bounded drain proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrainProof {
    /// Exact-main signal/stop was issued.
    pub exact_main_stopped: bool,
    /// The manager supplied a terminal transition.
    pub manager_terminal: bool,
    /// The anchored cgroup leaf is empty.
    pub cgroup_empty: bool,
}

/// Validate the systemd drain sequence.
pub fn validate(proof: DrainProof) -> Result<DrainStage, DrainError> {
    if !proof.exact_main_stopped || !proof.manager_terminal {
        return Err(DrainError::TerminalTransitionMissing);
    }
    if !proof.cgroup_empty {
        return Err(DrainError::LeafNotEmpty);
    }
    Ok(DrainStage::Complete)
}

/// Stable drain refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainError {
    /// Exact stop or manager terminal state was absent.
    TerminalTransitionMissing,
    /// Cgroup leaf still contains a process.
    LeafNotEmpty,
}

impl core::fmt::Display for DrainError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::TerminalTransitionMissing => "systemd-terminal-transition-missing",
            Self::LeafNotEmpty => "systemd-cgroup-leaf-not-empty",
        })
    }
}

impl std::error::Error for DrainError {}

/// The system manager, not the Provider, owns wait and reap.
pub const WAIT_REAP_OWNER: WaitReapOwner = WaitReapOwner::ServiceManager;
