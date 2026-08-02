//! Redacted Zone configuration audit records.
//!
//! The controller owns the decision to emit a configuration event, while the
//! store owns the revision that proves a deletion committed.  This module
//! keeps those two facts separate: an event can be appended only after the
//! caller supplies the committed revision, and the recovery key makes replay
//! idempotent without retaining a resource name.

use std::collections::BTreeSet;

use d2b_contracts::v3::{
    ConfigurationGeneration, ResourceBundleGenerationId, ResourceName, ResourceTypeName,
    SchemaFingerprint, Timestamp, ZoneId, ZoneRevision, canonical_digest,
};

/// Domain tag for the digest of a resource name in an audit record.
pub const RESOURCE_NAME_AUDIT_DOMAIN: &str = "d2b:v3:audit-resource-name";

/// Compute the stable, non-reversible resource-name digest used by cleanup
/// audit records.
pub fn resource_name_digest(name: &ResourceName) -> SchemaFingerprint {
    SchemaFingerprint::parse(canonical_digest(
        RESOURCE_NAME_AUDIT_DOMAIN,
        name.as_str().as_bytes(),
    ))
    .expect("canonical digest is a valid schema fingerprint")
}

/// Closed configuration audit event kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuditEventKind {
    /// A bundle passed integrity checks and became active.
    GenerationActivated,
    /// A bundle was rejected before resource mutation.
    GenerationRejected,
    /// A configuration-owned resource received a Delete request.
    ResourceDeletionRequested,
    /// A resource deletion committed in the store.
    ResourceDeleted,
    /// Cleanup remained blocked beyond its configured threshold.
    CleanupStalled,
    /// A retained generation was activated as a rollback.
    GenerationRolledBack,
    /// A bundle item collided with a foreign owner.
    ConfigurationCollision,
}

impl AuditEventKind {
    /// Return the stable event token.
    pub const fn label(self) -> &'static str {
        match self {
            Self::GenerationActivated => "zone.config.generation.activate",
            Self::GenerationRejected => "zone.config.activate.error",
            Self::ResourceDeletionRequested => "config-resource-deletion-requested",
            Self::ResourceDeleted => "zone.config.cleanup.complete",
            Self::CleanupStalled => "zone.config.cleanup.stuck",
            Self::GenerationRolledBack => "zone.config.generation.rollback",
            Self::ConfigurationCollision => "config-collision",
        }
    }
}

/// Closed reason values used by configuration audit records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuditReason {
    /// The resource was absent from the successor bundle.
    AbsentFromNewGeneration,
    /// The bundle's content digest did not verify.
    BundleIntegrityFailed,
    /// The artifact catalog anchor did not verify.
    CatalogMismatch,
    /// A ResourceType schema did not verify.
    SchemaMismatch,
    /// An installed Provider schema did not verify.
    ProviderSchemaMismatch,
    /// The resource was owned by a controller or API.
    ForeignOwner,
    /// A Provider finalizer did not clear in time.
    FinalizerBlocked,
    /// A controller-owned child prevented parent deletion.
    OwnerChildBlocked,
    /// A caller supplied an invalid bundle.
    SchemaValidationFailed,
}

impl AuditReason {
    /// Return the stable reason token.
    pub const fn label(self) -> &'static str {
        match self {
            Self::AbsentFromNewGeneration => "absent-from-new-generation",
            Self::BundleIntegrityFailed => "config-bundle-integrity-failed",
            Self::CatalogMismatch => "config-catalog-mismatch",
            Self::SchemaMismatch => "config-schema-mismatch",
            Self::ProviderSchemaMismatch => "provider-schema-mismatch",
            Self::ForeignOwner => "cleanup-config-ownership-mismatch",
            Self::FinalizerBlocked => "finalizer-blocked",
            Self::OwnerChildBlocked => "owner-child-blocked",
            Self::SchemaValidationFailed => "schema-validation-failed",
        }
    }
}

/// The exactly-once recovery key for a committed audit append.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuditRecoveryKey {
    kind: AuditEventKind,
    revision: Option<ZoneRevision>,
    resource_name_digest: Option<SchemaFingerprint>,
    content_hash: Option<ResourceBundleGenerationId>,
    active_generation: Option<ConfigurationGeneration>,
}

impl AuditRecoveryKey {
    /// Return the event kind bound by this key.
    pub const fn kind(&self) -> AuditEventKind {
        self.kind
    }

    /// Return the committed revision, when this is a deletion event.
    pub const fn revision(&self) -> Option<ZoneRevision> {
        self.revision
    }
}

