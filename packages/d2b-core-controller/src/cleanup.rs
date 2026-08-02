//! Pending-cleanup status and prior-generation retention policy.
//!
//! This module computes state only. Store transactions, bundle copies, watch
//! delivery, finalizer effects, and audit appends remain responsibilities of
//! their eventual production adapters.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::v3::{
    ResourceBundleGenerationId, Timestamp, ZoneRevision, execution_policy::DurationMs,
};

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
    /// Construct a status projection from an aggregate count.
    pub(crate) const fn from_count(pending_count: usize) -> Self {
        if pending_count == 0 {
            Self {
                state: PendingCleanupState::False,
                phase: CleanupZonePhase::Ready,
                pending_count,
            }
        } else {
            Self {
                state: PendingCleanupState::True,
                phase: CleanupZonePhase::Degraded,
                pending_count,
            }
        }
    }

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
    PendingCleanupCondition::from_count(pending_count)
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

/// Default successful EphemeralProcess retention, in milliseconds.
pub const EPHEMERAL_PROCESS_SUCCESSFUL_TTL_MS_DEFAULT: u64 = 3_600_000;

/// Default failed EphemeralProcess retention, in milliseconds.
pub const EPHEMERAL_PROCESS_FAILED_TTL_MS_DEFAULT: u64 = 86_400_000;

/// Closed terminal phases understood by the EphemeralProcess cleanup handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EphemeralProcessPhase {
    /// The process has not completed and must never age out.
    Pending,
    /// The process completed successfully.
    Succeeded,
    /// The process completed unsuccessfully.
    Failed,
    /// The process outcome cannot be proven after a restart.
    Unknown,
}

impl EphemeralProcessPhase {
    /// Return the stable phase label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Unknown => "Unknown",
        }
    }
}

/// Why a terminal EphemeralProcess is not currently deletable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EphemeralCleanupBlock {
    /// A non-terminal process never participates in TTL cleanup.
    NotTerminal,
    /// The completion timestamp is not yet available.
    CompletionUnobserved,
    /// An incident hold requires an explicit release.
    IncidentHold,
    /// A finalizer still owns teardown.
    Finalizer,
    /// The owner is being deleted and must drain first.
    OwnerDeletion,
    /// The terminal result is ambiguous after restart.
    UnknownOutcome,
}

impl EphemeralCleanupBlock {
    /// Return the stable condition reason.
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotTerminal => "ttl-pending",
            Self::CompletionUnobserved => "completion-unobserved",
            Self::IncidentHold => "incident-held",
            Self::Finalizer => "finalizer-pending",
            Self::OwnerDeletion => "owner-deletion-pending",
            Self::UnknownOutcome => "unknown-outcome",
        }
    }
}

/// Closed failure from the EphemeralProcess cleanup controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EphemeralCleanupError {
    /// A timestamp could not be parsed or represented.
    InvalidTimestamp,
    /// The supplied expected revision is zero.
    InvalidRevision,
    /// The observed resource was not an EphemeralProcess key.
    WrongResourceType,
    /// A status update was attempted for an unknown tracked resource.
    NotTracked,
    /// A watch event carried a different revision than the Delete request.
    RevisionMismatch,
    /// A supplied TTL exceeded the EphemeralProcess contract bound.
    TtlOutOfRange,
}

impl EphemeralCleanupError {
    /// Return the stable failure label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::InvalidTimestamp => "ephemeral-cleanup-timestamp-invalid",
            Self::InvalidRevision => "ephemeral-cleanup-revision-invalid",
            Self::WrongResourceType => "ephemeral-cleanup-resource-type-invalid",
            Self::NotTracked => "ephemeral-cleanup-resource-not-tracked",
            Self::RevisionMismatch => "ephemeral-cleanup-revision-mismatch",
            Self::TtlOutOfRange => "ephemeral-cleanup-ttl-out-of-range",
        }
    }
}

impl core::fmt::Display for EphemeralCleanupError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for EphemeralCleanupError {}

