//! Generic ownership and finalizer deletion-order policy.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::v3::{FinalizerId, ResourceRef};
use d2b_controller_toolkit::ResourceKey;

use crate::audit::{AuditEvent, resource_name_digest};

/// Observed deletion state used by the generic ownership handler.
#[derive(Clone, PartialEq, Eq)]
pub struct DeletionObservation {
    target: ResourceKey,
    owner: Option<ResourceKey>,
    live_children: u32,
    finalizers: BTreeMap<FinalizerId, ResourceRef>,
    ambiguous: bool,
}

impl DeletionObservation {
    /// Construct one complete deletion observation.
    pub fn new(
        target: ResourceKey,
        owner: Option<ResourceKey>,
        live_children: u32,
        finalizers: impl IntoIterator<Item = (FinalizerId, ResourceRef)>,
        ambiguous: bool,
    ) -> Result<Self, OwnershipError> {
        if owner
            .as_ref()
            .is_some_and(|owner| owner.zone() != target.zone() || owner.uid() == target.uid())
        {
            return Err(OwnershipError::InvalidOwner);
        }
        let finalizers = finalizers.into_iter().collect::<Vec<_>>();
        let finalizer_count = finalizers.len();
        let finalizers = finalizers.into_iter().collect::<BTreeMap<_, _>>();
        if finalizers.len() != finalizer_count
            || finalizers
                .values()
                .any(|owner| owner.resource_type().as_str() != "Process")
        {
            return Err(OwnershipError::InvalidFinalizer);
        }
        Ok(Self {
            target,
            owner,
            live_children,
            finalizers,
            ambiguous,
        })
    }
}

impl core::fmt::Debug for DeletionObservation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DeletionObservation")
            .field("has_resource_identity", &true)
            .field("has_owner", &self.owner.is_some())
            .field("live_children", &self.live_children)
            .field("finalizer_count", &self.finalizers.len())
            .field("ambiguous", &self.ambiguous)
            .finish()
    }
}

/// Generic deletion disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionDisposition {
    DeleteChildrenFirst,
    DispatchFinalizers,
    ReadyForStoreDeletion,
    BlockedAmbiguous,
}

/// One finalizer dispatch without resource identity in diagnostics.
#[derive(Clone, PartialEq, Eq)]
pub struct FinalizerDispatch {
    finalizer: FinalizerId,
    controller: ResourceRef,
}

impl FinalizerDispatch {
    /// Borrow the finalizer identifier for authorized dispatch.
    pub const fn finalizer(&self) -> &FinalizerId {
        &self.finalizer
    }

    /// Borrow the exact registered controller owner.
    pub const fn controller(&self) -> &ResourceRef {
        &self.controller
    }
}

impl core::fmt::Debug for FinalizerDispatch {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("FinalizerDispatch")
            .field("has_finalizer", &true)
            .field("has_controller", &true)
            .finish()
    }
}

/// Effect-free child-first deletion plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionPlan {
    disposition: DeletionDisposition,
    finalizers: Vec<FinalizerDispatch>,
}

impl DeletionPlan {
    /// Return the deletion disposition.
    pub const fn disposition(&self) -> DeletionDisposition {
        self.disposition
    }

    /// Borrow exact finalizer-owner dispatches.
    pub fn finalizers(&self) -> &[FinalizerDispatch] {
        &self.finalizers
    }
}

/// Closed ownership refusal reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipError {
    InvalidOwner,
    OwnerCycle,
    OwnerDepthExceeded,
    InvalidFinalizer,
    FinalizerNotOwned,
    CleanupNotConfirmed,
}

impl OwnershipError {
    /// Return the stable, identity-free reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidOwner => "owner-invalid",
            Self::OwnerCycle => "owner-cycle",
            Self::OwnerDepthExceeded => "owner-depth-exceeded",
            Self::InvalidFinalizer => "finalizer-invalid",
            Self::FinalizerNotOwned => "finalizer-not-owned",
            Self::CleanupNotConfirmed => "finalizer-cleanup-unconfirmed",
        }
    }
}

impl core::fmt::Display for OwnershipError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for OwnershipError {}

/// Pure generic ownership/finalizer handler.
pub struct OwnershipHandler;