/// One redacted configuration audit event.
#[derive(Clone, PartialEq, Eq)]
pub struct AuditEvent {
    kind: AuditEventKind,
    zone: ZoneId,
    resource_type: Option<ResourceTypeName>,
    resource_name_digest: Option<SchemaFingerprint>,
    prior_generation: Option<ConfigurationGeneration>,
    active_generation: Option<ConfigurationGeneration>,
    content_hash: Option<ResourceBundleGenerationId>,
    revision: Option<ZoneRevision>,
    reason: Option<AuditReason>,
    timestamp: Timestamp,
}

impl AuditEvent {
    /// Construct a bundle activation event.
    pub fn generation_activated(
        zone: ZoneId,
        content_hash: ResourceBundleGenerationId,
        active_generation: ConfigurationGeneration,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            kind: AuditEventKind::GenerationActivated,
            zone,
            resource_type: None,
            resource_name_digest: None,
            prior_generation: None,
            active_generation: Some(active_generation),
            content_hash: Some(content_hash),
            revision: None,
            reason: None,
            timestamp,
        }
    }

    /// Construct a bundle rejection event.
    pub fn generation_rejected(zone: ZoneId, reason: AuditReason, timestamp: Timestamp) -> Self {
        Self::generation_rejected_for_bundle(zone, reason, None, timestamp)
    }

    /// Construct a rejection event bound to one candidate bundle identity.
    pub fn generation_rejected_for_bundle(
        zone: ZoneId,
        reason: AuditReason,
        content_hash: Option<ResourceBundleGenerationId>,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            kind: AuditEventKind::GenerationRejected,
            zone,
            resource_type: None,
            resource_name_digest: None,
            prior_generation: None,
            active_generation: None,
            content_hash,
            revision: None,
            reason: Some(reason),
            timestamp,
        }
    }

    /// Construct a request-to-delete event without retaining the raw name.
    pub fn resource_deletion_requested(
        zone: ZoneId,
        resource_type: ResourceTypeName,
        resource_name: &ResourceName,
        prior_generation: ConfigurationGeneration,
        active_generation: ConfigurationGeneration,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            kind: AuditEventKind::ResourceDeletionRequested,
            zone,
            resource_type: Some(resource_type),
            resource_name_digest: Some(resource_name_digest(resource_name)),
            prior_generation: Some(prior_generation),
            active_generation: Some(active_generation),
            content_hash: None,
            revision: None,
            reason: Some(AuditReason::AbsentFromNewGeneration),
            timestamp,
        }
    }

    /// Construct an audit event for an already committed deletion.
    pub fn resource_deleted(
        zone: ZoneId,
        resource_type: ResourceTypeName,
        resource_name_digest: SchemaFingerprint,
        prior_generation: ConfigurationGeneration,
        active_generation: ConfigurationGeneration,
        revision: ZoneRevision,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            kind: AuditEventKind::ResourceDeleted,
            zone,
            resource_type: Some(resource_type),
            resource_name_digest: Some(resource_name_digest),
            prior_generation: Some(prior_generation),
            active_generation: Some(active_generation),
            content_hash: None,
            revision: Some(revision),
            reason: None,
            timestamp,
        }
    }

    /// Construct a generation rollback event.
    pub fn generation_rolled_back(
        zone: ZoneId,
        content_hash: ResourceBundleGenerationId,
        active_generation: ConfigurationGeneration,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            kind: AuditEventKind::GenerationRolledBack,
            zone,
            resource_type: None,
            resource_name_digest: None,
            prior_generation: None,
            active_generation: Some(active_generation),
            content_hash: Some(content_hash),
            revision: None,
            reason: None,
            timestamp,
        }
    }

    /// Construct a per-item foreign-owner collision event.
    pub fn configuration_collision(
        zone: ZoneId,
        resource_type: ResourceTypeName,
        resource_name: &ResourceName,
        active_generation: ConfigurationGeneration,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            kind: AuditEventKind::ConfigurationCollision,
            zone,
            resource_type: Some(resource_type),
            resource_name_digest: Some(resource_name_digest(resource_name)),
            prior_generation: None,
            active_generation: Some(active_generation),
            content_hash: None,
            revision: None,
            reason: Some(AuditReason::ForeignOwner),
            timestamp,
        }
    }

    /// Construct a stalled-cleanup event.
    pub fn cleanup_stalled(
        zone: ZoneId,
        resource_type: ResourceTypeName,
        resource_name_digest: SchemaFingerprint,
        active_generation: ConfigurationGeneration,
        reason: AuditReason,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            kind: AuditEventKind::CleanupStalled,
            zone,
            resource_type: Some(resource_type),
            resource_name_digest: Some(resource_name_digest),
            prior_generation: None,
            active_generation: Some(active_generation),
            content_hash: None,
            revision: None,
            reason: Some(reason),
            timestamp,
        }
    }

    /// Return the event kind.
    pub const fn kind(&self) -> AuditEventKind {
        self.kind
    }

    /// Return the stable mutation event token used by ResourceMutation
    /// projections.
    pub const fn event(&self) -> &'static str {
        match self.kind {
            AuditEventKind::GenerationActivated => "generation-activate",
            AuditEventKind::GenerationRejected => "generation-rejected",
            AuditEventKind::ResourceDeletionRequested => "delete-scheduled",
            AuditEventKind::ResourceDeleted => "deleted",
            AuditEventKind::CleanupStalled => "cleanup-stalled",
            AuditEventKind::GenerationRolledBack => "generation-rollback",
            AuditEventKind::ConfigurationCollision => "collision",
        }
    }

    /// Return the closed trigger token for resource cleanup events.
    pub const fn trigger(&self) -> Option<&'static str> {
        match self.kind {
            AuditEventKind::ResourceDeletionRequested | AuditEventKind::ResourceDeleted => {
                Some("config-cleanup")
            }
            _ => None,
        }
    }

    /// Borrow the Zone used for the authorized sink partition.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the optional resource type.
    pub const fn resource_type(&self) -> Option<&ResourceTypeName> {
        self.resource_type.as_ref()
    }

    /// Borrow the digested resource name, never the raw name.
    pub const fn resource_name_digest(&self) -> Option<&SchemaFingerprint> {
        self.resource_name_digest.as_ref()
    }

    /// Return the prior generation ordinal.
    pub const fn prior_generation(&self) -> Option<ConfigurationGeneration> {
        self.prior_generation
    }

    /// Return the active generation ordinal.
    pub const fn active_generation(&self) -> Option<ConfigurationGeneration> {
        self.active_generation
    }

    /// Borrow the content-derived generation identity, when present.
    pub const fn content_hash(&self) -> Option<&ResourceBundleGenerationId> {
        self.content_hash.as_ref()
    }

    /// Return the store revision that committed a deletion, when present.
    pub const fn revision(&self) -> Option<ZoneRevision> {
        self.revision
    }

    /// Return the closed reason, when present.
    pub const fn reason(&self) -> Option<AuditReason> {
        self.reason
    }

    /// Borrow the event timestamp.
    pub const fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }

    /// Return the recovery key used for exactly-once append.
    pub fn recovery_key(&self) -> AuditRecoveryKey {
        AuditRecoveryKey {
            kind: self.kind,
            revision: self.revision,
            resource_name_digest: self.resource_name_digest.clone(),
            content_hash: self.content_hash.clone(),
            active_generation: self.active_generation,
        }
    }
}