/// One watch observation consumed by the TTL cleanup controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EphemeralProcessObservation {
    key: ResourceKey,
    phase: EphemeralProcessPhase,
    completed_at: Option<Timestamp>,
    successful_ttl_ms: u64,
    failed_ttl_ms: u64,
    incident_hold: bool,
    pending_finalizers: u32,
    owner_deletion_requested: bool,
    cleanup_eligible_at: Option<Timestamp>,
    expected_revision: ZoneRevision,
}

impl EphemeralProcessObservation {
    /// Construct an observation from ResourceClient status fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: ResourceKey,
        phase: EphemeralProcessPhase,
        completed_at: Option<Timestamp>,
        successful_ttl: &DurationMs,
        failed_ttl: &DurationMs,
        incident_hold: bool,
        pending_finalizers: u32,
        owner_deletion_requested: bool,
        cleanup_eligible_at: Option<Timestamp>,
        expected_revision: ZoneRevision,
    ) -> Result<Self, EphemeralCleanupError> {
        if key.type_name().as_str() != "EphemeralProcess" {
            return Err(EphemeralCleanupError::WrongResourceType);
        }
        if expected_revision.get() == 0 {
            return Err(EphemeralCleanupError::InvalidRevision);
        }
        if successful_ttl.as_millis() > 7 * 86_400_000 || failed_ttl.as_millis() > 30 * 86_400_000 {
            return Err(EphemeralCleanupError::TtlOutOfRange);
        }
        if completed_at
            .as_ref()
            .is_some_and(|timestamp| timestamp_millis(timestamp).is_none())
            || cleanup_eligible_at
                .as_ref()
                .is_some_and(|timestamp| timestamp_millis(timestamp).is_none())
        {
            return Err(EphemeralCleanupError::InvalidTimestamp);
        }
        Ok(Self {
            key,
            phase,
            completed_at,
            successful_ttl_ms: successful_ttl.as_millis(),
            failed_ttl_ms: failed_ttl.as_millis(),
            incident_hold,
            pending_finalizers,
            owner_deletion_requested,
            cleanup_eligible_at,
            expected_revision,
        })
    }

    /// Construct an observation with the frozen default TTLs.
    #[allow(clippy::too_many_arguments)]
    pub fn with_defaults(
        key: ResourceKey,
        phase: EphemeralProcessPhase,
        completed_at: Option<Timestamp>,
        incident_hold: bool,
        pending_finalizers: u32,
        owner_deletion_requested: bool,
        cleanup_eligible_at: Option<Timestamp>,
        expected_revision: ZoneRevision,
    ) -> Result<Self, EphemeralCleanupError> {
        Self::new(
            key,
            phase,
            completed_at,
            &duration_ms(EPHEMERAL_PROCESS_SUCCESSFUL_TTL_MS_DEFAULT),
            &duration_ms(EPHEMERAL_PROCESS_FAILED_TTL_MS_DEFAULT),
            incident_hold,
            pending_finalizers,
            owner_deletion_requested,
            cleanup_eligible_at,
            expected_revision,
        )
    }

    /// Borrow the observed resource key.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Return the observed phase.
    pub const fn phase(&self) -> EphemeralProcessPhase {
        self.phase
    }

    /// Borrow the terminal completion timestamp.
    pub const fn completed_at(&self) -> Option<&Timestamp> {
        self.completed_at.as_ref()
    }

    /// Return the expected revision used for Delete.
    pub const fn expected_revision(&self) -> ZoneRevision {
        self.expected_revision
    }
}

/// One decision returned to the ResourceClient caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EphemeralCleanupDecision {
    /// Persist `cleanupEligibleAt` with one expected revision.
    UpdateStatus {
        cleanup_eligible_at: Timestamp,
        incident_held: bool,
        expected_revision: ZoneRevision,
    },
    /// Retry at the bounded TTL expiry instead of polling continuously.
    RequeueAt {
        at: Timestamp,
        expected_revision: ZoneRevision,
    },
    /// Issue the normal Delete API request; the handler never removes a row.
    Delete { expected_revision: ZoneRevision },
    /// Keep the row and report the closed blocking condition.
    Blocked {
        reason: EphemeralCleanupBlock,
        cleanup_eligible_at: Option<Timestamp>,
    },
    /// No action is needed for this observation.
    Noop,
}

