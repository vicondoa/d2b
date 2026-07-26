//! Bounded child-change hints emitted after ownership-index commits.

use d2b_contracts::v3::{ResourceRef, ResourceUid, ZoneRevision};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Maximum validated owner-chain depth followed by hint propagation.
pub const MAX_OWNER_HINT_DEPTH: usize = 8;
/// Maximum resources visited by one owner-hint propagation pass.
pub const MAX_OWNER_HINT_WORK_ITEMS: usize = 64;

/// Child mutation class carried by an owner hint.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum OwnerChangeEvent {
    Created,
    SpecUpdated,
    StatusUpdated,
    MetadataUpdated,
    FinalizersUpdated,
    DeletionRequested,
    Deleted,
    Reparented,
}

/// One `owned-resource-changed` notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnedResourceChangedHint {
    owner_ref: ResourceRef,
    owner_uid: ResourceUid,
    child_ref: ResourceRef,
    child_uid: ResourceUid,
    revision: ZoneRevision,
    event: OwnerChangeEvent,
}

impl OwnedResourceChangedHint {
    /// Construct a committed child-change hint.
    pub fn new(
        owner_ref: ResourceRef,
        owner_uid: ResourceUid,
        child_ref: ResourceRef,
        child_uid: ResourceUid,
        revision: ZoneRevision,
        event: OwnerChangeEvent,
    ) -> Result<Self, OwnerHintCoalesceError> {
        if owner_uid == child_uid || owner_ref == child_ref {
            return Err(OwnerHintCoalesceError::SelfOwnership);
        }
        if revision.get() == 0 {
            return Err(OwnerHintCoalesceError::UncommittedRevision);
        }
        Ok(Self {
            owner_ref,
            owner_uid,
            child_ref,
            child_uid,
            revision,
            event,
        })
    }

    /// Borrow the singular owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the owner UID binding.
    pub const fn owner_uid(&self) -> &ResourceUid {
        &self.owner_uid
    }

    /// Borrow the changed child reference.
    pub const fn child_ref(&self) -> &ResourceRef {
        &self.child_ref
    }

    /// Borrow the immutable child UID.
    pub const fn child_uid(&self) -> &ResourceUid {
        &self.child_uid
    }

    /// Return the committed revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Return the latest child event.
    pub const fn event(&self) -> OwnerChangeEvent {
        self.event
    }

    /// Coalesce a newer change for the same owner-child binding.
    ///
    /// Queue ownership decides when coalescing is allowed; this method only
    /// enforces that two already-eligible hints name the same immutable UIDs.
    pub fn coalesce(&mut self, newer: Self) -> Result<OwnerHintDispatch, OwnerHintCoalesceError> {
        if self.owner_uid != newer.owner_uid || self.child_uid != newer.child_uid {
            return Err(OwnerHintCoalesceError::DifferentBinding);
        }
        if self.owner_ref != newer.owner_ref || self.child_ref != newer.child_ref {
            return Err(OwnerHintCoalesceError::UidReferenceMismatch);
        }
        if newer.revision <= self.revision {
            return Ok(OwnerHintDispatch::AlreadyCovered);
        }
        self.revision = newer.revision;
        self.event = newer.event;
        Ok(OwnerHintDispatch::Replaced)
    }
}

/// Result of bounded hint coalescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerHintDispatch {
    Replaced,
    AlreadyCovered,
}

/// Invalid owner hint or coalescing request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerHintCoalesceError {
    SelfOwnership,
    UncommittedRevision,
    DifferentBinding,
    UidReferenceMismatch,
}

impl core::fmt::Display for OwnerHintCoalesceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SelfOwnership => f.write_str("owner and child must be distinct"),
            Self::UncommittedRevision => f.write_str("owner hints require a committed revision"),
            Self::DifferentBinding => {
                f.write_str("only hints for the same owner-child UID binding can coalesce")
            }
            Self::UidReferenceMismatch => {
                f.write_str("owner hint references do not match their immutable UID binding")
            }
        }
    }
}

impl std::error::Error for OwnerHintCoalesceError {}

/// Post-commit dispatcher seam implemented by the controller runtime.
pub trait OwnerHintDispatcher: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Submit a committed hint. Implementations must enforce bounded queueing.
    fn dispatch(&self, hint: OwnedResourceChangedHint) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(revision: u64, event: OwnerChangeEvent) -> OwnedResourceChangedHint {
        OwnedResourceChangedHint::new(
            ResourceRef::parse("Guest/work").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceRef::parse("Process/app").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap(),
            ZoneRevision::new(revision),
            event,
        )
        .unwrap()
    }

    #[test]
    fn literal_hint_json_pins_names_and_uid_binding() {
        const JSON: &str = concat!(
            "{\"ownerRef\":\"Guest/work\",",
            "\"ownerUid\":\"123e4567-e89b-42d3-a456-426614174000\",",
            "\"childRef\":\"Process/app\",",
            "\"childUid\":\"123e4567-e89b-42d3-a456-426614174001\",",
            "\"revision\":4,\"event\":\"status-updated\"}"
        );
        let decoded: OwnedResourceChangedHint = serde_json::from_str(JSON).unwrap();
        assert_eq!(decoded, hint(4, OwnerChangeEvent::StatusUpdated));
        assert_eq!(serde_json::to_string(&decoded).unwrap(), JSON);
    }

    #[test]
    fn coalescing_keeps_latest_revision_for_one_binding() {
        let mut queued = hint(4, OwnerChangeEvent::StatusUpdated);
        assert_eq!(
            queued
                .coalesce(hint(6, OwnerChangeEvent::DeletionRequested))
                .unwrap(),
            OwnerHintDispatch::Replaced
        );
        assert_eq!(queued.revision(), ZoneRevision::new(6));
        assert_eq!(queued.event(), OwnerChangeEvent::DeletionRequested);
        assert_eq!(
            queued
                .coalesce(hint(5, OwnerChangeEvent::MetadataUpdated))
                .unwrap(),
            OwnerHintDispatch::AlreadyCovered
        );
        assert_eq!(queued.revision(), ZoneRevision::new(6));
    }
}