impl core::fmt::Debug for AuditEvent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AuditEvent")
            .field("kind", &self.kind)
            .field("has_zone", &true)
            .field("has_resource_type", &self.resource_type.is_some())
            .field(
                "has_resource_name_digest",
                &self.resource_name_digest.is_some(),
            )
            .field(
                "active_generation",
                &self.active_generation.map(|value| value.get()),
            )
            .field("revision", &self.revision.map(ZoneRevision::get))
            .field("reason", &self.reason)
            .finish()
    }
}

/// Closed audit append failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditError {
    /// The recovery key was already appended.
    AlreadyAppended,
    /// The event belongs to a different Zone partition.
    ZoneMismatch,
}

impl AuditError {
    /// Return the stable failure label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::AlreadyAppended => "audit-already-appended",
            Self::ZoneMismatch => "audit-zone-mismatch",
        }
    }
}

impl core::fmt::Display for AuditError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for AuditError {}

/// In-memory model of a per-Zone append-only audit sink.
///
/// Production persistence is supplied by the Zone audit owner.  The model
/// deliberately has no delete or truncate operation, so cleanup cannot erase
/// prior audit segments while pruning a resource.
#[derive(Debug, Clone)]
pub struct AuditLedger {
    zone: ZoneId,
    events: Vec<AuditEvent>,
    appended: BTreeSet<AuditRecoveryKey>,
}

impl AuditLedger {
    /// Create an empty ledger for one Zone.
    pub fn new(zone: ZoneId) -> Self {
        Self {
            zone,
            events: Vec::new(),
            appended: BTreeSet::new(),
        }
    }

    /// Borrow the ledger's Zone partition.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Append one event exactly once by its committed recovery key.
    pub fn append(&mut self, event: AuditEvent) -> Result<(), AuditError> {
        if event.zone() != &self.zone {
            return Err(AuditError::ZoneMismatch);
        }
        if !self.appended.insert(event.recovery_key()) {
            return Err(AuditError::AlreadyAppended);
        }
        self.events.push(event);
        Ok(())
    }

    /// Borrow events in append order.
    pub fn events(&self) -> &[AuditEvent] {
        &self.events
    }

    /// Return whether a recovery key has already been appended.
    pub fn contains(&self, key: &AuditRecoveryKey) -> bool {
        self.appended.contains(key)
    }
}