/// Durable cleanup state retained across controller restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EphemeralCleanupRecord {
    key: ResourceKey,
    cleanup_eligible_at: Option<Timestamp>,
    expected_revision: ZoneRevision,
    incident_held: bool,
}

impl EphemeralCleanupRecord {
    /// Borrow the tracked key.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Borrow the computed eligibility timestamp.
    pub const fn cleanup_eligible_at(&self) -> Option<&Timestamp> {
        self.cleanup_eligible_at.as_ref()
    }

    /// Return the expected Delete revision.
    pub const fn expected_revision(&self) -> ZoneRevision {
        self.expected_revision
    }

    /// Whether the status projection carries an incident hold.
    pub const fn incident_held(&self) -> bool {
        self.incident_held
    }
}

/// Pure EphemeralProcess TTL cleanup controller.
///
/// It consumes watch observations, emits status or Delete intents, and never
/// removes a row directly. Its only process-local state is a bounded cache of
/// status projections; callers can snapshot and restore it at startup.
#[derive(Debug, Default, Clone)]
pub struct EphemeralProcessCleanupController {
    records: BTreeMap<ResourceKey, EphemeralCleanupRecord>,
}

impl EphemeralProcessCleanupController {
    /// Restore status projections that were persisted by the Zone store.
    pub fn restore(records: impl IntoIterator<Item = EphemeralCleanupRecord>) -> Self {
        Self {
            records: records
                .into_iter()
                .map(|record| (record.key.clone(), record))
                .collect(),
        }
    }

    /// Snapshot the bounded status projections for restart recovery.
    pub fn snapshot(&self) -> Vec<EphemeralCleanupRecord> {
        self.records.values().cloned().collect()
    }

    /// Borrow a tracked status projection.
    pub fn record(&self, key: &ResourceKey) -> Option<&EphemeralCleanupRecord> {
        self.records.get(key)
    }

    /// Reconcile one watch observation at a supplied clock instant.
    pub fn reconcile(
        &mut self,
        observation: EphemeralProcessObservation,
        now: &Timestamp,
    ) -> Result<EphemeralCleanupDecision, EphemeralCleanupError> {
        let key = observation.key.clone();
        let existing = self.records.get(&key);
        let had_record = existing.is_some();
        let prior_incident_held = existing.is_some_and(EphemeralCleanupRecord::incident_held);
        let record = self
            .records
            .entry(key.clone())
            .or_insert_with(|| EphemeralCleanupRecord {
                key: key.clone(),
                cleanup_eligible_at: observation.cleanup_eligible_at.clone(),
                expected_revision: observation.expected_revision,
                incident_held: observation.incident_hold,
            });
        record.expected_revision = observation.expected_revision;
        record.incident_held = observation.incident_hold;

        let terminal = match observation.phase {
            EphemeralProcessPhase::Succeeded | EphemeralProcessPhase::Failed => true,
            EphemeralProcessPhase::Pending => {
                return Ok(EphemeralCleanupDecision::Blocked {
                    reason: EphemeralCleanupBlock::NotTerminal,
                    cleanup_eligible_at: None,
                });
            }
            EphemeralProcessPhase::Unknown => {
                return Ok(EphemeralCleanupDecision::Blocked {
                    reason: EphemeralCleanupBlock::UnknownOutcome,
                    cleanup_eligible_at: None,
                });
            }
        };
        if !terminal {
            unreachable!("all terminal phases were handled above");
        }
        let Some(completed_at) = observation.completed_at.as_ref() else {
            return Ok(EphemeralCleanupDecision::Blocked {
                reason: EphemeralCleanupBlock::CompletionUnobserved,
                cleanup_eligible_at: None,
            });
        };
        let ttl_ms = match observation.phase {
            EphemeralProcessPhase::Succeeded => observation.successful_ttl_ms,
            EphemeralProcessPhase::Failed => observation.failed_ttl_ms,
            EphemeralProcessPhase::Pending | EphemeralProcessPhase::Unknown => 0,
        };
        let computed = add_millis(completed_at, ttl_ms)?;
        // `cleanupEligibleAt` is a derived status field, not an authority
        // supplied by the resource. Recompute it on every terminal watch and
        // repair a missing, stale, or tampered projection before considering
        // Delete. A caller-provided value that matches the derivation is
        // already persisted and needs no redundant status write.
        let supplied_eligible_is_valid =
            observation.cleanup_eligible_at.as_ref() == Some(&computed);
        let status_needs_update = !supplied_eligible_is_valid
            || (had_record && prior_incident_held != observation.incident_hold);
        record.cleanup_eligible_at = Some(computed.clone());
        if status_needs_update {
            return Ok(EphemeralCleanupDecision::UpdateStatus {
                cleanup_eligible_at: computed,
                incident_held: observation.incident_hold,
                expected_revision: observation.expected_revision,
            });
        }

        if observation.incident_hold {
            return Ok(EphemeralCleanupDecision::Blocked {
                reason: EphemeralCleanupBlock::IncidentHold,
                cleanup_eligible_at: Some(computed),
            });
        }
        if observation.pending_finalizers > 0 {
            return Ok(EphemeralCleanupDecision::Blocked {
                reason: EphemeralCleanupBlock::Finalizer,
                cleanup_eligible_at: Some(computed),
            });
        }
        if observation.owner_deletion_requested {
            return Ok(EphemeralCleanupDecision::Blocked {
                reason: EphemeralCleanupBlock::OwnerDeletion,
                cleanup_eligible_at: Some(computed),
            });
        }
        if timestamp_millis(now).ok_or(EphemeralCleanupError::InvalidTimestamp)?
            < timestamp_millis(&computed).ok_or(EphemeralCleanupError::InvalidTimestamp)?
        {
            return Ok(EphemeralCleanupDecision::RequeueAt {
                at: computed,
                expected_revision: observation.expected_revision,
            });
        }
        Ok(EphemeralCleanupDecision::Delete {
            expected_revision: observation.expected_revision,
        })
    }

