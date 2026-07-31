//! Redacted Volume-state lifecycle audit events.
//!
//! Audit records are emitted to the authoritative Zone audit stream. They may
//! identify the bounded Zone, Volume resource, and declared state schema, but
//! cannot carry content bytes, paths, credentials, process identity, command
//! data, or backend diagnostics.

use std::fmt;
use std::future::Future;

use d2b_contracts::v3::zone_routing::ZonePath;
use d2b_contracts::v3::{
    MigrationPolicy, PersistenceClass, ResourceRef, SchemaVersion, VolumeStateSchemaId,
};
use serde::Serialize;

/// Every Volume-state lifecycle audit event kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeAuditKind {
    /// First provisioning completed.
    VolumeProvisioned,
    /// A migration worker was requested.
    VolumeMigrationStart,
    /// A migration committed.
    VolumeMigrationCommitted,
    /// A migration failed.
    VolumeMigrationFailed,
    /// A precommit migration rollback completed.
    VolumeMigrationRolledBack,
    /// A snapshot completed.
    VolumeSnapshotCreated,
    /// Relocation began.
    VolumeRelocationStart,
    /// Relocation committed.
    VolumeRelocationCommitted,
    /// Incident hold was set.
    VolumeIncidentHoldSet,
    /// Incident hold was cleared.
    VolumeIncidentHoldCleared,
    /// Sealing rotation began.
    VolumeSealingRotationStart,
    /// Sealing rotation committed.
    VolumeSealingRotationCommitted,
    /// Destruction completed.
    VolumeDestroyed,
}

impl VolumeAuditKind {
    /// Every event kind in canonical order.
    pub const ALL: [Self; 13] = [
        Self::VolumeProvisioned,
        Self::VolumeMigrationStart,
        Self::VolumeMigrationCommitted,
        Self::VolumeMigrationFailed,
        Self::VolumeMigrationRolledBack,
        Self::VolumeSnapshotCreated,
        Self::VolumeRelocationStart,
        Self::VolumeRelocationCommitted,
        Self::VolumeIncidentHoldSet,
        Self::VolumeIncidentHoldCleared,
        Self::VolumeSealingRotationStart,
        Self::VolumeSealingRotationCommitted,
        Self::VolumeDestroyed,
    ];
}

/// Closed result of a lifecycle transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeAuditOutcome {
    /// The transition completed.
    Succeeded,
    /// The transition failed closed.
    Failed,
}

/// Closed reason for a failed transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeAuditReason {
    /// The external provisioning marker did not verify.
    MarkerInvalid,
    /// The declared quota could not be satisfied.
    QuotaExceeded,
    /// A schema transition failed.
    MigrationFailed,
    /// The sealing transition failed.
    SealingFailed,
    /// The injected effect adapter was unavailable.
    EffectUnavailable,
    /// A committed transition is awaiting durable audit.
    AuditPending,
}

/// Closed snapshot trigger class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SnapshotTrigger {
    /// An operator explicitly requested the snapshot.
    Manual,
    /// A migration requested a protective snapshot.
    PreMigration,
    /// A relocation requested a protective snapshot.
    PreRelocation,
}

/// Closed actor class for incident-hold transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeAuditActor {
    /// An authorized Zone administrator.
    Admin,
    /// The installed incident controller.
    IncidentController,
}

/// One authorized Zone audit stream record.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeAuditEvent {
    kind: VolumeAuditKind,
    zone: ZonePath,
    volume_ref: ResourceRef,
    outcome: VolumeAuditOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<VolumeAuditReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_id: Option<VolumeStateSchemaId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_version: Option<SchemaVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    persistence_class: Option<PersistenceClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_version: Option<SchemaVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_version: Option<SchemaVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    migration_policy: Option<MigrationPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trigger: Option<SnapshotTrigger>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot_id: Option<d2b_contracts::v3::execution_policy::BoundedToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_execution_ref: Option<ResourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_execution_ref: Option<ResourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actor: Option<VolumeAuditActor>,
}