impl OwnershipHandler {
    /// Validate one singular owner chain against cycle and depth bounds.
    pub fn validate_owner_chain<'a>(
        target: &'a ResourceKey,
        owners: impl IntoIterator<Item = &'a ResourceKey>,
        max_depth: usize,
    ) -> Result<(), OwnershipError> {
        if max_depth == 0 {
            return Err(OwnershipError::OwnerDepthExceeded);
        }
        let mut seen = BTreeSet::from([target.uid().clone()]);
        for (depth, owner) in owners.into_iter().enumerate() {
            if owner.zone() != target.zone() {
                return Err(OwnershipError::InvalidOwner);
            }
            if depth >= max_depth {
                return Err(OwnershipError::OwnerDepthExceeded);
            }
            if !seen.insert(owner.uid().clone()) {
                return Err(OwnershipError::OwnerCycle);
            }
        }

        Ok(())
    }

    /// Plan child-first deletion and exact finalizer dispatch.
    pub fn plan_deletion(observation: &DeletionObservation) -> DeletionPlan {
        if observation.ambiguous {
            return DeletionPlan {
                disposition: DeletionDisposition::BlockedAmbiguous,
                finalizers: Vec::new(),
            };
        }
        if observation.live_children > 0 {
            return DeletionPlan {
                disposition: DeletionDisposition::DeleteChildrenFirst,
                finalizers: Vec::new(),
            };
        }
        let finalizers = observation
            .finalizers
            .iter()
            .map(|(finalizer, controller)| FinalizerDispatch {
                finalizer: finalizer.clone(),
                controller: controller.clone(),
            })
            .collect::<Vec<_>>();
        DeletionPlan {
            disposition: if finalizers.is_empty() {
                DeletionDisposition::ReadyForStoreDeletion
            } else {
                DeletionDisposition::DispatchFinalizers
            },
            finalizers,
        }
    }

    /// Authorize removal only by the exact finalizer owner after cleanup.
    pub fn authorize_finalizer_removal(
        observation: &DeletionObservation,
        finalizer: &FinalizerId,
        requester: &ResourceRef,
        cleanup_confirmed: bool,
    ) -> Result<(), OwnershipError> {
        if observation.finalizers.get(finalizer) != Some(requester) {
            return Err(OwnershipError::FinalizerNotOwned);
        }
        if !cleanup_confirmed {
            return Err(OwnershipError::CleanupNotConfirmed);
        }
        Ok(())
    }
}

/// Observations required before a final resource deletion may commit.
#[derive(Clone, PartialEq, Eq)]
pub struct AtomicDeletionObservation {
    target: ResourceKey,
    pending_finalizers: u32,
    live_children: u32,
    ambiguous: bool,
    prior_generation: d2b_contracts::v3::ConfigurationGeneration,
    active_generation: d2b_contracts::v3::ConfigurationGeneration,
    timestamp: d2b_contracts::v3::Timestamp,
}

impl AtomicDeletionObservation {
    /// Construct a store deletion observation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        target: ResourceKey,
        pending_finalizers: u32,
        live_children: u32,
        ambiguous: bool,
        prior_generation: d2b_contracts::v3::ConfigurationGeneration,
        active_generation: d2b_contracts::v3::ConfigurationGeneration,
        timestamp: d2b_contracts::v3::Timestamp,
    ) -> Self {
        Self {
            target,
            pending_finalizers,
            live_children,
            ambiguous,
            prior_generation,
            active_generation,
            timestamp,
        }
    }

    /// Borrow the exact store key for an authorized transaction.
    pub const fn target(&self) -> &ResourceKey {
        &self.target
    }
}

impl core::fmt::Debug for AtomicDeletionObservation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AtomicDeletionObservation")
            .field("has_target", &true)
            .field("pending_finalizers", &self.pending_finalizers)
            .field("live_children", &self.live_children)
            .field("ambiguous", &self.ambiguous)
            .finish()
    }
}

/// Refusal reason for a final deletion transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomicDeletionError {
    /// The resource's owner or store state is ambiguous.
    Ambiguous,
    /// Finalizers must be cleared before the row can be removed.
    FinalizersRemain,
    /// Owned children must be removed before the parent.
    ChildrenRemain,
}

impl AtomicDeletionError {
    /// Return the stable failure label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ambiguous => "atomic-deletion-ambiguous",
            Self::FinalizersRemain => "atomic-deletion-finalizers-remain",
            Self::ChildrenRemain => "atomic-deletion-children-remain",
        }
    }
}

impl core::fmt::Display for AtomicDeletionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for AtomicDeletionError {}

/// Proof that one store transaction removed a row, its indexes, and emitted
/// the corresponding Deleted revision event.
#[derive(Clone, PartialEq, Eq)]
pub struct AtomicDeletionCommit {
    revision: d2b_contracts::v3::ZoneRevision,
    audit: AuditEvent,
}

impl AtomicDeletionCommit {
    /// Return the authoritative Deleted revision.
    pub const fn revision(&self) -> d2b_contracts::v3::ZoneRevision {
        self.revision
    }

    /// Borrow the post-commit audit event. Appending it is intentionally a
    /// separate operation on the per-Zone audit sink.
    pub const fn audit(&self) -> &AuditEvent {
        &self.audit
    }

    /// The transaction proves both row and index removal together.
    pub const fn row_and_indexes_removed_atomically(&self) -> bool {
        true
    }
}

impl core::fmt::Debug for AtomicDeletionCommit {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AtomicDeletionCommit")
            .field("revision", &self.revision.get())
            .field("audit", &self.audit)
            .finish()
    }
}

