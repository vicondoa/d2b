//! Canonical identity for one resource incarnation.

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};

/// Immutable identity for one resource incarnation.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceKey {
    zone: ZoneId,
    resource_ref: ResourceRef,
    uid: ResourceUid,
}

impl ResourceKey {
    /// Construct a Zone-local resource key.
    pub fn new(zone: ZoneId, resource_ref: ResourceRef, uid: ResourceUid) -> Self {
        Self {
            zone,
            resource_ref,
            uid,
        }
    }

    /// Borrow the Zone identity.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the canonical resource reference.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the immutable resource UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }
}

impl core::fmt::Debug for ResourceKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceKey")
            .field("resource_type", self.resource_ref.resource_type())
            .field("has_zone", &true)
            .field("has_uid", &true)
            .finish()
    }
}