impl VolumeAuditEvent {
    /// Start a record with only mandatory bounded fields.
    pub const fn new(
        kind: VolumeAuditKind,
        zone: ZonePath,
        volume_ref: ResourceRef,
        outcome: VolumeAuditOutcome,
    ) -> Self {
        Self {
            kind,
            zone,
            volume_ref,
            outcome,
            reason: None,
            schema_id: None,
            schema_version: None,
            persistence_class: None,
            from_version: None,
            to_version: None,
            migration_policy: None,
            trigger: None,
            snapshot_id: None,
            from_execution_ref: None,
            to_execution_ref: None,
            actor: None,
        }
    }

    /// Attach a closed failure reason.
    pub const fn with_reason(mut self, reason: VolumeAuditReason) -> Self {
        self.reason = Some(reason);
        self
    }

    /// Attach a declared schema identity and version.
    pub fn with_schema(
        mut self,
        schema_id: VolumeStateSchemaId,
        schema_version: SchemaVersion,
    ) -> Self {
        self.schema_id = Some(schema_id);
        self.schema_version = Some(schema_version);
        self
    }

    /// Attach the persistence class used for first provision.
    pub const fn with_persistence(mut self, persistence: PersistenceClass) -> Self {
        self.persistence_class = Some(persistence);
        self
    }

    /// Attach a schema migration transition.
    pub const fn with_migration(
        mut self,
        from: SchemaVersion,
        to: SchemaVersion,
        policy: MigrationPolicy,
    ) -> Self {
        self.from_version = Some(from);
        self.to_version = Some(to);
        self.migration_policy = Some(policy);
        self
    }

    /// Attach the closed snapshot trigger class.
    pub const fn with_snapshot_trigger(mut self, trigger: SnapshotTrigger) -> Self {
        self.trigger = Some(trigger);
        self
    }

    /// Attach the bounded opaque snapshot identity.
    pub fn with_snapshot_id(
        mut self,
        snapshot_id: d2b_contracts::v3::execution_policy::BoundedToken,
    ) -> Self {
        self.snapshot_id = Some(snapshot_id);
        self
    }

    /// Attach the source execution resource for relocation.
    pub fn with_relocation_source(mut self, execution_ref: ResourceRef) -> Self {
        self.from_execution_ref = Some(execution_ref);
        self
    }

    /// Attach the destination execution resource for relocation.
    pub fn with_relocation_destination(mut self, execution_ref: ResourceRef) -> Self {
        self.to_execution_ref = Some(execution_ref);
        self
    }

    /// Attach the closed actor class for an incident-hold transition.
    pub const fn with_actor(mut self, actor: VolumeAuditActor) -> Self {
        self.actor = Some(actor);
        self
    }

    /// Return the event kind.
    pub const fn kind(&self) -> VolumeAuditKind {
        self.kind
    }

    /// Return the transition result.
    pub const fn outcome(&self) -> VolumeAuditOutcome {
        self.outcome
    }
}

impl fmt::Debug for VolumeAuditEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VolumeAuditEvent")
            .field("kind", &self.kind)
            .field("outcome", &self.outcome)
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

/// A path-free audit emission failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeAuditError {
    /// The authoritative Zone stream is unavailable.
    StreamUnavailable,
    /// The event could not be encoded within the stream bound.
    EncodingFailed,
}

impl VolumeAuditError {
    /// Return the stable redacted code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::StreamUnavailable => "volume-audit-stream-unavailable",
            Self::EncodingFailed => "volume-audit-encoding-failed",
        }
    }
}

impl fmt::Display for VolumeAuditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for VolumeAuditError {}

/// Injected authoritative Zone audit stream.
pub trait VolumeAuditSink: Send + Sync {
    /// Emit one bounded lifecycle event.
    fn emit(
        &self,
        event: &VolumeAuditEvent,
    ) -> impl Future<Output = Result<(), VolumeAuditError>> + Send;
}

/// Emit one Volume lifecycle event through the injected Zone stream.
pub async fn emit_volume_event<S: VolumeAuditSink>(
    sink: &S,
    event: &VolumeAuditEvent,
) -> Result<(), VolumeAuditError> {
    sink.emit(event).await
}
