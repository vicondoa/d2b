//! Dependency-safe Volume finalization policy.

use crate::identity::EntryDigest;

/// Facts the core owner supplies before a Volume finalizer may release state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizationObservation {
    dependents_remaining: u32,
    store_view_writer_released: bool,
}

impl FinalizationObservation {
    /// Construct a bounded finalization observation.
    pub const fn new(dependents_remaining: u32, store_view_writer_released: bool) -> Self {
        Self {
            dependents_remaining,
            store_view_writer_released,
        }
    }

    /// Number of dependent resources that still hold the Volume.
    pub const fn dependents_remaining(self) -> u32 {
        self.dependents_remaining
    }

    /// Whether the store-view writer lease has been closed.
    pub const fn store_view_writer_released(self) -> bool {
        self.store_view_writer_released
    }
}

/// Finalization action selected from dependency and writer evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizationAction {
    /// Dependents must finalize first.
    WaitForDependents,
    /// The store-view writer still owns the Volume.
    WaitForStoreWriter,
    /// All owned effects may be cleaned up in leaf-first order.
    Cleanup,
}

/// Result of a dependency-safe finalization attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizationResult {
    /// Finalization must be retried after the returned dependency condition.
    Waiting(FinalizationAction),
    /// Leaf-first cleanup completed.
    Cleaned(Vec<EntryDigest>),
}

/// Select the only safe finalization step.
pub const fn finalization_plan(observation: FinalizationObservation) -> FinalizationAction {
    if observation.dependents_remaining > 0 {
        FinalizationAction::WaitForDependents
    } else if !observation.store_view_writer_released {
        FinalizationAction::WaitForStoreWriter
    } else {
        FinalizationAction::Cleanup
    }
}

// ---------------------------------------------------------------------------
// Release and deletion are two decisions (AE17).
//
// Releasing the last consumer is a fact about a relationship going away; it
// says nothing about whether the source it pointed at should disappear.  A
// shared source outlives its consumers by definition, so a release never
// implies a deletion and the two are selected from separate facts here.
// ---------------------------------------------------------------------------

/// The facts one source release decision is made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceReleaseObservation {
    live_consumers: u32,
    deletion_requested: bool,
    externally_owned: bool,
}

impl SourceReleaseObservation {
    /// Construct a bounded release observation.
    ///
    /// `externally_owned` marks a source this Provider does not own outright,
    /// such as a closure store or a backing store another resource created.
    /// Deleting one is never this Provider's decision, whatever else is true.
    pub const fn new(
        live_consumers: u32,
        deletion_requested: bool,
        externally_owned: bool,
    ) -> Self {
        Self {
            live_consumers,
            deletion_requested,
            externally_owned,
        }
    }

    /// Number of admitted relationships still holding this source.
    pub const fn live_consumers(self) -> u32 {
        self.live_consumers
    }

    /// Whether deletion was requested for this exact source.
    pub const fn deletion_requested(self) -> bool {
        self.deletion_requested
    }

    /// Whether this source is owned outside this Provider.
    pub const fn externally_owned(self) -> bool {
        self.externally_owned
    }
}

/// What releasing the last consumer decided about the shared source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceReleaseDecision {
    /// At least one relationship still holds the source; nothing is released.
    ConsumersRemain {
        /// How many relationships still hold it.
        live_consumers: u32,
    },
    /// The last consumer released and the shared source is retained.  This is
    /// the ordinary outcome of a release: the relationship is gone, the
    /// Volume is not.
    RetainSharedSource,
    /// The last consumer released AND deletion was requested for this source,
    /// which this Provider owns, so its owned effects may be cleaned up.
    DeleteSource,
}

/// Select the one safe release decision.
///
/// Releasing the last consumer alone is never enough to delete a shared
/// source: a retained source is what makes the next consumer's admission
/// possible without recreating state.  Only an explicit deletion request for
/// a source this Provider owns reaches the cleanup decision.
pub const fn decide_source_release(observation: SourceReleaseObservation) -> SourceReleaseDecision {
    if observation.live_consumers > 0 {
        return SourceReleaseDecision::ConsumersRemain {
            live_consumers: observation.live_consumers,
        };
    }
    if observation.deletion_requested && !observation.externally_owned {
        SourceReleaseDecision::DeleteSource
    } else {
        SourceReleaseDecision::RetainSharedSource
    }
}
