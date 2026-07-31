//! Bounded snapshot policy, status, and retention planning.
//!
//! Filesystem copying remains an EphemeralProcess effect. This module decides
//! whether to dispatch that worker and which opaque snapshots have expired.

use std::collections::BTreeSet;
use std::fmt;

use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::{SchemaVersion, Timestamp};
use serde::Serialize;

use crate::audit::{SnapshotTrigger, VolumeAuditKind};

/// Signed snapshot worker template name.
pub const SNAPSHOT_WORKER_TEMPLATE: &str = "volume-snapshot-worker";
/// Provider-private snapshot subtree, never exposed through component views.
pub const SNAPSHOT_PRIVATE_SUBTREE: &str = ".snapshots";
/// Validated bounded snapshot policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotPolicy {
    retain_count: usize,
    retain_duration_hours: u64,
    trigger_on_migration: bool,
    trigger_on_relocation: bool,
}

impl SnapshotPolicy {
    /// Construct policy with a nonzero bounded retained count.
    pub fn new(
        retain_count: usize,
        retain_duration_hours: u64,
        trigger_on_migration: bool,
        trigger_on_relocation: bool,
    ) -> Result<Self, SnapshotError> {
        if retain_count == 0 || retain_count.checked_add(1).is_none() {
            return Err(SnapshotError::PolicyInvalid);
        }
        Ok(Self {
            retain_count,
            retain_duration_hours,
            trigger_on_migration,
            trigger_on_relocation,
        })
    }

    /// Return the retained-count limit.
    pub const fn retain_count(self) -> usize {
        self.retain_count
    }

    /// Return the TTL in hours; zero disables time-based expiry.
    pub const fn retain_duration_hours(self) -> u64 {
        self.retain_duration_hours
    }

    /// Return whether this trigger is enabled by policy.
    pub const fn permits(self, trigger: SnapshotTrigger) -> bool {
        match trigger {
            SnapshotTrigger::Manual => true,
            SnapshotTrigger::PreMigration => self.trigger_on_migration,
            SnapshotTrigger::PreRelocation => self.trigger_on_relocation,
        }
    }
}

/// Opaque snapshot identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct SnapshotId(BoundedToken);

impl SnapshotId {
    /// Parse one bounded opaque identity.
    pub fn parse(value: impl Into<String>) -> Result<Self, SnapshotError> {
        BoundedToken::parse(value)
            .map(Self)
            .map_err(|_| SnapshotError::SnapshotIdInvalid)
    }
}

impl fmt::Debug for SnapshotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SnapshotId(<redacted>)")
    }
}

/// Snapshot phase projected into Volume status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum SnapshotPhase {
    /// Snapshot bytes are complete and immutable.
    Ready,
    /// The worker failed without publishing a ready snapshot.
    Failed,
    /// Retention selected the snapshot for cleanup.
    Expired,
}

/// Bounded snapshot status without paths or content digests.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotStatus {
    id: SnapshotId,
    created_at: Timestamp,
    #[serde(skip)]
    created_hour: u64,
    schema_version: SchemaVersion,
    size_bytes: u64,
    trigger: SnapshotTrigger,
    phase: SnapshotPhase,
}

impl SnapshotStatus {
    /// Construct one terminal worker observation.
    pub fn new(
        id: SnapshotId,
        created_at: Timestamp,
        created_hour: u64,
        schema_version: SchemaVersion,
        size_bytes: u64,
        trigger: SnapshotTrigger,
        phase: SnapshotPhase,
    ) -> Result<Self, SnapshotError> {
        if phase == SnapshotPhase::Expired {
            return Err(SnapshotError::InvalidTransition);
        }
        Ok(Self {
            id,
            created_at,
            created_hour,
            schema_version,
            size_bytes,
            trigger,
            phase,
        })
    }

    /// Borrow the opaque snapshot identity.
    pub const fn id(&self) -> &SnapshotId {
        &self.id
    }

    /// Borrow the canonical creation timestamp.
    pub const fn created_at(&self) -> &Timestamp {
        &self.created_at
    }

    /// Return the snapshotted schema version.
    pub const fn schema_version(&self) -> SchemaVersion {
        self.schema_version
    }

    /// Return the informational size estimate.
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Return the closed trigger class.
    pub const fn trigger(&self) -> SnapshotTrigger {
        self.trigger
    }

    /// Return the projected terminal phase.
    pub const fn phase(&self) -> SnapshotPhase {
        self.phase
    }
}

impl fmt::Debug for SnapshotStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotStatus")
            .field("phase", &self.phase)
            .field("trigger", &self.trigger)
            .finish_non_exhaustive()
    }
}

