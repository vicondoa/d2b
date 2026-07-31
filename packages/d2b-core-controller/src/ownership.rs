//! Generic ownership and finalizer deletion-order policy.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::v3::{FinalizerId, ResourceRef};
use d2b_controller_toolkit::ResourceKey;

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
}
