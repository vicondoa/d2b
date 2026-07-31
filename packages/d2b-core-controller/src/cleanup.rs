//! Pending-cleanup status and prior-generation retention policy.
//!
//! This module computes state only. Store transactions, bundle copies, watch
//! delivery, finalizer effects, and audit appends remain responsibilities of
//! their eventual production adapters.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::v3::ResourceBundleGenerationId;

use crate::{
    configuration::{ResourceKey, RetainedGenerations},
    resource_store::{ManagedBy, PersistedResourceRecord},
};

/// Boolean state of the Zone `PendingCleanup` condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingCleanupState {
    /// At least one configuration-owned resource awaits atomic removal.
    True,
    /// No configuration-owned resource awaits removal.
    False,
}

impl PendingCleanupState {
    /// Return the stable condition value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::True => "True",
            Self::False => "False",
        }
    }
}

/// Cleanup-derived aggregate Zone phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupZonePhase {
    /// Pending cleanup degrades the Zone while normal reconciliation continues.
    Degraded,
    /// Cleanup contributes no degradation.
    Ready,
}

impl CleanupZonePhase {
    /// Return the stable phase spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Degraded => "Degraded",
            Self::Ready => "Ready",
        }
    }
}

/// Bounded status projection for pending configuration cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingCleanupCondition {
    state: PendingCleanupState,
    phase: CleanupZonePhase,
    pending_count: usize,
}

impl PendingCleanupCondition {
    /// Return the fixed condition name.
    pub const fn name(&self) -> &'static str {
        "PendingCleanup"
    }

    /// Return the condition state.
    pub const fn state(&self) -> PendingCleanupState {
        self.state
    }

    /// Return the cleanup-derived aggregate phase.
    pub const fn phase(&self) -> CleanupZonePhase {
        self.phase
    }

    /// Return the number of pending rows without exposing their identities.
    pub const fn pending_count(&self) -> usize {
        self.pending_count
    }
}

/// Project Zone cleanup status from persisted resource metadata.
pub fn pending_cleanup_condition(resources: &[PersistedResourceRecord]) -> PendingCleanupCondition {
    let pending_count = resources
        .iter()
        .filter(|resource| {
            resource.metadata().managed_by() == ManagedBy::Configuration
                && resource.metadata().deletion_requested_at().is_some()
        })
        .count();
    if pending_count == 0 {
        PendingCleanupCondition {
            state: PendingCleanupState::False,
            phase: CleanupZonePhase::Ready,
            pending_count,
        }
    } else {
        PendingCleanupCondition {
            state: PendingCleanupState::True,
            phase: CleanupZonePhase::Degraded,
            pending_count,
        }
    }
}

/// One retained prior bundle and the configuration-owned set it introduced.
#[derive(Clone, PartialEq, Eq)]
pub struct PriorGenerationBundle {
    content_hash: ResourceBundleGenerationId,
    resources: BTreeSet<ResourceKey>,
}

impl PriorGenerationBundle {
    /// Record one prior bundle, with duplicate identities collapsed.
    pub fn new(
        content_hash: ResourceBundleGenerationId,
        resources: impl IntoIterator<Item = ResourceKey>,
    ) -> Self {
        Self {
            content_hash,
            resources: resources.into_iter().collect(),
        }
    }

    /// Borrow the content-addressed bundle identity selected for pruning.
    pub const fn content_hash(&self) -> &ResourceBundleGenerationId {
        &self.content_hash
    }

    /// Return the number of configuration-owned resources without identities.
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }
}

impl core::fmt::Debug for PriorGenerationBundle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PriorGenerationBundle")
            .field("resources", &self.resources.len())
            .finish_non_exhaustive()
    }
}

/// Select prior bundles that may be pruned under count-based retention.
///
/// A source resource is resolved only when its row is absent after atomic
/// deletion or when the generation transition proved it unchanged in a newer
/// generation. No time or TTL input participates in this decision.
pub fn prunable_prior_bundles<'a>(
    prior: &'a [PriorGenerationBundle],
    retained: RetainedGenerations,
    current: &[PersistedResourceRecord],
    unchanged_in_newer_generation: &BTreeSet<ResourceKey>,
) -> Vec<&'a PriorGenerationBundle> {
    let mut remaining = prior.len();
    let cap = usize::from(retained.get());
    if remaining <= cap {
        return Vec::new();
    }

    let current_by_key: BTreeMap<_, _> = current
        .iter()
        .map(|resource| (resource.key(), resource))
        .collect();
    let mut prunable = Vec::new();
    for generation in prior {
        if remaining <= cap {
            break;
        }
        let resolved = generation.resources.iter().all(|key| {
            !current_by_key.contains_key(key) || unchanged_in_newer_generation.contains(key)
        });
        if resolved {
            prunable.push(generation);
            remaining -= 1;
        }
    }
    prunable
}