    /// Consume a matching Deleted watch event and forget the local projection.
    pub fn observe_deleted(
        &mut self,
        key: &ResourceKey,
        revision: ZoneRevision,
    ) -> Result<(), EphemeralCleanupError> {
        let Some(record) = self.records.get(key) else {
            return Err(EphemeralCleanupError::NotTracked);
        };
        if record.expected_revision != revision {
            return Err(EphemeralCleanupError::RevisionMismatch);
        }
        self.records.remove(key);
        Ok(())
    }
}

fn duration_ms(value: u64) -> DurationMs {
    DurationMs::parse(format!("{value}ms"), 0, u64::MAX).expect("bounded default duration is valid")
}

fn timestamp_millis(timestamp: &Timestamp) -> Option<i128> {
    let value = timestamp.as_str();
    if value.len() != 24 {
        return None;
    }
    let number = |start: usize, end: usize| value.get(start..end)?.parse::<i128>().ok();
    if value.as_bytes()[4] != b'-'
        || value.as_bytes()[7] != b'-'
        || value.as_bytes()[10] != b'T'
        || value.as_bytes()[13] != b':'
        || value.as_bytes()[16] != b':'
        || value.as_bytes()[19] != b'.'
        || value.as_bytes()[23] != b'Z'
    {
        return None;
    }
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    let millis = number(20, 23)?;
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
        || millis > 999
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some((((days * 24 + hour) * 60 + minute) * 60 + second) * 1_000 + millis)
}

fn add_millis(timestamp: &Timestamp, amount: u64) -> Result<Timestamp, EphemeralCleanupError> {
    let millis = timestamp_millis(timestamp).ok_or(EphemeralCleanupError::InvalidTimestamp)?;
    timestamp_from_millis(
        millis
            .checked_add(i128::from(amount))
            .ok_or(EphemeralCleanupError::InvalidTimestamp)?,
    )
}

fn timestamp_from_millis(value: i128) -> Result<Timestamp, EphemeralCleanupError> {
    if value < 0 {
        return Err(EphemeralCleanupError::InvalidTimestamp);
    }
    let days = value / 86_400_000;
    let remainder = value % 86_400_000;
    let (year, month, day) = civil_from_days(days);
    if !(0..=9999).contains(&year) {
        return Err(EphemeralCleanupError::InvalidTimestamp);
    }
    let hour = remainder / 3_600_000;
    let minute = (remainder / 60_000) % 60;
    let second = (remainder / 1_000) % 60;
    let millis = remainder % 1_000;
    Timestamp::parse(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    ))
    .map_err(|_| EphemeralCleanupError::InvalidTimestamp)
}

