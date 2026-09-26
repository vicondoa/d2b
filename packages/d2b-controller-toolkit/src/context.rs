//! Manager-served resource snapshots consumed by controller policy.
//!
//! A snapshot is the target body a handler observes: the managed identifier,
//! the allocated revision and generation, and the canonical resource bytes.
//! The reconcile-pass context machinery (reconcile contexts, effect permits,
//! committed-revision proofs, cancellation, and operation identifiers) was
//! deleted with the store-driven reconcile loop it existed for.

use d2b_contracts_resource::v3::{ResourceGeneration, ResourceUid, ZoneRevision};

use crate::ResourceKey;

/// The store's immutable singular-owner identity attached to a snapshot.
///
/// The owner UID and generation are bound together: the store writes them as
/// one unit, so a snapshot never carries one without the other.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnerIdentity {
    uid: ResourceUid,
    generation: ResourceGeneration,
}

impl OwnerIdentity {
    /// Construct an owner identity.
    pub const fn new(uid: ResourceUid, generation: ResourceGeneration) -> Self {
        Self { uid, generation }
    }

    /// Borrow the immutable owner UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Return the immutable owner generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }
}

/// Manager-served target body observed by one controller pass.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceSnapshot {
    key: ResourceKey,
    owner: Option<OwnerIdentity>,
    revision: ZoneRevision,
    generation: ResourceGeneration,
    canonical_json: Vec<u8>,
    deleting: bool,
}

impl ResourceSnapshot {
    /// Construct a fresh resource snapshot.
    pub fn new(
        key: ResourceKey,
        revision: ZoneRevision,
        generation: ResourceGeneration,
        canonical_json: Vec<u8>,
        deleting: bool,
    ) -> Self {
        Self {
            key,
            owner: None,
            revision,
            generation,
            canonical_json,
            deleting,
        }
    }

    /// Borrow the immutable identity.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Borrow the immutable singular owner identity when the store supplied it.
    pub const fn owner(&self) -> Option<&OwnerIdentity> {
        self.owner.as_ref()
    }

    /// Attach the store's immutable owner identity to this snapshot.
    pub fn with_owner_identity(mut self, owner: Option<OwnerIdentity>) -> Self {
        self.owner = owner;
        self
    }

    /// Return the fresh revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Return the desired-state generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// Borrow canonical resource bytes.
    pub fn canonical_json(&self) -> &[u8] {
        &self.canonical_json
    }

    /// Whether deletion has been requested.
    pub const fn deleting(&self) -> bool {
        self.deleting
    }
}

impl core::fmt::Debug for ResourceSnapshot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ResourceSnapshot")
            .field("key", &self.key)
            .field("revision", &self.revision)
            .field("generation", &self.generation)
            .field(
                "canonical_json",
                &format_args!("<{} bytes>", self.canonical_json.len()),
            )
            .field("deleting", &self.deleting)
            .finish()
    }
}

/// Base-only dependency snapshot from the same Zone as the target.
#[derive(Clone, PartialEq, Eq)]
pub struct DependencySnapshot {
    resource: ResourceSnapshot,
}

impl DependencySnapshot {
    /// Wrap a base-only dependency resource.
    pub fn new(resource: ResourceSnapshot) -> Self {
        Self { resource }
    }

    /// Borrow the dependency resource.
    pub const fn resource(&self) -> &ResourceSnapshot {
        &self.resource
    }
}

impl core::fmt::Debug for DependencySnapshot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DependencySnapshot")
            .field("resource", &self.resource)
            .finish()
    }
}

