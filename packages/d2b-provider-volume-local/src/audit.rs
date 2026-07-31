//! Redacted Volume-state lifecycle audit events.
//!
//! Audit records are emitted to the authoritative Zone audit stream. They may
//! identify the bounded Zone, Volume resource, and declared state schema, but
//! cannot carry content bytes, paths, credentials, process identity, command
//! data, or backend diagnostics.

use std::fmt::{self, Write as _};
use std::future::Future;

use d2b_contracts::v3::volume::EntryType;
use d2b_contracts::v3::zone_routing::ZonePath;
use d2b_contracts::v3::{
    MigrationPolicy, PersistenceClass, ResourceGeneration, ResourceRef, SchemaVersion,
    VolumeStateSchemaId,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Every Volume-state lifecycle audit event kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeAuditKind {
    /// First provisioning completed.
    VolumeProvisioned,
    /// A declared layout entry was repaired.
    VolumeLayoutRepaired,
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
    /// Sealing rotation failed terminally.
    VolumeSealingRotationFailed,
    /// Sealing rotation committed.
    VolumeSealingRotationCommitted,
    /// Destruction completed.
    VolumeDestroyed,
    /// A provisioning marker was checked.
    VolumeMarkerCheck,
    /// A write was rejected because its quota was exceeded.
    VolumeQuotaExceeded,
    /// A closure-only store-view synchronization completed.
    VolumeStoreSyncComplete,
}

impl VolumeAuditKind {
    /// Every event kind in canonical order.
    pub const ALL: [Self; 18] = [
        Self::VolumeProvisioned,
        Self::VolumeLayoutRepaired,
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
        Self::VolumeSealingRotationFailed,
        Self::VolumeSealingRotationCommitted,
        Self::VolumeDestroyed,
        Self::VolumeMarkerCheck,
        Self::VolumeQuotaExceeded,
        Self::VolumeStoreSyncComplete,
    ];
}

/// Broker-owned Volume operation audit kinds named by the provider contract.
///
/// These names are catalogued here for completeness, but their records remain
/// owned and emitted by the privileged broker rather than by the Zone stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VolumeBrokerAuditKind {
    /// A TPM state directory was provisioned or reconciled.
    PrepareSwtpmDir,
    /// A declared layout entry was created.
    ProvisionLayoutEntry,
    /// A declared layout entry was repaired.
    RepairLayoutEntry,
    /// A declared layout entry was removed.
    CleanupLayoutEntry,
    /// A closure-only store-view synchronization completed.
    StoreSyncComplete,
    /// A sealing-key rotation committed or recovered.
    RotateSealingKey,
}

impl VolumeBrokerAuditKind {
    /// Every broker-owned kind in canonical order.
    pub const ALL: [Self; 6] = [
        Self::PrepareSwtpmDir,
        Self::ProvisionLayoutEntry,
        Self::RepairLayoutEntry,
        Self::CleanupLayoutEntry,
        Self::StoreSyncComplete,
        Self::RotateSealingKey,
    ];

    /// Return the stable broker operation name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PrepareSwtpmDir => "PrepareSwtpmDir",
            Self::ProvisionLayoutEntry => "ProvisionLayoutEntry",
            Self::RepairLayoutEntry => "RepairLayoutEntry",
            Self::CleanupLayoutEntry => "CleanupLayoutEntry",
            Self::StoreSyncComplete => "StoreSyncComplete",
            Self::RotateSealingKey => "RotateSealingKey",
        }
    }
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

/// A fixed-size digest of audit identity that never exposes its input.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VolumeAuditDigest([u8; 32]);

impl VolumeAuditDigest {
    /// Derive an actor digest for an incident-hold transition.
    pub fn actor(value: &[u8]) -> Self {
        Self::derive(b"d2b-volume-audit-actor-v1\0", value)
    }

    /// Derive an operation-ID digest for a sealing transition.
    pub fn operation_id(value: &[u8]) -> Self {
        Self::derive(b"d2b-volume-audit-operation-v1\0", value)
    }

    fn derive(domain: &[u8], value: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(domain);
        hasher.update(value);
        Self(hasher.finalize().into())
    }
}

impl Serialize for VolumeAuditDigest {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut rendered = String::with_capacity(71);
        rendered.push_str("sha256:");
        for byte in self.0 {
            write!(&mut rendered, "{byte:02x}").expect("writing to a String cannot fail");
        }
        serializer.serialize_str(&rendered)
    }
}

impl fmt::Debug for VolumeAuditDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VolumeAuditDigest(<redacted>)")
    }
}

impl fmt::Display for VolumeAuditDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VolumeAuditDigest(<redacted>)")
    }
}

/// Closed layout-repair action class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeRepairAction {
    /// Reconcile owner or group identity.
    Owner,
    /// Reconcile mode bits.
    Mode,
    /// Reconcile the declared ACL.
    Acl,
    /// Reconcile more than one declared property.
    Combined,
}

/// Closed result class used by marker and sealing audit events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeAuditResultClass {
    /// The checked state was valid.
    Verified,
    /// The checked state was missing.
    Missing,
    /// The checked state had been replaced.
    Replaced,
    /// A sealing transition committed normally.
    Committed,
    /// A sealing transition recovered an already-committed result.
    Recovered,
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
    actor_digest: Option<VolumeAuditDigest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_id_digest: Option<VolumeAuditDigest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entry_type: Option<EntryType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action_class: Option<VolumeRepairAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_class: Option<VolumeAuditResultClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_generation: Option<ResourceGeneration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_generation: Option<ResourceGeneration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_number: Option<ResourceGeneration>,
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
            actor_digest: None,
            operation_id_digest: None,
            entry_type: None,
            action_class: None,
            result_class: None,
            from_generation: None,
            to_generation: None,
            generation_number: None,
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

    /// Attach the fixed digest of an incident-hold actor.
    pub const fn with_actor_digest(mut self, actor: VolumeAuditDigest) -> Self {
        self.actor_digest = Some(actor);
        self
    }

    /// Attach the fixed digest of a sealing operation ID.
    pub const fn with_operation_id_digest(mut self, operation_id: VolumeAuditDigest) -> Self {
        self.operation_id_digest = Some(operation_id);
        self
    }

    /// Attach closed layout-repair details without an entry path or ACL value.
    pub const fn with_layout_repair(
        mut self,
        entry_type: EntryType,
        action: VolumeRepairAction,
    ) -> Self {
        self.entry_type = Some(entry_type);
        self.action_class = Some(action);
        self
    }

    /// Attach a closed marker or transition result class.
    pub const fn with_result_class(mut self, result: VolumeAuditResultClass) -> Self {
        self.result_class = Some(result);
        self
    }

    /// Attach a sealing generation transition.
    pub const fn with_generation_transition(
        mut self,
        from: ResourceGeneration,
        to: ResourceGeneration,
    ) -> Self {
        self.from_generation = Some(from);
        self.to_generation = Some(to);
        self
    }

    /// Attach a completed store-view generation number.
    pub const fn with_generation_number(mut self, generation: ResourceGeneration) -> Self {
        self.generation_number = Some(generation);
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