fn days_in_month(year: i128, month: i128) -> i128 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Howard Hinnant's proleptic Gregorian civil-date conversion, expressed with
// the Unix epoch as day zero. Inputs are bounded by Timestamp's four-digit
// year parser, so the arithmetic cannot overflow.
fn days_from_civil(year: i128, month: i128, day: i128) -> i128 {
    let adjusted_year = year - i128::from(month <= 2);
    let era = (if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    }) / 400;
    let year_of_era = adjusted_year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i128) -> (i128, i128, i128) {
    let shifted = days + 719_468;
    let era = (if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    }) / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + i128::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod ephemeral_cleanup_tests {
    use super::*;
    use d2b_contracts::v3::{ResourceName, ResourceTypeName};

    fn key() -> ResourceKey {
        ResourceKey::new(
            ResourceTypeName::parse("EphemeralProcess").unwrap(),
            ResourceName::parse("job").unwrap(),
        )
    }

    fn timestamp(value: &str) -> Timestamp {
        Timestamp::parse(value).unwrap()
    }

    #[test]
    fn terminal_success_uses_one_hour_ttl_and_requeues_without_polling() {
        let completed = timestamp("2026-08-01T00:00:00.000Z");
        let mut controller = EphemeralProcessCleanupController::default();
        let observation = EphemeralProcessObservation::with_defaults(
            key(),
            EphemeralProcessPhase::Succeeded,
            Some(completed),
            false,
            0,
            false,
            None,
            ZoneRevision::new(1),
        )
        .unwrap();
        assert_eq!(
            controller
                .reconcile(observation.clone(), &timestamp("2026-08-01T00:00:01.000Z"))
                .unwrap(),
            EphemeralCleanupDecision::UpdateStatus {
                cleanup_eligible_at: timestamp("2026-08-01T01:00:00.000Z"),
                incident_held: false,
                expected_revision: ZoneRevision::new(1),
            }
        );
        let eligible = Some(timestamp("2026-08-01T01:00:00.000Z"));
        let waiting = EphemeralProcessObservation::with_defaults(
            key(),
            EphemeralProcessPhase::Succeeded,
            Some(timestamp("2026-08-01T00:00:00.000Z")),
            false,
            0,
            false,
            eligible.clone(),
            ZoneRevision::new(2),
        )
        .unwrap();
        assert_eq!(
            controller
                .reconcile(waiting.clone(), &timestamp("2026-08-01T00:59:59.000Z"))
                .unwrap(),
            EphemeralCleanupDecision::RequeueAt {
                at: eligible.clone().unwrap(),
                expected_revision: ZoneRevision::new(2),
            }
        );
        assert_eq!(
            controller
                .reconcile(waiting, &timestamp("2026-08-01T01:00:00.000Z"))
                .unwrap(),
            EphemeralCleanupDecision::Delete {
                expected_revision: ZoneRevision::new(2)
            }
        );
    }

    #[test]
    fn mismatched_cleanup_eligibility_is_repaired_before_delete() {
        let mut controller = EphemeralProcessCleanupController::default();
        let observation = EphemeralProcessObservation::with_defaults(
            key(),
            EphemeralProcessPhase::Succeeded,
            Some(timestamp("2026-08-01T00:00:00.000Z")),
            false,
            0,
            false,
            Some(timestamp("2026-08-01T00:00:00.000Z")),
            ZoneRevision::new(6),
        )
        .unwrap();
        assert_eq!(
            controller
                .reconcile(observation, &timestamp("2026-08-01T02:00:00.000Z"))
                .unwrap(),
            EphemeralCleanupDecision::UpdateStatus {
                cleanup_eligible_at: timestamp("2026-08-01T01:00:00.000Z"),
                incident_held: false,
                expected_revision: ZoneRevision::new(6),
            }
        );

        let valid = EphemeralProcessObservation::with_defaults(
            key(),
            EphemeralProcessPhase::Succeeded,
            Some(timestamp("2026-08-01T00:00:00.000Z")),
            false,
            0,
            false,
            Some(timestamp("2026-08-01T01:00:00.000Z")),
            ZoneRevision::new(7),
        )
        .unwrap();
        assert_eq!(
            controller
                .reconcile(valid, &timestamp("2026-08-01T01:00:00.000Z"))
                .unwrap(),
            EphemeralCleanupDecision::Delete {
                expected_revision: ZoneRevision::new(7),
            }
        );
    }

    #[test]
    fn incident_hold_finalizer_and_unknown_outcome_never_issue_delete() {
        let completed = Some(timestamp("2026-08-01T00:00:00.000Z"));
        let mut controller = EphemeralProcessCleanupController::default();
        let held = EphemeralProcessObservation::with_defaults(
            key(),
            EphemeralProcessPhase::Succeeded,
            completed.clone(),
            true,
            0,
            false,
            Some(timestamp("2026-08-01T01:00:00.000Z")),
            ZoneRevision::new(1),
        )
        .unwrap();
        assert!(matches!(
            controller
                .reconcile(held, &timestamp("2026-08-01T02:00:00.000Z"))
                .unwrap(),
            EphemeralCleanupDecision::Blocked {
                reason: EphemeralCleanupBlock::IncidentHold,
                ..
            }
        ));
        let finalizer = EphemeralProcessObservation::with_defaults(
            key(),
            EphemeralProcessPhase::Failed,
            completed,
            false,
            1,
            false,
            Some(timestamp("2026-08-02T00:00:00.000Z")),
            ZoneRevision::new(2),
        )
        .unwrap();
        let mut finalizer_controller = EphemeralProcessCleanupController::default();
        assert!(matches!(
            finalizer_controller
                .reconcile(finalizer, &timestamp("2026-08-01T02:00:00.000Z"))
                .unwrap(),
            EphemeralCleanupDecision::Blocked {
                reason: EphemeralCleanupBlock::Finalizer,
                ..
            }
        ));
        let unknown = EphemeralProcessObservation::with_defaults(
            key(),
            EphemeralProcessPhase::Unknown,
            None,
            false,
            0,
            false,
            None,
            ZoneRevision::new(3),
        )
        .unwrap();
        assert!(matches!(
            controller
                .reconcile(unknown, &timestamp("2026-08-01T02:00:00.000Z"))
                .unwrap(),
            EphemeralCleanupDecision::Blocked {
                reason: EphemeralCleanupBlock::UnknownOutcome,
                ..
            }
        ));
    }

    #[test]
    fn restart_restores_status_projection_and_deleted_watch_removes_it() {
        let observation = EphemeralProcessObservation::with_defaults(
            key(),
            EphemeralProcessPhase::Succeeded,
            Some(timestamp("2026-08-01T00:00:00.000Z")),
            false,
            0,
            false,
            Some(timestamp("2026-08-01T01:00:00.000Z")),
            ZoneRevision::new(4),
        )
        .unwrap();
        let mut controller = EphemeralProcessCleanupController::default();
        controller
            .reconcile(observation, &timestamp("2026-08-01T00:30:00.000Z"))
            .unwrap();
        let mut restarted = EphemeralProcessCleanupController::restore(controller.snapshot());
        assert!(restarted.record(&key()).is_some());
        restarted
            .observe_deleted(&key(), ZoneRevision::new(4))
            .unwrap();
        assert!(restarted.record(&key()).is_none());
    }

    #[test]
    fn timestamp_arithmetic_handles_month_boundaries() {
        let observation = EphemeralProcessObservation::with_defaults(
            key(),
            EphemeralProcessPhase::Succeeded,
            Some(timestamp("2026-01-31T23:30:00.000Z")),
            false,
            0,
            false,
            None,
            ZoneRevision::new(5),
        )
        .unwrap();
        let mut controller = EphemeralProcessCleanupController::default();
        let decision = controller
            .reconcile(observation, &timestamp("2026-01-31T23:31:00.000Z"))
            .unwrap();
        assert!(matches!(
            decision,
            EphemeralCleanupDecision::UpdateStatus {
                cleanup_eligible_at,
                ..
            } if cleanup_eligible_at.as_str() == "2026-02-01T00:30:00.000Z"
        ));
    }
}