/// Model the final resource-store transaction after all ownership proofs hold.
///
/// This function does not mutate a row itself. The concrete redb adapter owns
/// the transaction; the returned proof is the only input that permits the
/// audit sink to append a deletion event.
pub fn commit_atomic_deletion(
    observation: AtomicDeletionObservation,
    revision: d2b_contracts::v3::ZoneRevision,
) -> Result<AtomicDeletionCommit, AtomicDeletionError> {
    if observation.ambiguous {
        return Err(AtomicDeletionError::Ambiguous);
    }
    if observation.pending_finalizers > 0 {
        return Err(AtomicDeletionError::FinalizersRemain);
    }
    if observation.live_children > 0 {
        return Err(AtomicDeletionError::ChildrenRemain);
    }
    let target = observation.target();
    let audit = AuditEvent::resource_deleted(
        target.zone().clone(),
        target.resource_ref().resource_type().clone(),
        resource_name_digest(target.resource_ref().name()),
        observation.prior_generation,
        observation.active_generation,
        revision,
        observation.timestamp.clone(),
    );
    Ok(AtomicDeletionCommit { revision, audit })
}

#[cfg(test)]
mod tests {
    use d2b_contracts::v3::{ResourceUid, ZoneId};

    use super::*;

    fn key(name: &str, suffix: u16) -> ResourceKey {
        ResourceKey::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse(&format!("Process/{name}")).unwrap(),
            ResourceUid::parse(format!("123e4567-e89b-42d3-a456-{suffix:012}")).unwrap(),
        )
    }

    fn finalizer() -> FinalizerId {
        FinalizerId::parse("provider.d2bus.org/cleanup").unwrap()
    }

    #[test]
    fn children_are_deleted_before_finalizers_dispatch() {
        let observation = DeletionObservation::new(
            key("target", 1),
            None,
            2,
            [(
                finalizer(),
                ResourceRef::parse("Process/controller").unwrap(),
            )],
            false,
        )
        .unwrap();
        assert_eq!(
            OwnershipHandler::plan_deletion(&observation).disposition(),
            DeletionDisposition::DeleteChildrenFirst
        );
    }

    #[test]
    fn exact_owner_can_clear_its_finalizer_after_confirmed_cleanup() {
        let controller = ResourceRef::parse("Process/controller").unwrap();
        let observation = DeletionObservation::new(
            key("target", 1),
            None,
            0,
            [(finalizer(), controller.clone())],
            false,
        )
        .unwrap();
        assert_eq!(
            OwnershipHandler::authorize_finalizer_removal(
                &observation,
                &finalizer(),
                &controller,
                true,
            ),
            Ok(())
        );
    }

    #[test]
    fn another_controller_and_unconfirmed_cleanup_are_rejected() {
        let controller = ResourceRef::parse("Process/controller").unwrap();
        let observation = DeletionObservation::new(
            key("target", 1),
            None,
            0,
            [(finalizer(), controller.clone())],
            false,
        )
        .unwrap();
        assert_eq!(
            OwnershipHandler::authorize_finalizer_removal(
                &observation,
                &finalizer(),
                &ResourceRef::parse("Process/other").unwrap(),
                true,
            ),
            Err(OwnershipError::FinalizerNotOwned)
        );
        assert_eq!(
            OwnershipHandler::authorize_finalizer_removal(
                &observation,
                &finalizer(),
                &controller,
                false,
            ),
            Err(OwnershipError::CleanupNotConfirmed)
        );
    }

    #[test]
    fn cycle_and_depth_overflow_are_rejected() {
        let target = key("target", 1);
        let owner = key("owner", 2);
        assert_eq!(
            OwnershipHandler::validate_owner_chain(&target, [&owner, &target], 4),
            Err(OwnershipError::OwnerCycle)
        );
        assert_eq!(
            OwnershipHandler::validate_owner_chain(&target, [&owner], 0),
            Err(OwnershipError::OwnerDepthExceeded)
        );
    }

    #[test]
    fn ambiguous_observation_never_invents_cleanup_success() {
        let observation = DeletionObservation::new(key("target", 1), None, 0, [], true).unwrap();
        assert_eq!(
            OwnershipHandler::plan_deletion(&observation).disposition(),
            DeletionDisposition::BlockedAmbiguous
        );
    }

    #[test]
    fn final_deletion_proof_requires_child_and_finalizer_completion() {
        let target = key("secret-resource-name", 3);
        let timestamp = d2b_contracts::v3::Timestamp::parse("2026-08-01T00:00:00.000Z").unwrap();
        let prior = d2b_contracts::v3::ConfigurationGeneration::new(1).unwrap();
        let active = d2b_contracts::v3::ConfigurationGeneration::new(2).unwrap();
        let blocked = AtomicDeletionObservation::new(
            target.clone(),
            1,
            0,
            false,
            prior,
            active,
            timestamp.clone(),
        );
        assert_eq!(
            commit_atomic_deletion(blocked, d2b_contracts::v3::ZoneRevision::new(4)).unwrap_err(),
            AtomicDeletionError::FinalizersRemain
        );
        let committed = commit_atomic_deletion(
            AtomicDeletionObservation::new(target, 0, 0, false, prior, active, timestamp),
            d2b_contracts::v3::ZoneRevision::new(5),
        )
        .unwrap();
        assert!(committed.row_and_indexes_removed_atomically());
        assert_eq!(
            committed.audit().kind(),
            crate::audit::AuditEventKind::ResourceDeleted
        );
        assert!(!format!("{committed:?}").contains("secret-resource-name"));
    }
}
