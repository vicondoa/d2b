//! Native rollback boundary and source-preservation contract.

use std::fmt;

use crate::{
    inventory::HostInventory,
    model::{ArtifactId, CutoverPhase, FailureCode},
};

/// The last phase covered by native rollback.
pub const NATIVE_ROLLBACK_BOUNDARY: CutoverPhase = CutoverPhase::Disposition;

/// Result of a pure native rollback decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackResult {
    phase: CutoverPhase,
    sources_preserved: bool,
    quarantined_destinations: Vec<ArtifactId>,
}

impl RollbackResult {
    /// Return the phase at which rollback was requested.
    pub const fn phase(&self) -> CutoverPhase {
        self.phase
    }

    /// Return whether every identity-bearing source remained intact.
    pub const fn sources_preserved(&self) -> bool {
        self.sources_preserved
    }

    /// Borrow staged destinations that must be quarantined.
    pub fn quarantined_destinations(&self) -> &[ArtifactId] {
        &self.quarantined_destinations
    }
}

/// Check the rollback boundary and derive source-preservation obligations.
pub fn plan_native_rollback(
    phase: CutoverPhase,
    inventory: &HostInventory,
    staged_destinations: impl IntoIterator<Item = ArtifactId>,
) -> Result<RollbackResult, RollbackError> {
    if !phase.is_before_or_at_native_rollback_boundary() {
        return Err(RollbackError::BoundaryClosed(phase));
    }
    if !inventory.sources_retained() {
        return Err(RollbackError::SourcesNotPreserved);
    }
    let mut quarantined_destinations = staged_destinations.into_iter().collect::<Vec<_>>();
    quarantined_destinations.sort();
    quarantined_destinations.dedup();
    Ok(RollbackResult {
        phase,
        sources_preserved: true,
        quarantined_destinations,
    })
}

/// Rollback failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollbackError {
    /// The operation crossed the native rollback boundary.
    BoundaryClosed(CutoverPhase),
    /// An identity-bearing source was not retained.
    SourcesNotPreserved,
}

impl RollbackError {
    /// Return the stable failure class.
    pub const fn code(&self) -> FailureCode {
        match self {
            Self::BoundaryClosed(_) => FailureCode::RollbackWindowClosed,
            Self::SourcesNotPreserved => FailureCode::SourceNotPreserved,
        }
    }
}

impl fmt::Display for RollbackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BoundaryClosed(_) => "native rollback boundary is closed",
            Self::SourcesNotPreserved => "rollback source-preservation invariant failed",
        })
    }
}

impl std::error::Error for RollbackError {}