/// Snapshot worker dispatch decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotDispatch {
    /// The existing audit event emitted after successful worker completion.
    pub success_audit: VolumeAuditKind,
}

/// Closed snapshot policy or state failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotError {
    /// Retention count was zero or exceeded the implementation bound.
    PolicyInvalid,
    /// An opaque snapshot ID violated its fixed bound.
    SnapshotIdInvalid,
    /// Policy disables this automatic trigger.
    TriggerDisabled,
    /// A duplicate snapshot identity was observed.
    DuplicateSnapshot,
    /// The bounded status catalogue is full.
    CatalogueFull,
    /// A phase transition was invalid.
    InvalidTransition,
    /// Time arithmetic overflowed.
    TimeInvalid,
    /// Equal creation times straddled the retained-count boundary.
    RetentionOrderAmbiguous,
}

impl SnapshotError {
    /// Return the stable redacted error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::PolicyInvalid => "volume-snapshot-policy-invalid",
            Self::SnapshotIdInvalid => "volume-snapshot-id-invalid",
            Self::TriggerDisabled => "volume-snapshot-trigger-disabled",
            Self::DuplicateSnapshot => "volume-snapshot-duplicate",
            Self::CatalogueFull => "volume-snapshot-catalogue-full",
            Self::InvalidTransition => "volume-snapshot-transition-invalid",
            Self::TimeInvalid => "volume-snapshot-time-invalid",
            Self::RetentionOrderAmbiguous => "volume-snapshot-retention-order-ambiguous",
        }
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for SnapshotError {}

/// Bounded in-memory projection of snapshot status entries.
pub struct SnapshotCatalog {
    policy: SnapshotPolicy,
    records: Vec<SnapshotStatus>,
}

impl SnapshotCatalog {
    /// Create an empty catalogue for one Volume.
    pub const fn new(policy: SnapshotPolicy) -> Self {
        Self {
            policy,
            records: Vec::new(),
        }
    }

    /// Plan one policy-authorized worker dispatch.
    pub fn plan_dispatch(
        &self,
        trigger: SnapshotTrigger,
    ) -> Result<SnapshotDispatch, SnapshotError> {
        if !self.policy.permits(trigger) {
            return Err(SnapshotError::TriggerDisabled);
        }
        Ok(SnapshotDispatch {
            success_audit: VolumeAuditKind::VolumeSnapshotCreated,
        })
    }

    /// Record one terminal worker result for status projection.
    pub fn record(&mut self, status: SnapshotStatus) -> Result<(), SnapshotError> {
        if self.records.len() >= self.policy.retain_count() + 1 {
            return Err(SnapshotError::CatalogueFull);
        }
        if self.records.iter().any(|entry| entry.id == status.id) {
            return Err(SnapshotError::DuplicateSnapshot);
        }
        self.records.push(status);
        Ok(())
    }

    /// Borrow the bounded Volume status snapshot list.
    pub fn records(&self) -> &[SnapshotStatus] {
        &self.records
    }

    /// Select snapshots expired by TTL or by newest-first retained count.
    pub fn retention_plan(&self, now_hour: u64) -> Result<Vec<SnapshotId>, SnapshotError> {
        let mut live: Vec<&SnapshotStatus> = self
            .records
            .iter()
            .filter(|entry| entry.phase != SnapshotPhase::Expired)
            .collect();
        live.sort_by(|left, right| right.created_at.cmp(&left.created_at));

        let retained = self.policy.retain_count();
        if live.len() > retained && live[retained - 1].created_at == live[retained].created_at {
            return Err(SnapshotError::RetentionOrderAmbiguous);
        }

        let ttl = self.policy.retain_duration_hours();
        let mut expired = BTreeSet::new();
        for (position, entry) in live.into_iter().enumerate() {
            let age = now_hour
                .checked_sub(entry.created_hour)
                .ok_or(SnapshotError::TimeInvalid)?;
            if position >= retained || (ttl != 0 && age >= ttl) {
                expired.insert(entry.id.clone());
            }
        }
        Ok(expired.into_iter().collect())
    }

    /// Mark selected snapshots expired after their cleanup request commits.
    pub fn apply_expired(&mut self, expired: &[SnapshotId]) {
        for record in &mut self.records {
            if expired.contains(&record.id) {
                record.phase = SnapshotPhase::Expired;
            }
        }
    }

    /// Remove expired status entries after their cleanup effects complete.
    pub fn remove_expired(&mut self) {
        self.records
            .retain(|record| record.phase != SnapshotPhase::Expired);
    }
}

impl fmt::Debug for SnapshotCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotCatalog")
            .field("record_count", &self.records.len())
            .finish()
    }
}
