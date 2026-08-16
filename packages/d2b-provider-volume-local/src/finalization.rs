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
