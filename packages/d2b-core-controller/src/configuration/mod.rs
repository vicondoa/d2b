//! Zone configuration ownership, activation, and cleanup (`ADR046-routing-013`).
//!
//! This module owns the "Configuration ownership and cleanup contract" of
//! `ADR-046-zone-routing`: it turns one integrity-verified per-Zone
//! `resource-bundle.json` into a durable generation record, a bounded set of
//! Create/UpdateSpec/Delete intents, and the cleanup state machine that
//! completes a Delete.
//!
//! Like [`crate::zone_links`], the module is a planner, not a runner. It opens
//! no file, holds no descriptor, performs no store transaction, and names no
//! host path, socket, or store path. The bundle is addressed only by its
//! content-derived [`ResourceBundleGenerationId`] - the D101
//! `d2b:v3:resource-bundle` digest that D119 freezes as the generation
//! identity - and every durable side effect is expressed as a typed effect the
//! caller performs.
//!
//! Four invariants are structural rather than advisory:
//!
//! * **Commit before effect.** [`ConfigurationService::begin_activation`]
//!   releases nothing: it returns an [`ActivationPlan`] that names the bundle
//!   the caller must durably stage and the [`GenerationRecord`] the caller must
//!   durably commit. [`ConfigurationService::commit_activation`] is the sole
//!   issuer of a [`GenerationCommitProof`], and effects - every queued intent,
//!   every prune, and the reconcile notification - are obtainable only by
//!   consuming that proof. This is the D122 ordering: the `generation.json`
//!   commit is the sole activation point and precedes all intent queuing and
//!   reconcile notification, so an aborted pass, a stale proof, or a restart
//!   mid-pass cannot release an effect.
//! * **Single flight.** At most one activation pass may be open, and at most
//!   one cleanup pass per resource; a second attempt fails closed with
//!   [`ConfigurationError::ActivationInFlight`] or
//!   [`ConfigurationError::CleanupInFlight`].
//! * **Ownership boundary.** A diff never deletes a resource whose
//!   `managedBy` is not the configuration service's value, whose
//!   `configurationGeneration` is absent, or whose `configurationGeneration`
//!   does not match the serving generation; an attempt to *seize* a
//!   foreign-managed resource named by the bundle fails closed with no
//!   mutation.
//! * **Recover before cleanup.** [`ConfigurationService::restore`] resolves the
//!   recorded generation to adopted or quarantined before any planning, and a
//!   prune is refused for a generation that is still a rollback target or the
//!   source of an in-flight Delete (ADR 0034 continuation semantics).
//!
//! The audit append for a completed deletion is deliberately *not* part of the
//! store transaction: it is released only from the committed cleanup proof and
//! deduplicated by committed revision, so recovery replay appends exactly once.
//!
//! This is a directory module so that the work items extending it each own a
//! distinct file. [`bundle_apply`] and [`generation_transition`] are empty
//! scaffolds; every item landed so far lives here in `mod.rs`, unmoved, and
//! `mod.rs` stays the single re-export point for the module's public surface.

pub mod bundle_apply;
pub mod generation_transition;

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::{
    ZoneBundle,
    v3::identity::{
        ConfigurationGeneration, ResourceBundleGenerationId, ResourceName, ResourceTypeName,
        ResourceUid, SchemaFingerprint, Timestamp, ZoneId, ZoneRevision,
    },
};

use crate::{
    audit::{AuditError, AuditEvent, AuditLedger, AuditReason},
    cleanup::PendingCleanupCondition,
};

/// Default number of retained prior generation bundles (D119).
pub const RETAINED_GENERATIONS_DEFAULT: u8 = 3;

/// Lowest permitted `retainedGenerations` value.
pub const RETAINED_GENERATIONS_MIN: u8 = 1;

/// Highest permitted `retainedGenerations` value.
pub const RETAINED_GENERATIONS_MAX: u8 = 16;

/// Closed fail-closed reason for every configuration refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConfigurationError {
    /// The bundle names a Zone other than the one this service serves.
    ZoneMismatch,
    /// The bundle declares the same `(type, name)` twice.
    DuplicateResource,
    /// The same canonical spec field was supplied twice.
    DuplicateSpecField,
    /// Another activation pass is already open for this Zone.
    ActivationInFlight,
    /// The presented commit proof does not match the committed pass.
    StaleCommitProof,
    /// A bundle resource is already owned by a different management agent.
    ManagedByCollision,
    /// A configuration-managed resource carries no `configurationGeneration`.
    ConfigurationGenerationMissing,
    /// The rollback target is not present in the retention ring.
    RollbackTargetUnavailable,
    /// The requested `retainedGenerations` is outside its frozen range.
    RetentionOutOfRange,
    /// The activation ordinal cannot advance.
    GenerationOrdinalExhausted,
    /// No Delete intent is outstanding for this resource.
    CleanupNotRequested,
    /// Another cleanup pass is already open for this resource.
    CleanupInFlight,
    /// The cleanup audit record for this revision was already appended.
    AuditAlreadyAppended,
    /// The committed cleanup pass released no audit record.
    CleanupNotCompleted,
    /// No Create or UpdateSpec intent is outstanding for this resource.
    IntentNotOutstanding,
}

impl ConfigurationError {
    /// Return the closed kebab-case reason label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ZoneMismatch => "bundle-zone-mismatch",
            Self::DuplicateResource => "bundle-duplicate-resource",
            Self::DuplicateSpecField => "spec-duplicate-field",
            Self::ActivationInFlight => "activation-in-flight",
            Self::StaleCommitProof => "stale-commit-proof",
            Self::ManagedByCollision => "resource-managed-by-collision",
            Self::ConfigurationGenerationMissing => "configuration-generation-missing",
            Self::RollbackTargetUnavailable => "rollback-target-unavailable",
            Self::RetentionOutOfRange => "retention-out-of-range",
            Self::GenerationOrdinalExhausted => "generation-ordinal-exhausted",
            Self::CleanupNotRequested => "cleanup-not-requested",
            Self::CleanupInFlight => "cleanup-in-flight",
            Self::AuditAlreadyAppended => "cleanup-audit-already-appended",
            Self::CleanupNotCompleted => "cleanup-not-completed",
            Self::IntentNotOutstanding => "intent-not-outstanding",
        }
    }
}

impl core::fmt::Display for ConfigurationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for ConfigurationError {}

/// The management agent that owns a resource's lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ManagementAgent {
    /// The configuration service; the only agent whose resources a diff may
    /// delete.
    Configuration,
    /// A controller-created resource; never seized or deleted by a diff.
    Controller,
    /// An API-created resource; never seized or deleted by a diff.
    Api,
}

impl ManagementAgent {
    /// Return the closed label for this agent.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Controller => "controller",
            Self::Api => "api",
        }
    }
}

/// D088 phase of the active generation resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GenerationPhase {
    /// One or more Create or UpdateSpec intents are in flight.
    Pending,
    /// Creates and updates are done; cleanup is still pending.
    Degraded,
    /// No intent is outstanding and no cleanup is pending.
    Ready,
    /// A Delete intent exhausted its retries.
    Failed,
}

impl GenerationPhase {
    /// Return the closed label for this phase.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Degraded => "Degraded",
            Self::Ready => "Ready",
            Self::Failed => "Failed",
        }
    }
}

/// Whether the recorded active generation was adopted at restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActivationStatus {
    /// No generation has ever been activated for this Zone.
    Uninitialized,
    /// The recorded active generation was adopted.
    Active,
    /// The recorded active generation was ambiguous; the prior pointer serves.
    Quarantined,
}

impl ActivationStatus {
    /// Return the closed label for this status.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Uninitialized => "uninitialized",
            Self::Active => "active",
            Self::Quarantined => "quarantined",
        }
    }
}

/// Integrity outcome for the staged bundle named by a recorded generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StagedBundleIntegrity {
    /// The staged bundle is present and its digest matches the record.
    Verified,
    /// The staged bundle is absent.
    Missing,
    /// The staged bundle is present but its digest does not match.
    Mismatched,
}

/// Closed audit event kind emitted under the `zone-config` category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConfigurationAuditKind {
    /// A new bundle was processed.
    GenerationActivate,
    /// A rollback to a retained generation was initiated.
    GenerationRollback,
    /// A resource deletion committed.
    ResourceCleanup,
    /// A Delete intent exhausted its retries.
    ResourceCleanupFailed,
}

impl ConfigurationAuditKind {
    /// Return the closed kebab-case audit event kind.
    pub const fn label(self) -> &'static str {
        match self {
            Self::GenerationActivate => "zone-generation-activate",
            Self::GenerationRollback => "zone-generation-rollback",
            Self::ResourceCleanup => "zone-resource-cleanup",
            Self::ResourceCleanupFailed => "zone-resource-cleanup-failed",
        }
    }
}

/// The bounded retention count for prior generation bundles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RetainedGenerations(u8);

impl RetainedGenerations {
    /// Validate one `retainedGenerations` value against its frozen range.
    pub const fn new(value: u8) -> Result<Self, ConfigurationError> {
        if value < RETAINED_GENERATIONS_MIN || value > RETAINED_GENERATIONS_MAX {
            return Err(ConfigurationError::RetentionOutOfRange);
        }
        Ok(Self(value))
    }

    /// Return the frozen default.
    pub const fn default_value() -> Self {
        Self(RETAINED_GENERATIONS_DEFAULT)
    }

    /// Return the numeric count.
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// A `(ResourceType, name)` identity within one Zone.
///
/// Both components already carry redacted `Debug`, so a key cannot leak a
/// resource name into a log, span, or metric.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceKey {
    type_name: ResourceTypeName,
    name: ResourceName,
}

impl ResourceKey {
    /// Build one resource key.
    pub const fn new(type_name: ResourceTypeName, name: ResourceName) -> Self {
        Self { type_name, name }
    }

    /// Borrow the ResourceType name.
    pub const fn type_name(&self) -> &ResourceTypeName {
        &self.type_name
    }

    /// Borrow the resource name.
    pub const fn name(&self) -> &ResourceName {
        &self.name
    }
}

/// The canonical form of one resource `spec`.
///
/// Construction sorts the supplied field projection by key and rejects a
/// duplicate key, so two renderings of the same spec compare equal regardless
/// of the order the Nix emitter produced. The values are the already
/// canonicalized leaf projection supplied by the bundle reader; this type does
/// not itself encode JSON.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalSpec {
    fields: Vec<(String, String)>,
}

impl CanonicalSpec {
    /// Build a canonical spec from an unordered field projection.
    pub fn from_fields<I, K, V>(fields: I) -> Result<Self, ConfigurationError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut collected: Vec<(String, String)> = fields
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        collected.sort();
        if collected.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(ConfigurationError::DuplicateSpecField);
        }
        Ok(Self { fields: collected })
    }

    /// Return the number of canonical fields.
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }
}

impl core::fmt::Debug for CanonicalSpec {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("CanonicalSpec(<redacted>)")
    }
}

/// One resource as declared by a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleResource {
    key: ResourceKey,
    spec: CanonicalSpec,
}

impl BundleResource {
    /// Build one bundle resource.
    pub const fn new(key: ResourceKey, spec: CanonicalSpec) -> Self {
        Self { key, spec }
    }

    /// Borrow the resource key.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }
}

/// One integrity-verified per-Zone resource bundle.
///
/// The bundle is addressed only by its content-derived generation identity; no
/// path is carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceBundle {
    zone: ZoneId,
    generation_id: ResourceBundleGenerationId,
    resources: Vec<BundleResource>,
}

impl ResourceBundle {
    /// Build one bundle, rejecting a duplicate `(type, name)`.
    pub fn new(
        zone: ZoneId,
        generation_id: ResourceBundleGenerationId,
        resources: Vec<BundleResource>,
    ) -> Result<Self, ConfigurationError> {
        let mut seen = BTreeSet::new();
        for resource in &resources {
            if !seen.insert(resource.key.clone()) {
                return Err(ConfigurationError::DuplicateResource);
            }
        }
        Ok(Self {
            zone,
            generation_id,
            resources,
        })
    }

    /// Borrow the Zone this bundle configures.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the generation identity of this bundle.
    pub const fn generation_id(&self) -> &ResourceBundleGenerationId {
        &self.generation_id
    }

    /// Borrow the declared resources.
    pub fn resources(&self) -> &[BundleResource] {
        &self.resources
    }
}

/// One resource observed in the runtime store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredResource {
    key: ResourceKey,
    managed_by: ManagementAgent,
    configuration_generation: Option<ResourceBundleGenerationId>,
    spec: CanonicalSpec,
}

impl StoredResource {
    /// Build one observed store row.
    pub const fn new(
        key: ResourceKey,
        managed_by: ManagementAgent,
        configuration_generation: Option<ResourceBundleGenerationId>,
        spec: CanonicalSpec,
    ) -> Self {
        Self {
            key,
            managed_by,
            configuration_generation,
            spec,
        }
    }

    /// Borrow the resource key.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Return the management agent that owns this resource.
    pub const fn managed_by(&self) -> ManagementAgent {
        self.managed_by
    }
}

/// The durable per-Zone generation record (`generation.json`).
///
/// `ADR046-routing-013` is its sole durable writer. The record carries the
/// active generation identity, the activation ordinal, the prior pointer, the
/// retention bound, and the retention-ring membership, all committed together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationRecord {
    active_generation_id: ResourceBundleGenerationId,
    active_ordinal: ConfigurationGeneration,
    prior_generation_id: Option<ResourceBundleGenerationId>,
    retained_generations: RetainedGenerations,
    retention_ring: Vec<ResourceBundleGenerationId>,
}

impl GenerationRecord {
    /// Build one record, rejecting a duplicated ring member.
    pub fn new(
        active_generation_id: ResourceBundleGenerationId,
        active_ordinal: ConfigurationGeneration,
        prior_generation_id: Option<ResourceBundleGenerationId>,
        retained_generations: RetainedGenerations,
        retention_ring: Vec<ResourceBundleGenerationId>,
    ) -> Result<Self, ConfigurationError> {
        let unique: BTreeSet<&ResourceBundleGenerationId> = retention_ring.iter().collect();
        if unique.len() != retention_ring.len() {
            return Err(ConfigurationError::DuplicateResource);
        }
        Ok(Self {
            active_generation_id,
            active_ordinal,
            prior_generation_id,
            retained_generations,
            retention_ring,
        })
    }

    /// Borrow the active generation identity.
    pub const fn active_generation_id(&self) -> &ResourceBundleGenerationId {
        &self.active_generation_id
    }

    /// Return the activation ordinal.
    pub const fn active_ordinal(&self) -> ConfigurationGeneration {
        self.active_ordinal
    }

    /// Borrow the prior generation pointer, if any.
    pub const fn prior_generation_id(&self) -> Option<&ResourceBundleGenerationId> {
        self.prior_generation_id.as_ref()
    }

    /// Return the retention bound.
    pub const fn retained_generations(&self) -> RetainedGenerations {
        self.retained_generations
    }

    /// Borrow the retention ring, oldest first.
    pub fn retention_ring(&self) -> &[ResourceBundleGenerationId] {
        &self.retention_ring
    }
}

/// One queued configuration intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationIntent {
    /// Create the resource and set `configurationGeneration` and `managedBy`.
    Create(ResourceKey),
    /// Replace the resource spec and refresh `configurationGeneration`.
    UpdateSpec(ResourceKey),
    /// Asynchronously delete a resource omitted from the new bundle.
    Delete(ResourceKey),
    /// Cancel a not-yet-executed Delete for a revived resource.
    CancelDelete(ResourceKey),
}

impl ConfigurationIntent {
    /// Borrow the resource this intent targets.
    pub const fn key(&self) -> &ResourceKey {
        match self {
            Self::Create(key)
            | Self::UpdateSpec(key)
            | Self::Delete(key)
            | Self::CancelDelete(key) => key,
        }
    }

    /// Return the closed label for this intent.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Create(_) => "create",
            Self::UpdateSpec(_) => "update-spec",
            Self::Delete(_) => "delete",
            Self::CancelDelete(_) => "cancel-delete",
        }
    }
}

/// One post-commit configuration effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationEffect {
    /// Hand one intent to the reconcile queue.
    QueueIntent(ConfigurationIntent),
    /// Prune one retained prior bundle from the ring.
    PrunePriorBundle(ResourceBundleGenerationId),
    /// Append one audit record under `zone-config`.
    AppendAudit(ConfigurationAuditKind),
    /// Notify the reconcile loops; always released last.
    NotifyReconcile,
}

/// An open activation pass holding a planned, unreleased generation switch.
///
/// The pass releases nothing and is neither `Clone` nor `Copy`, so a plan
/// cannot be committed twice.
#[derive(Debug)]
pub struct ActivationPlan {
    sequence: u64,
    audit_kind: ConfigurationAuditKind,
    stage_generation: Option<ResourceBundleGenerationId>,
    next_record: GenerationRecord,
    intents: Vec<ConfigurationIntent>,
    unchanged: Vec<ResourceKey>,
    prunes: Vec<ResourceBundleGenerationId>,
}

impl ActivationPlan {
    /// Borrow the generation whose bundle must be durably staged in the
    /// retention ring before the record is committed.
    pub const fn stage_generation(&self) -> Option<&ResourceBundleGenerationId> {
        self.stage_generation.as_ref()
    }

    /// Borrow the record the caller must durably commit as the sole
    /// activation point.
    pub const fn next_record(&self) -> &GenerationRecord {
        &self.next_record
    }

    /// Borrow the intents this pass would queue once committed.
    pub fn planned_intents(&self) -> &[ConfigurationIntent] {
        &self.intents
    }

    /// Borrow the resources whose spec is unchanged by this generation.
    pub fn unchanged(&self) -> &[ResourceKey] {
        &self.unchanged
    }

    /// Borrow the prior bundles this pass would prune once committed.
    pub fn planned_prunes(&self) -> &[ResourceBundleGenerationId] {
        &self.prunes
    }

    /// Return the audit kind this activation records.
    pub const fn audit_kind(&self) -> ConfigurationAuditKind {
        self.audit_kind
    }
}

/// The outcome of diffing a bundle against the serving generation.
#[derive(Debug)]
pub enum ActivationOutcome {
    /// The bundle carries the serving generation identity; no action required.
    Unchanged,
    /// A planned, uncommitted activation.
    Planned(ActivationPlan),
}

/// Durable-commit evidence for exactly one committed activation.
///
/// The type has no public constructor and no `Clone`. It is issued only by
/// [`ConfigurationService::commit_activation`] and consumed by value in
/// [`ConfigurationService::release_activation_effects`].
#[derive(Debug)]
pub struct GenerationCommitProof {
    sequence: u64,
}

/// One cleanup step for a pending-delete resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CleanupStep {
    /// Registered finalizers must release first.
    DrainFinalizers,
    /// Live controller-created children must acknowledge teardown first.
    CascadeChildren,
    /// Every precondition holds; the store transaction may commit.
    CommitDeletion,
}

impl CleanupStep {
    /// Return the closed label for this step.
    pub const fn label(self) -> &'static str {
        match self {
            Self::DrainFinalizers => "drain-finalizers",
            Self::CascadeChildren => "cascade-children",
            Self::CommitDeletion => "commit-deletion",
        }
    }
}

/// The runtime observation a cleanup pass is planned against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupObservation {
    pending_finalizers: u32,
    live_controller_children: u32,
}

impl CleanupObservation {
    /// Build one cleanup observation.
    pub const fn new(pending_finalizers: u32, live_controller_children: u32) -> Self {
        Self {
            pending_finalizers,
            live_controller_children,
        }
    }
}

/// One post-commit cleanup effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupEffect {
    /// Notify every finalizer holder to release.
    NotifyFinalizerHolders,
    /// Notify the owning controller to tear down its children.
    NotifyControllerCascade,
    /// Append the authoritative cleanup audit for the committed revision.
    ///
    /// Released only after the store transaction that removed the row and its
    /// index entries has committed.
    AppendCleanupAudit(ZoneRevision),
}

/// An open cleanup pass for exactly one resource.
#[derive(Debug)]
pub struct CleanupPass {
    sequence: u64,
    key: ResourceKey,
    step: CleanupStep,
}

impl CleanupPass {
    /// Borrow the resource this pass targets.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Return the step this pass would perform.
    pub const fn step(&self) -> CleanupStep {
        self.step
    }
}

/// Durable-commit evidence for exactly one committed cleanup pass.
#[derive(Debug)]
pub struct CleanupCommitProof {
    sequence: u64,
}

/// Per-resource cleanup tracking for a pending-delete resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupTracking {
    deletion_requested_at: Timestamp,
    cleanup_config_generation: ResourceBundleGenerationId,
    cleanup_error: Option<ConfigurationError>,
    cleanup_attempt: u32,
}

impl CleanupTracking {
    /// Borrow the instant core queued the Delete intent.
    pub const fn deletion_requested_at(&self) -> &Timestamp {
        &self.deletion_requested_at
    }

    /// Borrow the generation that triggered the Delete intent.
    pub const fn cleanup_config_generation(&self) -> &ResourceBundleGenerationId {
        &self.cleanup_config_generation
    }

    /// Return the last Delete failure, if any.
    pub const fn cleanup_error(&self) -> Option<ConfigurationError> {
        self.cleanup_error
    }

    /// Return the attempt count.
    pub const fn cleanup_attempt(&self) -> u32 {
        self.cleanup_attempt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanMode {
    Activate,
    Rollback,
}

/// The per-Zone configuration service.
pub struct ConfigurationService {
    zone: ZoneId,
    status: ActivationStatus,
    record: Option<GenerationRecord>,
    quarantined: Option<ResourceBundleGenerationId>,
    retained_generations: RetainedGenerations,
    sequence: u64,
    activation_open: bool,
    pending_effects: Option<(u64, Vec<ConfigurationEffect>)>,
    outstanding_intents: BTreeSet<ResourceKey>,
    cleanup: BTreeMap<ResourceKey, CleanupTracking>,
    cleanup_open: BTreeSet<ResourceKey>,
    cleanup_sequence: u64,
    pending_cleanup_effects: Option<(u64, ResourceKey, CleanupStep, Vec<CleanupEffect>)>,
    appended_audits: BTreeSet<u64>,
    failed: bool,
}

impl ConfigurationService {
    /// Build a service for a Zone that has never activated a generation.
    pub fn empty(zone: ZoneId, retained_generations: RetainedGenerations) -> Self {
        Self {
            zone,
            status: ActivationStatus::Uninitialized,
            record: None,
            quarantined: None,
            retained_generations,
            sequence: 0,
            activation_open: false,
            pending_effects: None,
            outstanding_intents: BTreeSet::new(),
            cleanup: BTreeMap::new(),
            cleanup_open: BTreeSet::new(),
            cleanup_sequence: 0,
            pending_cleanup_effects: None,
            appended_audits: BTreeSet::new(),
            failed: false,
        }
    }

    /// Recover from the durable generation record at startup.
    ///
    /// A normal restart is a continuation event: the recorded active
    /// generation is adopted when its staged bundle integrity-verifies, and is
    /// otherwise quarantined while service continues from the recorded prior
    /// pointer. Recovery resolves before any planning, so no prune can precede
    /// adoption or quarantine.
    pub fn restore(zone: ZoneId, record: GenerationRecord, staged: StagedBundleIntegrity) -> Self {
        let retained_generations = record.retained_generations;
        let (status, quarantined) = match staged {
            StagedBundleIntegrity::Verified => (ActivationStatus::Active, None),
            StagedBundleIntegrity::Missing | StagedBundleIntegrity::Mismatched => (
                ActivationStatus::Quarantined,
                Some(record.active_generation_id.clone()),
            ),
        };
        Self {
            zone,
            status,
            record: Some(record),
            quarantined,
            retained_generations,
            sequence: 0,
            activation_open: false,
            pending_effects: None,
            outstanding_intents: BTreeSet::new(),
            cleanup: BTreeMap::new(),
            cleanup_open: BTreeSet::new(),
            cleanup_sequence: 0,
            pending_cleanup_effects: None,
            appended_audits: BTreeSet::new(),
            failed: false,
        }
    }

    /// Borrow the Zone this service configures.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Return the recovery status of the recorded generation.
    pub const fn status(&self) -> ActivationStatus {
        self.status
    }

    /// Borrow the durable generation record, if one exists.
    pub const fn record(&self) -> Option<&GenerationRecord> {
        self.record.as_ref()
    }

    /// Borrow the generation currently being served.
    ///
    /// This is the recorded active generation once adopted, and the recorded
    /// prior pointer while the active generation is quarantined.
    pub fn serving_generation(&self) -> Option<&ResourceBundleGenerationId> {
        let record = self.record.as_ref()?;
        match self.status {
            ActivationStatus::Active => Some(&record.active_generation_id),
            ActivationStatus::Quarantined => record.prior_generation_id.as_ref(),
            ActivationStatus::Uninitialized => None,
        }
    }

    /// Borrow the quarantined generation, if recovery quarantined one.
    pub const fn quarantined_generation(&self) -> Option<&ResourceBundleGenerationId> {
        self.quarantined.as_ref()
    }

    /// Return the D088 phase of the active generation resource.
    pub fn phase(&self) -> GenerationPhase {
        if self.failed {
            GenerationPhase::Failed
        } else if !self.outstanding_intents.is_empty() {
            GenerationPhase::Pending
        } else if !self.cleanup.is_empty() {
            GenerationPhase::Degraded
        } else {
            GenerationPhase::Ready
        }
    }

    /// Return the pending-cleanup resources, in canonical key order.
    pub fn pending_cleanup(&self) -> Vec<&ResourceKey> {
        self.cleanup.keys().collect()
    }

    /// Borrow the cleanup tracking for one pending-delete resource.
    pub fn cleanup_tracking(&self, key: &ResourceKey) -> Option<&CleanupTracking> {
        self.cleanup.get(key)
    }

    /// Plan the activation of a newly installed bundle.
    ///
    /// Returns [`ActivationOutcome::Unchanged`] when the bundle carries the
    /// serving generation identity. The returned plan releases nothing.
    pub fn begin_activation(
        &mut self,
        bundle: &ResourceBundle,
        store: &[StoredResource],
        now: &Timestamp,
    ) -> Result<ActivationOutcome, ConfigurationError> {
        self.plan(bundle, store, now, PlanMode::Activate)
    }

    /// Plan a rollback onto a retained prior bundle.
    ///
    /// A rollback is restaged as a new generation rather than trusting a
    /// rewritten input file, so it re-runs the same commit ordering.
    pub fn begin_rollback(
        &mut self,
        retained: &ResourceBundle,
        store: &[StoredResource],
        now: &Timestamp,
    ) -> Result<ActivationOutcome, ConfigurationError> {
        let retained_in_ring = self
            .record
            .as_ref()
            .is_some_and(|record| record.retention_ring.contains(&retained.generation_id));
        if !retained_in_ring {
            return Err(ConfigurationError::RollbackTargetUnavailable);
        }
        self.plan(retained, store, now, PlanMode::Rollback)
    }

    /// Discard an open activation pass without durable mutation or effect.
    pub fn abort_activation(&mut self, plan: ActivationPlan) {
        debug_assert_eq!(plan.sequence, self.sequence + 1);
        self.activation_open = false;
    }

    /// Record the durable `generation.json` commit and issue its proof.
    ///
    /// This is the sole activation point. `deletionRequestedAt` is stamped on
    /// every pending-delete resource here, and a revived resource has its
    /// pending-delete tracking cleared here, both before any effect is
    /// obtainable.
    pub fn commit_activation(
        &mut self,
        plan: ActivationPlan,
        now: &Timestamp,
    ) -> Result<GenerationCommitProof, ConfigurationError> {
        if !self.activation_open || plan.sequence != self.sequence + 1 {
            return Err(ConfigurationError::StaleCommitProof);
        }
        self.sequence = plan.sequence;
        self.activation_open = false;

        let generation_id = plan.next_record.active_generation_id.clone();
        // The Delete intent originates from the generation that owned the
        // resource, which is the generation this activation supersedes. The
        // retention ring pins that bundle until the Delete resolves.
        let superseded = plan
            .next_record
            .prior_generation_id
            .clone()
            .unwrap_or_else(|| generation_id.clone());
        let mut effects = Vec::with_capacity(plan.intents.len() + plan.prunes.len() + 2);
        for intent in &plan.intents {
            match intent {
                ConfigurationIntent::Create(key) | ConfigurationIntent::UpdateSpec(key) => {
                    self.outstanding_intents.insert(key.clone());
                }
                ConfigurationIntent::Delete(key) => {
                    self.cleanup.insert(
                        key.clone(),
                        CleanupTracking {
                            deletion_requested_at: now.clone(),
                            cleanup_config_generation: superseded.clone(),
                            cleanup_error: None,
                            cleanup_attempt: 0,
                        },
                    );
                }
                ConfigurationIntent::CancelDelete(key) => {
                    self.cleanup.remove(key);
                    self.cleanup_open.remove(key);
                }
            }
            effects.push(ConfigurationEffect::QueueIntent(intent.clone()));
        }
        for pruned in &plan.prunes {
            effects.push(ConfigurationEffect::PrunePriorBundle(pruned.clone()));
        }
        effects.push(ConfigurationEffect::AppendAudit(plan.audit_kind));
        effects.push(ConfigurationEffect::NotifyReconcile);

        self.record = Some(plan.next_record);
        self.status = ActivationStatus::Active;
        self.quarantined = None;
        self.retained_generations = self
            .record
            .as_ref()
            .map_or(self.retained_generations, |record| {
                record.retained_generations
            });
        self.pending_effects = Some((plan.sequence, effects));
        Ok(GenerationCommitProof {
            sequence: plan.sequence,
        })
    }

    /// Consume one activation proof and release its effects exactly once.
    ///
    /// The reconcile notification is always the last effect, so a caller that
    /// applies the slice in order cannot notify before the queued intents
    /// exist, and cannot reach either before the durable commit returned.
    pub fn release_activation_effects(
        &mut self,
        proof: GenerationCommitProof,
    ) -> Result<Vec<ConfigurationEffect>, ConfigurationError> {
        match self.pending_effects.take() {
            Some((sequence, effects)) if sequence == proof.sequence => Ok(effects),
            Some(pending) => {
                self.pending_effects = Some(pending);
                Err(ConfigurationError::StaleCommitProof)
            }
            None => Err(ConfigurationError::StaleCommitProof),
        }
    }

    /// Mark one Create or UpdateSpec intent complete.
    pub fn complete_intent(&mut self, key: &ResourceKey) -> Result<(), ConfigurationError> {
        if self.outstanding_intents.remove(key) {
            Ok(())
        } else {
            Err(ConfigurationError::IntentNotOutstanding)
        }
    }

    /// Plan the next cleanup step for one pending-delete resource.
    pub fn begin_cleanup(
        &mut self,
        key: &ResourceKey,
        observed: CleanupObservation,
    ) -> Result<CleanupPass, ConfigurationError> {
        if !self.cleanup.contains_key(key) {
            return Err(ConfigurationError::CleanupNotRequested);
        }
        if self.cleanup_open.contains(key) {
            return Err(ConfigurationError::CleanupInFlight);
        }
        let step = if observed.pending_finalizers > 0 {
            CleanupStep::DrainFinalizers
        } else if observed.live_controller_children > 0 {
            CleanupStep::CascadeChildren
        } else {
            CleanupStep::CommitDeletion
        };
        self.cleanup_sequence += 1;
        self.cleanup_open.insert(key.clone());
        Ok(CleanupPass {
            sequence: self.cleanup_sequence,
            key: key.clone(),
            step,
        })
    }

    /// Discard an open cleanup pass without durable mutation or effect.
    pub fn abort_cleanup(&mut self, pass: CleanupPass) {
        self.cleanup_open.remove(&pass.key);
    }

    /// Record the durable outcome of one cleanup step and issue its proof.
    ///
    /// For [`CleanupStep::CommitDeletion`] the caller has committed the single
    /// store transaction that wrote the `Deleted` revision and removed the row
    /// and every index entry; the authoritative audit record is released from
    /// the proof afterwards and is never part of that transaction.
    pub fn commit_cleanup(
        &mut self,
        pass: CleanupPass,
        revision: ZoneRevision,
    ) -> Result<CleanupCommitProof, ConfigurationError> {
        if !self.cleanup_open.contains(&pass.key) {
            return Err(ConfigurationError::StaleCommitProof);
        }
        let Some(tracking) = self.cleanup.get_mut(&pass.key) else {
            return Err(ConfigurationError::CleanupNotRequested);
        };
        tracking.cleanup_attempt = tracking.cleanup_attempt.saturating_add(1);
        tracking.cleanup_error = None;
        self.cleanup_open.remove(&pass.key);

        let effects = match pass.step {
            CleanupStep::DrainFinalizers => vec![CleanupEffect::NotifyFinalizerHolders],
            CleanupStep::CascadeChildren => vec![CleanupEffect::NotifyControllerCascade],
            CleanupStep::CommitDeletion => {
                self.cleanup.remove(&pass.key);
                vec![CleanupEffect::AppendCleanupAudit(revision)]
            }
        };
        self.pending_cleanup_effects = Some((pass.sequence, pass.key, pass.step, effects));
        Ok(CleanupCommitProof {
            sequence: pass.sequence,
        })
    }

    /// Consume one cleanup proof and release its effects exactly once.
    pub fn release_cleanup_effects(
        &mut self,
        proof: CleanupCommitProof,
    ) -> Result<Vec<CleanupEffect>, ConfigurationError> {
        match self.pending_cleanup_effects.take() {
            Some((sequence, _, _, effects)) if sequence == proof.sequence => Ok(effects),
            Some(pending) => {
                self.pending_cleanup_effects = Some(pending);
                Err(ConfigurationError::StaleCommitProof)
            }
            None => Err(ConfigurationError::StaleCommitProof),
        }
    }

    /// Record that the cleanup audit for one committed revision was appended.
    ///
    /// Recovery may replay a released audit effect after a restart; the second
    /// attempt fails closed with [`ConfigurationError::AuditAlreadyAppended`],
    /// which is the exactly-once property.
    pub fn record_cleanup_audit_appended(
        &mut self,
        revision: ZoneRevision,
    ) -> Result<ConfigurationAuditKind, ConfigurationError> {
        if !self.appended_audits.insert(revision.get()) {
            return Err(ConfigurationError::AuditAlreadyAppended);
        }
        Ok(ConfigurationAuditKind::ResourceCleanup)
    }

    /// Record a permanent Delete failure for one resource.
    ///
    /// The generation moves to [`GenerationPhase::Failed`] and the prior bundle
    /// stays pinned, because the Delete intent from its generation is still
    /// unresolved.
    pub fn record_cleanup_failure(
        &mut self,
        key: &ResourceKey,
        reason: ConfigurationError,
    ) -> Result<ConfigurationAuditKind, ConfigurationError> {
        let Some(tracking) = self.cleanup.get_mut(key) else {
            return Err(ConfigurationError::CleanupNotRequested);
        };
        tracking.cleanup_error = Some(reason);
        tracking.cleanup_attempt = tracking.cleanup_attempt.saturating_add(1);
        self.cleanup_open.remove(key);
        self.failed = true;
        Ok(ConfigurationAuditKind::ResourceCleanupFailed)
    }

    fn plan(
        &mut self,
        bundle: &ResourceBundle,
        store: &[StoredResource],
        _now: &Timestamp,
        mode: PlanMode,
    ) -> Result<ActivationOutcome, ConfigurationError> {
        if bundle.zone != self.zone {
            return Err(ConfigurationError::ZoneMismatch);
        }
        if self.activation_open {
            return Err(ConfigurationError::ActivationInFlight);
        }
        if self.serving_generation() == Some(&bundle.generation_id) {
            return Ok(ActivationOutcome::Unchanged);
        }

        let serving = self.serving_generation().cloned();
        let observed: BTreeMap<&ResourceKey, &StoredResource> =
            store.iter().map(|row| (&row.key, row)).collect();
        let declared: BTreeSet<&ResourceKey> =
            bundle.resources.iter().map(BundleResource::key).collect();

        let mut intents = Vec::new();
        let mut unchanged = Vec::new();

        for resource in &bundle.resources {
            match observed.get(&resource.key) {
                None => intents.push(ConfigurationIntent::Create(resource.key.clone())),
                Some(row) => {
                    // Collision guard: a bundle may never seize a resource
                    // owned by a controller or by the API.
                    if row.managed_by != ManagementAgent::Configuration {
                        return Err(ConfigurationError::ManagedByCollision);
                    }
                    if row.configuration_generation.is_none() {
                        return Err(ConfigurationError::ConfigurationGenerationMissing);
                    }
                    if row.spec == resource.spec {
                        unchanged.push(resource.key.clone());
                    } else {
                        intents.push(ConfigurationIntent::UpdateSpec(resource.key.clone()));
                    }
                }
            }
        }

        for row in store {
            if declared.contains(&row.key) {
                continue;
            }
            // A diff deletes only its own resources, and only those stamped
            // with the generation it is superseding.
            if row.managed_by != ManagementAgent::Configuration {
                continue;
            }
            let Some(generation) = row.configuration_generation.as_ref() else {
                continue;
            };
            if Some(generation) != serving.as_ref() {
                continue;
            }
            intents.push(ConfigurationIntent::Delete(row.key.clone()));
        }

        if mode == PlanMode::Rollback {
            for resource in &bundle.resources {
                if self.cleanup.contains_key(&resource.key) {
                    intents.push(ConfigurationIntent::CancelDelete(resource.key.clone()));
                }
            }
        }

        let next_ordinal = match self.record.as_ref() {
            Some(record) => record
                .active_ordinal
                .checked_next()
                .ok_or(ConfigurationError::GenerationOrdinalExhausted)?,
            None => ConfigurationGeneration::new(1)
                .map_err(|_| ConfigurationError::GenerationOrdinalExhausted)?,
        };

        let mut ring: Vec<ResourceBundleGenerationId> = self
            .record
            .as_ref()
            .map_or_else(Vec::new, |record| record.retention_ring.clone());
        let stage_generation = serving.clone().filter(|id| !ring.contains(id));
        if let Some(staged) = stage_generation.clone() {
            ring.push(staged);
        }

        let prunes = self.plan_prunes(&mut ring, serving.as_ref());

        let next_record = GenerationRecord::new(
            bundle.generation_id.clone(),
            next_ordinal,
            serving,
            self.retained_generations,
            ring,
        )?;

        self.activation_open = true;
        Ok(ActivationOutcome::Planned(ActivationPlan {
            sequence: self.sequence + 1,
            audit_kind: match mode {
                PlanMode::Activate => ConfigurationAuditKind::GenerationActivate,
                PlanMode::Rollback => ConfigurationAuditKind::GenerationRollback,
            },
            stage_generation,
            next_record,
            intents,
            unchanged,
            prunes,
        }))
    }

    /// Select prunable ring members, oldest first.
    ///
    /// A generation is never pruned while it is the incoming prior pointer, is
    /// quarantined, or is the source generation of an in-flight Delete, so a
    /// rollback target and a pending cleanup always keep their bundle.
    fn plan_prunes(
        &self,
        ring: &mut Vec<ResourceBundleGenerationId>,
        incoming_prior: Option<&ResourceBundleGenerationId>,
    ) -> Vec<ResourceBundleGenerationId> {
        let cap = usize::from(self.retained_generations.get());
        let mut prunes = Vec::new();
        let mut index = 0;
        while ring.len() > cap && index < ring.len() {
            let candidate = &ring[index];
            let protected = Some(candidate) == incoming_prior
                || Some(candidate) == self.quarantined.as_ref()
                || self
                    .cleanup
                    .values()
                    .any(|tracking| &tracking.cleanup_config_generation == candidate);
            if protected {
                index += 1;
                continue;
            }
            prunes.push(ring.remove(index));
        }
        prunes
    }
}

impl core::fmt::Debug for ConfigurationService {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ConfigurationService")
            .field("status", &self.status.label())
            .field("phase", &self.phase().label())
            .field("outstanding_intents", &self.outstanding_intents.len())
            .field("pending_cleanup", &self.cleanup.len())
            .finish()
    }
}

/// Closed failure from Phase 3 bundle activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationError {
    /// The candidate bundle belongs to another Zone.
    ZoneMismatch,
    /// A subsequent activation supplied a different Zone uid.
    ZoneUidMismatch,
    /// The private artifact-catalog anchor does not match the installed one.
    ArtifactCatalogMismatch,
    /// A bundled Provider schema does not match the installed Provider.
    ProviderSchemaMismatch,
    /// The bundle passed no integrity proof and cannot be activated.
    BundleIntegrityFailed,
    /// The underlying post-commit planner refused the transition.
    Configuration(ConfigurationError),
    /// The bundle adapter could not represent a canonical desired resource.
    BundleApply(bundle_apply::BundleApplyError),
    /// The audit sink rejected an event.
    Audit(AuditError),
}

impl ActivationError {
    /// Return the stable fail-closed label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ZoneMismatch => "config-zone-mismatch",
            Self::ZoneUidMismatch => "config-zone-uid-mismatch",
            Self::ArtifactCatalogMismatch => "config-catalog-mismatch",
            Self::ProviderSchemaMismatch => "provider-schema-mismatch",
            Self::BundleIntegrityFailed => "config-bundle-integrity-failed",
            Self::Configuration(error) => error.label(),
            Self::BundleApply(error) => error.label(),
            Self::Audit(error) => error.label(),
        }
    }
}

impl core::fmt::Display for ActivationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for ActivationError {}

impl From<ConfigurationError> for ActivationError {
    fn from(error: ConfigurationError) -> Self {
        Self::Configuration(error)
    }
}

impl From<bundle_apply::BundleApplyError> for ActivationError {
    fn from(error: bundle_apply::BundleApplyError) -> Self {
        Self::BundleApply(error)
    }
}

impl From<AuditError> for ActivationError {
    fn from(error: AuditError) -> Self {
        Self::Audit(error)
    }
}

/// Candidate bundle plus the private integrity facts supplied by the
/// configuration publication adapter.
#[derive(Clone)]
pub struct BundleActivation {
    bundle: ZoneBundle,
    zone_uid: Option<ResourceUid>,
    artifact_catalog_digest: Option<SchemaFingerprint>,
    installed_provider_schema_digests: BTreeMap<String, SchemaFingerprint>,
}

impl BundleActivation {
    /// Wrap one already-decoded bundle without adding an optional catalog
    /// expectation. The controller still verifies the bundle's own content
    /// digest because `ZoneBundle::from_json` and `ZoneBundle::build` do so.
    pub const fn new(bundle: ZoneBundle) -> Self {
        Self {
            bundle,
            zone_uid: None,
            artifact_catalog_digest: None,
            installed_provider_schema_digests: BTreeMap::new(),
        }
    }

    /// Alias for callers that make the verification boundary explicit.
    pub const fn from_verified_bundle(bundle: ZoneBundle) -> Self {
        Self::new(bundle)
    }

    /// Bind the candidate to the Zone uid read from store metadata.
    pub fn with_zone_uid(mut self, zone_uid: Option<ResourceUid>) -> Self {
        self.zone_uid = zone_uid;
        self
    }

    /// Require the private artifact catalog to have this digest.
    pub fn with_artifact_catalog_digest(mut self, digest: SchemaFingerprint) -> Self {
        self.artifact_catalog_digest = Some(digest);
        self
    }

    /// Supply the installed Provider schema digest projection.
    pub fn with_installed_provider_schema_digests(
        mut self,
        digests: BTreeMap<String, SchemaFingerprint>,
    ) -> Self {
        self.installed_provider_schema_digests = digests;
        self
    }

    /// Borrow the candidate bundle.
    pub const fn bundle(&self) -> &ZoneBundle {
        &self.bundle
    }

    /// Borrow the candidate Zone uid, if the bundle envelope carried one.
    pub const fn zone_uid(&self) -> Option<&ResourceUid> {
        self.zone_uid.as_ref()
    }
}

impl core::fmt::Debug for BundleActivation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BundleActivation")
            .field("has_zone_uid", &self.zone_uid.is_some())
            .field(
                "has_catalog_digest",
                &self.artifact_catalog_digest.is_some(),
            )
            .field(
                "provider_schema_count",
                &self.installed_provider_schema_digests.len(),
            )
            .finish_non_exhaustive()
    }
}

/// Classification of one `(type, name)` generation diff entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiffKind {
    /// The resource is absent from the observed store.
    New,
    /// The resource exists but its canonical desired spec changed.
    Changed,
    /// The resource exists with an equivalent canonical desired spec.
    Unchanged,
    /// A configuration-owned resource is absent from the candidate bundle.
    Removed,
    /// A foreign owner already holds the desired identity.
    Collision,
}

impl DiffKind {
    /// Return the stable diff label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Changed => "changed",
            Self::Unchanged => "unchanged",
            Self::Removed => "removed",
            Self::Collision => "collision",
        }
    }
}

/// One redacted generation diff entry.
#[derive(Clone, PartialEq, Eq)]
pub struct DiffEntry {
    key: ResourceKey,
    kind: DiffKind,
}

impl DiffEntry {
    /// Borrow the exact identity for an authorized mutation.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Return the classification.
    pub const fn kind(&self) -> DiffKind {
        self.kind
    }
}

impl core::fmt::Debug for DiffEntry {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DiffEntry")
            .field("kind", &self.kind)
            .field("has_key", &true)
            .finish()
    }
}

/// Deterministic generation diff, sorted by `(ResourceType, name)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationDiff {
    entries: Vec<DiffEntry>,
}

impl GenerationDiff {
    /// Compute a diff without mutating the service or the store.
    pub fn compute(bundle: &ZoneBundle, store: &[StoredResource]) -> Self {
        let observed: BTreeMap<&ResourceKey, &StoredResource> =
            store.iter().map(|row| (&row.key, row)).collect();
        let declared: BTreeSet<ResourceKey> = bundle
            .resources()
            .iter()
            .map(|resource| {
                ResourceKey::new(
                    resource.resource_type().clone(),
                    resource.metadata().name().clone(),
                )
            })
            .collect();
        let mut entries = Vec::new();
        for resource in bundle.resources() {
            let key = ResourceKey::new(
                resource.resource_type().clone(),
                resource.metadata().name().clone(),
            );
            let kind = match observed.get(&key) {
                None => DiffKind::New,
                Some(row) if row.managed_by != ManagementAgent::Configuration => {
                    DiffKind::Collision
                }
                Some(row) => {
                    let canonical = CanonicalSpec::from_fields([(
                        "spec",
                        String::from_utf8(resource.spec().to_canonical_bytes()).unwrap_or_default(),
                    )]);
                    match canonical {
                        Ok(spec) if spec == row.spec => DiffKind::Unchanged,
                        _ => DiffKind::Changed,
                    }
                }
            };
            entries.push(DiffEntry { key, kind });
        }
        for row in store {
            if row.managed_by == ManagementAgent::Configuration && !declared.contains(&row.key) {
                entries.push(DiffEntry {
                    key: row.key.clone(),
                    kind: DiffKind::Removed,
                });
            }
        }
        entries.sort_by(|left, right| left.key.cmp(&right.key));
        Self { entries }
    }

    /// Borrow all entries in canonical order.
    pub fn entries(&self) -> &[DiffEntry] {
        &self.entries
    }

    /// Return only entries of one kind.
    pub fn by_kind(&self, kind: DiffKind) -> Vec<&DiffEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .collect()
    }

    /// Return whether the diff has no effective change.
    pub fn is_empty(&self) -> bool {
        self.entries
            .iter()
            .all(|entry| entry.kind == DiffKind::Unchanged)
    }
}

/// Pending cleanup status projected into Zone status.
#[derive(Clone, PartialEq, Eq)]
pub struct PendingCleanup {
    key: ResourceKey,
    requested_at: Timestamp,
    prior_generation: ConfigurationGeneration,
    active_generation: ConfigurationGeneration,
    phase: CleanupPhase,
}

impl PendingCleanup {
    /// Construct one configuration-owned cleanup record.
    pub fn new(
        key: ResourceKey,
        requested_at: Timestamp,
        prior_generation: ConfigurationGeneration,
        active_generation: ConfigurationGeneration,
    ) -> Self {
        Self {
            key,
            requested_at,
            prior_generation,
            active_generation,
            phase: CleanupPhase::Pending,
        }
    }

    /// Borrow the pending resource identity.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Borrow the deletion request timestamp.
    pub const fn requested_at(&self) -> &Timestamp {
        &self.requested_at
    }

    /// Return the source generation.
    pub const fn prior_generation(&self) -> ConfigurationGeneration {
        self.prior_generation
    }

    /// Return the active generation.
    pub const fn active_generation(&self) -> ConfigurationGeneration {
        self.active_generation
    }

    /// Return the current cleanup phase.
    pub const fn phase(&self) -> CleanupPhase {
        self.phase
    }
}

impl core::fmt::Debug for PendingCleanup {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PendingCleanup")
            .field("has_key", &true)
            .field("phase", &self.phase)
            .finish()
    }
}

/// Cleanup lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CleanupPhase {
    /// Delete was requested and finalizers or children may still run.
    Pending,
    /// A Provider finalizer is blocking deletion.
    FinalizerBlocked,
    /// An owner controller's children are blocking deletion.
    OwnerChildBlocked,
    /// The configured stuck threshold elapsed; no force removal occurred.
    Stalled,
    /// The store committed a Deleted revision event.
    Deleted,
}

impl CleanupPhase {
    /// Return the stable phase label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending-cleanup",
            Self::FinalizerBlocked => "finalizer-blocked",
            Self::OwnerChildBlocked => "owner-child-blocked",
            Self::Stalled => "cleanup-stalled",
            Self::Deleted => "deleted",
        }
    }
}

/// Result of one cleanup observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CleanupOutcome {
    /// The normal Delete request is still in progress.
    Pending,
    /// A finalizer or child is blocking, without force clearing it.
    Blocked,
    /// The cleanup threshold was exceeded and the Zone stays Degraded.
    Stalled,
    /// The resource was atomically removed.
    Deleted,
}

impl CleanupOutcome {
    /// Return the stable outcome label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Blocked => "blocked",
            Self::Stalled => "stalled",
            Self::Deleted => "deleted",
        }
    }
}

/// Count-bounded prior-generation retention policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    retained_generations: RetainedGenerations,
}

impl RetentionPolicy {
    /// Validate and create a count-based retention policy.
    pub const fn new(retained_generations: u8) -> Result<Self, ConfigurationError> {
        match RetainedGenerations::new(retained_generations) {
            Ok(retained_generations) => Ok(Self {
                retained_generations,
            }),
            Err(error) => Err(error),
        }
    }

    /// Return the default count-based policy.
    pub const fn default_value() -> Self {
        Self {
            retained_generations: RetainedGenerations::default_value(),
        }
    }

    /// Return the configured prior-generation count.
    pub const fn retained_generations(self) -> RetainedGenerations {
        self.retained_generations
    }
}

/// Redacted retention-ring status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionState {
    active: Option<ResourceBundleGenerationId>,
    prior: Option<ResourceBundleGenerationId>,
    retained: Vec<ResourceBundleGenerationId>,
    prunable: Vec<ResourceBundleGenerationId>,
}

impl RetentionState {
    /// Borrow the active generation identity.
    pub const fn active(&self) -> Option<&ResourceBundleGenerationId> {
        self.active.as_ref()
    }

    /// Borrow the prior generation pointer.
    pub const fn prior(&self) -> Option<&ResourceBundleGenerationId> {
        self.prior.as_ref()
    }

    /// Borrow retained generations oldest first.
    pub fn retained(&self) -> &[ResourceBundleGenerationId] {
        &self.retained
    }

    /// Borrow generations selected for release after cleanup completion.
    pub fn prunable(&self) -> &[ResourceBundleGenerationId] {
        &self.prunable
    }
}

/// Active generation and Zone status projection owned by one controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationState {
    zone: ZoneId,
    zone_uid: Option<ResourceUid>,
    active_content_hash: Option<ResourceBundleGenerationId>,
    active_generation: Option<ConfigurationGeneration>,
    phase: GenerationPhase,
    pending_cleanup_count: u32,
    cleanup_failed: bool,
    last_activation_error: Option<ActivationError>,
}

impl GenerationState {
    /// Construct an uninitialized generation state.
    pub fn new(zone: ZoneId) -> Self {
        Self {
            zone,
            zone_uid: None,
            active_content_hash: None,
            active_generation: None,
            phase: GenerationPhase::Ready,
            pending_cleanup_count: 0,
            cleanup_failed: false,
            last_activation_error: None,
        }
    }

    /// Borrow the Zone identity.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the adopted Zone uid.
    pub const fn zone_uid(&self) -> Option<&ResourceUid> {
        self.zone_uid.as_ref()
    }

    /// Borrow the active content hash.
    pub const fn active_content_hash(&self) -> Option<&ResourceBundleGenerationId> {
        self.active_content_hash.as_ref()
    }

    /// Return the active runtime ordinal.
    pub const fn active_generation(&self) -> Option<ConfigurationGeneration> {
        self.active_generation
    }

    /// Return the projected Zone phase.
    pub const fn phase(&self) -> GenerationPhase {
        self.phase
    }

    /// Return the exact pending cleanup count.
    pub const fn pending_cleanup_count(&self) -> u32 {
        self.pending_cleanup_count
    }

    /// Whether the cleanup-failed condition is set.
    pub const fn cleanup_failed(&self) -> bool {
        self.cleanup_failed
    }

    /// Return the last activation refusal, if one was recorded.
    pub const fn last_activation_error(&self) -> Option<ActivationError> {
        self.last_activation_error
    }

    /// Return the `PendingCleanup` condition projection.
    pub fn pending_cleanup_condition(&self) -> PendingCleanupCondition {
        PendingCleanupCondition::from_count(
            usize::try_from(self.pending_cleanup_count).unwrap_or(usize::MAX),
        )
    }
}

/// Result returned after a candidate has passed validation and committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationResult {
    no_op: bool,
    generation: Option<ConfigurationGeneration>,
    diff: GenerationDiff,
    effects: Vec<bundle_apply::BundleApplyEffect>,
    state: GenerationState,
    audits: Vec<AuditEvent>,
}

impl ActivationResult {
    /// Whether the candidate content hash was already active.
    pub const fn is_noop(&self) -> bool {
        self.no_op
    }

    /// Return the newly committed ordinal, if this was not a no-op.
    pub const fn generation(&self) -> Option<ConfigurationGeneration> {
        self.generation
    }

    /// Borrow the deterministic diff.
    pub const fn diff(&self) -> &GenerationDiff {
        &self.diff
    }

    /// Borrow post-commit effects in dispatch order.
    pub fn effects(&self) -> &[bundle_apply::BundleApplyEffect] {
        &self.effects
    }

    /// Borrow the projected Zone state.
    pub const fn state(&self) -> &GenerationState {
        &self.state
    }

    /// Borrow audit events generated by this activation.
    pub fn audits(&self) -> &[AuditEvent] {
        &self.audits
    }
}

/// Per-Zone configuration publication controller.
///
/// The controller is intentionally store- and transport-agnostic. A caller
/// supplies the verified bundle, the current store snapshot, and the durable
/// commit clock; the returned effects are the only mutations the caller may
/// dispatch.
pub struct ZoneConfigController {
    zone: ZoneId,
    service: ConfigurationService,
    state: GenerationState,
    retention_policy: RetentionPolicy,
    artifact_catalog_digest: Option<SchemaFingerprint>,
    installed_provider_schema_digests: BTreeMap<String, SchemaFingerprint>,
    active_bundles: BTreeMap<ResourceBundleGenerationId, ZoneBundle>,
    pending_cleanup: BTreeMap<ResourceKey, PendingCleanup>,
    prunable_generations: Vec<ResourceBundleGenerationId>,
    audit: AuditLedger,
}

impl ZoneConfigController {
    /// Create an uninitialized controller for one Zone.
    pub fn new(zone: ZoneId, retained_generations: RetainedGenerations) -> Self {
        let retention_policy = RetentionPolicy {
            retained_generations,
        };
        Self {
            zone: zone.clone(),
            service: ConfigurationService::empty(zone.clone(), retained_generations),
            state: GenerationState::new(zone.clone()),
            retention_policy,
            artifact_catalog_digest: None,
            installed_provider_schema_digests: BTreeMap::new(),
            active_bundles: BTreeMap::new(),
            pending_cleanup: BTreeMap::new(),
            prunable_generations: Vec::new(),
            audit: AuditLedger::new(zone),
        }
    }

    /// Create a controller with the frozen default retention count.
    pub fn with_defaults(zone: ZoneId) -> Self {
        Self::new(zone, RetainedGenerations::default_value())
    }

    /// Restore a controller from the durable generation record.
    pub fn restore(zone: ZoneId, record: GenerationRecord, staged: StagedBundleIntegrity) -> Self {
        let retained = record.retained_generations();
        let mut controller = Self::new(zone.clone(), retained);
        controller.service = ConfigurationService::restore(zone.clone(), record, staged);
        controller.sync_state(None);
        controller
    }

    /// Configure the installed private artifact-catalog digest.
    pub fn set_artifact_catalog_digest(&mut self, digest: SchemaFingerprint) {
        self.artifact_catalog_digest = Some(digest);
    }

    /// Configure installed Provider schema fingerprints.
    pub fn set_installed_provider_schema_digests(
        &mut self,
        digests: BTreeMap<String, SchemaFingerprint>,
    ) {
        self.installed_provider_schema_digests = digests;
    }

    /// Borrow the Zone served by this controller.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the current generation state.
    pub const fn state(&self) -> &GenerationState {
        &self.state
    }

    /// Borrow the lower-level commit-before-effects service.
    pub const fn service(&self) -> &ConfigurationService {
        &self.service
    }

    /// Borrow pending configuration cleanup records in canonical key order.
    pub fn pending_cleanup(&self) -> Vec<&PendingCleanup> {
        self.pending_cleanup.values().collect()
    }

    /// Borrow the retained bundle for an authorized rollback caller.
    pub fn retained_bundle(&self, generation: &ResourceBundleGenerationId) -> Option<&ZoneBundle> {
        self.active_bundles.get(generation)
    }

    /// Borrow the per-Zone append-only audit ledger.
    pub const fn audit(&self) -> &AuditLedger {
        &self.audit
    }

    /// Borrow the count-based retention policy.
    pub const fn retention_policy(&self) -> RetentionPolicy {
        self.retention_policy
    }

    /// Return the current count-based retention projection.
    pub fn retention_state(&self) -> RetentionState {
        let Some(record) = self.service.record() else {
            return RetentionState {
                active: None,
                prior: None,
                retained: Vec::new(),
                prunable: self.prunable_generations.clone(),
            };
        };
        RetentionState {
            active: Some(record.active_generation_id.clone()),
            prior: record.prior_generation_id.clone(),
            retained: record.retention_ring.clone(),
            prunable: self.prunable_generations.clone(),
        }
    }

    /// Validate and activate one candidate bundle.
    ///
    /// Validation occurs before opening the lower-level activation pass. A
    /// rejection therefore leaves the active record, resource snapshot, and
    /// pending-cleanup set unchanged while still producing a redacted audit
    /// event.
    pub fn activate(
        &mut self,
        candidate: BundleActivation,
        stored: &[StoredResource],
        now: &Timestamp,
    ) -> Result<ActivationResult, ActivationError> {
        self.activate_mode(candidate, stored, now, false)
    }

    /// Re-activate one retained bundle through the same integrity and
    /// post-commit path, but with rollback audit semantics.
    pub fn rollback(
        &mut self,
        generation: &ResourceBundleGenerationId,
        stored: &[StoredResource],
        now: &Timestamp,
    ) -> Result<ActivationResult, ActivationError> {
        let record = self.service.record().ok_or(ActivationError::Configuration(
            ConfigurationError::RollbackTargetUnavailable,
        ))?;
        if !record.retention_ring().contains(generation) {
            return Err(ActivationError::Configuration(
                ConfigurationError::RollbackTargetUnavailable,
            ));
        }
        let bundle =
            self.active_bundles
                .get(generation)
                .cloned()
                .ok_or(ActivationError::Configuration(
                    ConfigurationError::RollbackTargetUnavailable,
                ))?;
        self.activate_mode(
            BundleActivation::new(bundle).with_zone_uid(self.state.zone_uid.clone()),
            stored,
            now,
            true,
        )
    }

    fn activate_mode(
        &mut self,
        candidate: BundleActivation,
        stored: &[StoredResource],
        now: &Timestamp,
        rollback: bool,
    ) -> Result<ActivationResult, ActivationError> {
        let bundle = candidate.bundle();
        if bundle.zone() != &self.zone {
            return self.reject(
                ActivationError::ZoneMismatch,
                AuditReason::BundleIntegrityFailed,
                bundle,
                now,
            );
        }
        if let Some(expected) = self.artifact_catalog_digest.as_ref()
            && bundle.artifact_catalog_digest() != expected
        {
            return self.reject(
                ActivationError::ArtifactCatalogMismatch,
                AuditReason::CatalogMismatch,
                bundle,
                now,
            );
        }
        if let Some(expected) = candidate.artifact_catalog_digest.as_ref()
            && bundle.artifact_catalog_digest() != expected
        {
            return self.reject(
                ActivationError::ArtifactCatalogMismatch,
                AuditReason::CatalogMismatch,
                bundle,
                now,
            );
        }
        let installed = if candidate.installed_provider_schema_digests.is_empty() {
            &self.installed_provider_schema_digests
        } else {
            &candidate.installed_provider_schema_digests
        };
        if bundle.verify_provider_schema_digests(installed).is_err() {
            return self.reject(
                ActivationError::ProviderSchemaMismatch,
                AuditReason::ProviderSchemaMismatch,
                bundle,
                now,
            );
        }
        if let Some(current_uid) = self.state.zone_uid()
            && candidate.zone_uid.as_ref() != Some(current_uid)
        {
            return self.reject(
                ActivationError::ZoneUidMismatch,
                AuditReason::BundleIntegrityFailed,
                bundle,
                now,
            );
        }

        let diff = GenerationDiff::compute(bundle, stored);
        let outcome = if rollback {
            bundle_apply::begin_bundle_rollback(&mut self.service, bundle, stored, now)?
        } else {
            bundle_apply::begin_bundle_apply(&mut self.service, bundle, stored, now)?
        };
        let plan = match outcome {
            bundle_apply::BundleApplyOutcome::Unchanged { .. } => {
                return Ok(ActivationResult {
                    no_op: true,
                    generation: self.state.active_generation,
                    diff,
                    effects: Vec::new(),
                    state: self.state.clone(),
                    audits: Vec::new(),
                });
            }
            bundle_apply::BundleApplyOutcome::Planned(plan) => plan,
        };
        let next_generation = plan.activation().next_record().active_ordinal();
        let prior_ordinal = next_generation
            .get()
            .checked_sub(1)
            .and_then(|value| ConfigurationGeneration::new(value).ok());
        let prior_generation = plan
            .activation()
            .next_record()
            .prior_generation_id()
            .cloned();
        let activation_kind = plan.activation().audit_kind();
        let proof = bundle_apply::commit_bundle_apply(&mut self.service, plan, now)?;
        let effects = bundle_apply::release_bundle_apply_effects(&mut self.service, proof)?;
        self.prunable_generations = effects
            .iter()
            .filter_map(|effect| match effect {
                bundle_apply::BundleApplyEffect::PrunePriorBundle(generation) => {
                    Some(generation.clone())
                }
                _ => None,
            })
            .collect();

        if self.state.zone_uid.is_none() {
            self.state.zone_uid = candidate.zone_uid.clone();
        }
        self.state.last_activation_error = None;
        self.active_bundles
            .insert(bundle.content_hash().clone(), bundle.clone());
        for effect in &effects {
            if let bundle_apply::BundleApplyEffect::DeleteResource { key, .. } = effect
                && let Some(prior_ordinal) = prior_ordinal
            {
                self.pending_cleanup.insert(
                    key.clone(),
                    PendingCleanup::new(key.clone(), now.clone(), prior_ordinal, next_generation),
                );
            }
            if let bundle_apply::BundleApplyEffect::CancelDelete(key) = effect {
                self.pending_cleanup.remove(key);
            }
        }
        let mut audits = Vec::new();
        let activation_audit = match activation_kind {
            ConfigurationAuditKind::GenerationRollback => AuditEvent::generation_rolled_back(
                self.zone.clone(),
                bundle.content_hash().clone(),
                next_generation,
                now.clone(),
            ),
            _ => AuditEvent::generation_activated(
                self.zone.clone(),
                bundle.content_hash().clone(),
                next_generation,
                now.clone(),
            ),
        };
        self.audit.append(activation_audit.clone())?;
        audits.push(activation_audit);
        for effect in &effects {
            match effect {
                bundle_apply::BundleApplyEffect::DeleteResource { key, .. } => {
                    if let Some(prior_ordinal) = prior_ordinal {
                        let event = AuditEvent::resource_deletion_requested(
                            self.zone.clone(),
                            key.type_name().clone(),
                            key.name(),
                            prior_ordinal,
                            next_generation,
                            now.clone(),
                        );
                        self.audit.append(event.clone())?;
                        audits.push(event);
                    }
                }
                bundle_apply::BundleApplyEffect::AppendNameConflictAudit(conflict) => {
                    let event = AuditEvent::configuration_collision(
                        self.zone.clone(),
                        conflict.key().type_name().clone(),
                        conflict.key().name(),
                        next_generation,
                        now.clone(),
                    );
                    self.audit.append(event.clone())?;
                    audits.push(event);
                }
                _ => {}
            }
        }
        self.sync_state(prior_generation);
        Ok(ActivationResult {
            no_op: false,
            generation: Some(next_generation),
            diff,
            effects,
            state: self.state.clone(),
            audits,
        })
    }

    /// Mark one post-commit Create or UpdateSpec intent complete.
    pub fn complete_intent(&mut self, key: &ResourceKey) -> Result<(), ActivationError> {
        self.service.complete_intent(key)?;
        self.sync_state(None);
        Ok(())
    }

    /// Consume a `Deleted` watch event after the store transaction committed.
    pub fn observe_deleted(
        &mut self,
        key: &ResourceKey,
        revision: ZoneRevision,
        now: &Timestamp,
    ) -> Result<CleanupOutcome, ActivationError> {
        let pending = self
            .pending_cleanup
            .get_mut(key)
            .ok_or(ActivationError::Configuration(
                ConfigurationError::CleanupNotRequested,
            ))?;
        let pass = self
            .service
            .begin_cleanup(key, CleanupObservation::new(0, 0))?;
        let proof = self.service.commit_cleanup(pass, revision)?;
        let effects = self.service.release_cleanup_effects(proof)?;
        if !effects.iter().any(|effect| {
            matches!(
                effect,
                CleanupEffect::AppendCleanupAudit(candidate) if *candidate == revision
            )
        }) {
            return Err(ActivationError::Configuration(
                ConfigurationError::CleanupNotCompleted,
            ));
        }
        self.service.record_cleanup_audit_appended(revision)?;
        let event = AuditEvent::resource_deleted(
            self.zone.clone(),
            key.type_name().clone(),
            crate::audit::resource_name_digest(key.name()),
            pending.prior_generation,
            pending.active_generation,
            revision,
            now.clone(),
        );
        self.audit.append(event)?;
        pending.phase = CleanupPhase::Deleted;
        self.pending_cleanup.remove(key);
        if self.pending_cleanup.is_empty() {
            self.state.cleanup_failed = false;
        }
        self.sync_state(None);
        Ok(CleanupOutcome::Deleted)
    }

    /// Mark cleanup stalled without force-removing a finalizer or child.
    pub fn mark_cleanup_stalled(
        &mut self,
        key: &ResourceKey,
        reason: AuditReason,
        now: &Timestamp,
    ) -> Result<CleanupOutcome, ActivationError> {
        let pending = self
            .pending_cleanup
            .get_mut(key)
            .ok_or(ActivationError::Configuration(
                ConfigurationError::CleanupNotRequested,
            ))?;
        pending.phase = match reason {
            AuditReason::OwnerChildBlocked => CleanupPhase::OwnerChildBlocked,
            AuditReason::FinalizerBlocked => CleanupPhase::FinalizerBlocked,
            _ => CleanupPhase::Stalled,
        };
        let generation = pending.active_generation;
        let event = AuditEvent::cleanup_stalled(
            self.zone.clone(),
            key.type_name().clone(),
            crate::audit::resource_name_digest(key.name()),
            generation,
            reason,
            now.clone(),
        );
        self.audit.append(event)?;
        self.state.cleanup_failed = true;
        self.sync_state(None);
        Ok(CleanupOutcome::Stalled)
    }

    fn reject(
        &mut self,
        error: ActivationError,
        reason: AuditReason,
        bundle: &ZoneBundle,
        now: &Timestamp,
    ) -> Result<ActivationResult, ActivationError> {
        let event = AuditEvent::generation_rejected_for_bundle(
            self.zone.clone(),
            reason,
            Some(bundle.content_hash().clone()),
            now.clone(),
        );
        // A repeated recovery attempt for the same content and reason is
        // exactly-once; the rejection itself remains a failure.
        if !matches!(
            self.audit.append(event),
            Ok(()) | Err(AuditError::AlreadyAppended)
        ) {
            return Err(error);
        }
        self.state.last_activation_error = Some(error);
        Err(error)
    }

    fn sync_state(&mut self, _prior_generation: Option<ResourceBundleGenerationId>) {
        let record = self.service.record();
        self.state.active_content_hash = record.map(|record| record.active_generation_id.clone());
        self.state.active_generation = record.map(|record| record.active_ordinal);
        self.state.pending_cleanup_count =
            u32::try_from(self.pending_cleanup.len()).unwrap_or(u32::MAX);
        self.state.phase = if !self.pending_cleanup.is_empty() {
            GenerationPhase::Degraded
        } else {
            self.service.phase()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-07-22T21:00:00.000Z";

    fn digest(byte: u8) -> ResourceBundleGenerationId {
        let hex = format!("{byte:02x}").repeat(32);
        ResourceBundleGenerationId::parse(format!("sha256:{hex}")).expect("valid digest")
    }

    fn zone() -> ZoneId {
        ZoneId::parse("work").expect("valid zone")
    }

    fn now() -> Timestamp {
        Timestamp::parse(NOW).expect("valid timestamp")
    }

    fn key(name: &str) -> ResourceKey {
        ResourceKey::new(
            ResourceTypeName::parse("ZoneLink").expect("valid type"),
            ResourceName::parse(name).expect("valid name"),
        )
    }

    fn spec(value: &str) -> CanonicalSpec {
        CanonicalSpec::from_fields([("mode", value), ("peer", "k1")]).expect("valid spec")
    }

    fn bundle(generation: u8, names: &[&str]) -> ResourceBundle {
        ResourceBundle::new(
            zone(),
            digest(generation),
            names
                .iter()
                .map(|name| BundleResource::new(key(name), spec("active")))
                .collect(),
        )
        .expect("valid bundle")
    }

    fn owned(name: &str, generation: u8) -> StoredResource {
        StoredResource::new(
            key(name),
            ManagementAgent::Configuration,
            Some(digest(generation)),
            spec("active"),
        )
    }

    fn service_at(generation: u8) -> ConfigurationService {
        let record = GenerationRecord::new(
            digest(generation),
            ConfigurationGeneration::new(1).expect("nonzero"),
            None,
            RetainedGenerations::default_value(),
            Vec::new(),
        )
        .expect("valid record");
        ConfigurationService::restore(zone(), record, StagedBundleIntegrity::Verified)
    }

    fn activation_err(result: Result<ActivationOutcome, ConfigurationError>) -> ConfigurationError {
        match result {
            Err(error) => error,
            Ok(_) => panic!("expected a typed refusal"),
        }
    }

    fn cleanup_err(result: Result<CleanupPass, ConfigurationError>) -> ConfigurationError {
        match result {
            Err(error) => error,
            Ok(_) => panic!("expected a typed refusal"),
        }
    }

    fn planned(outcome: ActivationOutcome) -> ActivationPlan {
        match outcome {
            ActivationOutcome::Planned(plan) => plan,
            ActivationOutcome::Unchanged => panic!("expected a planned activation"),
        }
    }

    #[test]
    fn generation_identity_is_the_deterministic_bundle_content_digest() {
        // D119 freezes the generation identity as the content-addressed
        // `d2b:v3:resource-bundle` digest, so equal content is equal identity.
        let first = bundle(1, &["a", "b"]);
        let second = bundle(1, &["a", "b"]);
        assert_eq!(first.generation_id(), second.generation_id());
        assert_ne!(
            first.generation_id(),
            bundle(2, &["a", "b"]).generation_id()
        );
        assert_eq!(
            ResourceBundleGenerationId::domain_tag(),
            "d2b:v3:resource-bundle"
        );
    }

    #[test]
    fn same_generation_id_is_a_no_op() {
        let mut service = service_at(1);
        let outcome = service
            .begin_activation(&bundle(1, &["a"]), &[owned("a", 1)], &now())
            .expect("planning succeeds");
        assert!(matches!(outcome, ActivationOutcome::Unchanged));
        // A no-op opens no pass, so a later real activation still admits.
        assert!(service.serving_generation() == Some(&digest(1)));
    }

    #[test]
    fn duplicate_bundle_resource_is_rejected() {
        let error = ResourceBundle::new(
            zone(),
            digest(1),
            vec![
                BundleResource::new(key("a"), spec("active")),
                BundleResource::new(key("a"), spec("active")),
            ],
        )
        .expect_err("duplicate rejected");
        assert_eq!(error.label(), "bundle-duplicate-resource");
    }

    #[test]
    fn foreign_zone_bundle_is_refused() {
        let mut service = service_at(1);
        let foreign = ResourceBundle::new(
            ZoneId::parse("personal").expect("valid zone"),
            digest(2),
            Vec::new(),
        )
        .expect("valid bundle");
        assert_eq!(
            activation_err(service.begin_activation(&foreign, &[], &now())),
            ConfigurationError::ZoneMismatch
        );
    }

    #[test]
    fn canonical_spec_comparison_ignores_rendering_order() {
        let rendered_one =
            CanonicalSpec::from_fields([("peer", "k1"), ("mode", "active")]).expect("valid");
        let rendered_two =
            CanonicalSpec::from_fields([("mode", "active"), ("peer", "k1")]).expect("valid");
        assert_eq!(rendered_one, rendered_two);
        assert_eq!(rendered_one.field_count(), 2);
        assert_eq!(
            CanonicalSpec::from_fields([("mode", "a"), ("mode", "b")]).expect_err("duplicate"),
            ConfigurationError::DuplicateSpecField
        );
    }

    #[test]
    fn unchanged_spec_plans_no_update_and_changed_spec_plans_one() {
        let mut service = service_at(1);
        let stored = vec![
            owned("a", 1),
            StoredResource::new(
                key("b"),
                ManagementAgent::Configuration,
                Some(digest(1)),
                spec("standby"),
            ),
        ];
        let plan = planned(
            service
                .begin_activation(&bundle(2, &["a", "b", "c"]), &stored, &now())
                .expect("planning succeeds"),
        );
        assert_eq!(plan.unchanged(), &[key("a")]);
        let labels: Vec<&str> = plan
            .planned_intents()
            .iter()
            .map(ConfigurationIntent::label)
            .collect();
        assert_eq!(labels, vec!["update-spec", "create"]);
    }

    #[test]
    fn controller_owned_resource_is_never_seized() {
        let mut service = service_at(1);
        let stored = vec![StoredResource::new(
            key("a"),
            ManagementAgent::Controller,
            None,
            spec("standby"),
        )];
        assert_eq!(
            activation_err(service.begin_activation(&bundle(2, &["a"]), &stored, &now())),
            ConfigurationError::ManagedByCollision
        );
        // Fail-closed means no pass was opened and no mutation occurred.
        assert_eq!(service.serving_generation(), Some(&digest(1)));
        assert!(service.pending_cleanup().is_empty());
    }

    #[test]
    fn configuration_owned_resource_without_generation_fails_closed() {
        let mut service = service_at(1);
        let stored = vec![StoredResource::new(
            key("a"),
            ManagementAgent::Configuration,
            None,
            spec("standby"),
        )];
        assert_eq!(
            activation_err(service.begin_activation(&bundle(2, &["a"]), &stored, &now())),
            ConfigurationError::ConfigurationGenerationMissing
        );
    }

    #[test]
    fn diff_deletes_only_matching_configuration_owned_resources() {
        let mut service = service_at(1);
        let stored = vec![
            owned("gone", 1),
            // Absent configurationGeneration: controller or API created.
            StoredResource::new(key("child"), ManagementAgent::Controller, None, spec("x")),
            // API created, coincidentally stamped: still never touched.
            StoredResource::new(
                key("adopted"),
                ManagementAgent::Api,
                Some(digest(1)),
                spec("x"),
            ),
            // Stale generation stamp: guards concurrent generation switches.
            owned("stale", 9),
        ];
        let plan = planned(
            service
                .begin_activation(&bundle(2, &[]), &stored, &now())
                .expect("planning succeeds"),
        );
        let deleted: Vec<&ResourceKey> = plan
            .planned_intents()
            .iter()
            .map(ConfigurationIntent::key)
            .collect();
        assert_eq!(deleted, vec![&key("gone")]);
    }

    #[test]
    fn commit_precedes_every_effect_and_reconcile_is_notified_last() {
        let mut service = service_at(1);
        let plan = planned(
            service
                .begin_activation(&bundle(2, &["a"]), &[owned("gone", 1)], &now())
                .expect("planning succeeds"),
        );
        // The outgoing bundle stages before the record commits.
        assert_eq!(plan.stage_generation(), Some(&digest(1)));
        assert_eq!(plan.next_record().active_generation_id(), &digest(2));
        assert_eq!(plan.next_record().prior_generation_id(), Some(&digest(1)));

        let proof = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        // The record is already active before any effect is obtainable.
        assert_eq!(service.serving_generation(), Some(&digest(2)));
        let effects = service
            .release_activation_effects(proof)
            .expect("release succeeds");
        assert!(matches!(
            effects.last(),
            Some(ConfigurationEffect::NotifyReconcile)
        ));
        let notify_index = effects.len() - 1;
        let queued = effects
            .iter()
            .position(|effect| matches!(effect, ConfigurationEffect::QueueIntent(_)))
            .expect("an intent is queued");
        assert!(queued < notify_index);
        assert!(effects.contains(&ConfigurationEffect::AppendAudit(
            ConfigurationAuditKind::GenerationActivate
        )));
    }

    #[test]
    fn aborted_pass_releases_no_effect() {
        let mut service = service_at(1);
        let plan = planned(
            service
                .begin_activation(&bundle(2, &["a"]), &[], &now())
                .expect("planning succeeds"),
        );
        service.abort_activation(plan);
        assert_eq!(service.serving_generation(), Some(&digest(1)));
        let replay = service.begin_activation(&bundle(2, &["a"]), &[], &now());
        assert!(matches!(replay, Ok(ActivationOutcome::Planned(_))));
    }

    #[test]
    fn second_activation_pass_fails_closed() {
        let mut service = service_at(1);
        let _plan = planned(
            service
                .begin_activation(&bundle(2, &["a"]), &[], &now())
                .expect("planning succeeds"),
        );
        assert_eq!(
            activation_err(service.begin_activation(&bundle(3, &["a"]), &[], &now())),
            ConfigurationError::ActivationInFlight
        );
    }

    #[test]
    fn effects_release_exactly_once() {
        let mut service = service_at(1);
        let plan = planned(
            service
                .begin_activation(&bundle(2, &["a"]), &[], &now())
                .expect("planning succeeds"),
        );
        let proof = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        assert!(service.release_activation_effects(proof).is_ok());

        let plan = planned(
            service
                .begin_activation(&bundle(3, &["a"]), &[], &now())
                .expect("planning succeeds"),
        );
        let stale = GenerationCommitProof { sequence: 999 };
        let fresh = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        assert_eq!(
            service.release_activation_effects(stale),
            Err(ConfigurationError::StaleCommitProof)
        );
        assert!(service.release_activation_effects(fresh).is_ok());
    }

    #[test]
    fn prior_bundle_ring_writes_and_prunes_oldest() {
        let mut service = ConfigurationService::empty(zone(), RetainedGenerations::default_value());
        for generation in 1..=6u8 {
            let plan = planned(
                service
                    .begin_activation(&bundle(generation, &[]), &[], &now())
                    .expect("planning succeeds"),
            );
            let proof = service
                .commit_activation(plan, &now())
                .expect("commit succeeds");
            service
                .release_activation_effects(proof)
                .expect("release succeeds");
        }
        let record = service.record().expect("a record exists");
        assert_eq!(record.retention_ring().len(), 3);
        assert_eq!(
            record.retention_ring(),
            &[digest(3), digest(4), digest(5)][..]
        );
        assert_eq!(record.active_ordinal().get(), 6);
    }

    #[test]
    fn retention_bound_is_validated() {
        assert_eq!(RetainedGenerations::default_value().get(), 3);
        assert!(RetainedGenerations::new(0).is_err());
        assert!(RetainedGenerations::new(17).is_err());
        assert!(RetainedGenerations::new(16).is_ok());
    }

    #[test]
    fn a_generation_with_an_in_flight_delete_is_never_pruned() {
        let mut service = ConfigurationService::empty(
            zone(),
            RetainedGenerations::new(1).expect("valid retention"),
        );
        // Generation 1 owns `gone`.
        let plan = planned(
            service
                .begin_activation(&bundle(1, &["gone"]), &[], &now())
                .expect("planning succeeds"),
        );
        let proof = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        service
            .release_activation_effects(proof)
            .expect("release succeeds");
        service
            .complete_intent(&key("gone"))
            .expect("intent completes");

        // Generation 2 removes it, so a Delete from generation 1 is in flight.
        let plan = planned(
            service
                .begin_activation(&bundle(2, &[]), &[owned("gone", 1)], &now())
                .expect("planning succeeds"),
        );
        let proof = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        service
            .release_activation_effects(proof)
            .expect("release succeeds");
        assert_eq!(service.phase(), GenerationPhase::Degraded);

        // Generation 3 would overflow a ring of one, but generation 1 is both
        // the source of the in-flight Delete and the incoming prior pointer.
        let plan = planned(
            service
                .begin_activation(&bundle(3, &[]), &[], &now())
                .expect("planning succeeds"),
        );
        assert!(plan.planned_prunes().is_empty());
        assert!(plan.next_record().retention_ring().contains(&digest(1)));
    }

    #[test]
    fn restart_adopts_a_verified_active_generation() {
        let record = GenerationRecord::new(
            digest(5),
            ConfigurationGeneration::new(4).expect("nonzero"),
            Some(digest(4)),
            RetainedGenerations::default_value(),
            vec![digest(4)],
        )
        .expect("valid record");
        let service =
            ConfigurationService::restore(zone(), record, StagedBundleIntegrity::Verified);
        assert_eq!(service.status(), ActivationStatus::Active);
        assert_eq!(service.serving_generation(), Some(&digest(5)));
        assert_eq!(service.quarantined_generation(), None);
    }

    #[test]
    fn restart_quarantines_an_ambiguous_generation_and_serves_the_prior_pointer() {
        for staged in [
            StagedBundleIntegrity::Missing,
            StagedBundleIntegrity::Mismatched,
        ] {
            let record = GenerationRecord::new(
                digest(5),
                ConfigurationGeneration::new(4).expect("nonzero"),
                Some(digest(4)),
                RetainedGenerations::default_value(),
                vec![digest(4)],
            )
            .expect("valid record");
            let mut service = ConfigurationService::restore(zone(), record, staged);
            assert_eq!(service.status(), ActivationStatus::Quarantined);
            assert_eq!(service.serving_generation(), Some(&digest(4)));
            assert_eq!(service.quarantined_generation(), Some(&digest(5)));

            // Recovery resolved before planning, and the quarantined
            // generation is not a prune candidate.
            let plan = planned(
                service
                    .begin_activation(&bundle(6, &[]), &[], &now())
                    .expect("planning succeeds"),
            );
            assert!(plan.planned_prunes().is_empty());
        }
    }

    #[test]
    fn cleanup_waits_for_finalizers_then_children_then_commits() {
        let mut service = service_at(1);
        let plan = planned(
            service
                .begin_activation(&bundle(2, &[]), &[owned("gone", 1)], &now())
                .expect("planning succeeds"),
        );
        let proof = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        service
            .release_activation_effects(proof)
            .expect("release succeeds");

        let tracking = service
            .cleanup_tracking(&key("gone"))
            .expect("cleanup tracked");
        assert_eq!(tracking.deletion_requested_at().as_str(), NOW);
        assert_eq!(tracking.cleanup_config_generation(), &digest(1));
        assert_eq!(tracking.cleanup_attempt(), 0);
        assert_eq!(tracking.cleanup_error(), None);

        let pass = service
            .begin_cleanup(&key("gone"), CleanupObservation::new(1, 0))
            .expect("pass opens");
        assert_eq!(pass.step(), CleanupStep::DrainFinalizers);
        assert_eq!(
            cleanup_err(service.begin_cleanup(&key("gone"), CleanupObservation::new(1, 0))),
            ConfigurationError::CleanupInFlight
        );
        let proof = service
            .commit_cleanup(pass, ZoneRevision::new(7))
            .expect("commit succeeds");
        assert_eq!(
            service
                .release_cleanup_effects(proof)
                .expect("release succeeds"),
            vec![CleanupEffect::NotifyFinalizerHolders]
        );

        // A live controller-created child blocks completion.
        let pass = service
            .begin_cleanup(&key("gone"), CleanupObservation::new(0, 2))
            .expect("pass opens");
        assert_eq!(pass.step(), CleanupStep::CascadeChildren);
        let proof = service
            .commit_cleanup(pass, ZoneRevision::new(8))
            .expect("commit succeeds");
        assert_eq!(
            service
                .release_cleanup_effects(proof)
                .expect("release succeeds"),
            vec![CleanupEffect::NotifyControllerCascade]
        );
        assert!(service.cleanup_tracking(&key("gone")).is_some());

        let pass = service
            .begin_cleanup(&key("gone"), CleanupObservation::new(0, 0))
            .expect("pass opens");
        assert_eq!(pass.step(), CleanupStep::CommitDeletion);
        let proof = service
            .commit_cleanup(pass, ZoneRevision::new(9))
            .expect("commit succeeds");
        assert_eq!(
            service
                .release_cleanup_effects(proof)
                .expect("release succeeds"),
            vec![CleanupEffect::AppendCleanupAudit(ZoneRevision::new(9))]
        );
        assert!(service.cleanup_tracking(&key("gone")).is_none());
        assert_eq!(service.phase(), GenerationPhase::Ready);
    }

    #[test]
    fn cleanup_audit_follows_the_store_transaction_and_appends_exactly_once() {
        let mut service = service_at(1);
        let plan = planned(
            service
                .begin_activation(&bundle(2, &[]), &[owned("gone", 1)], &now())
                .expect("planning succeeds"),
        );
        let proof = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        service
            .release_activation_effects(proof)
            .expect("release succeeds");

        let pass = service
            .begin_cleanup(&key("gone"), CleanupObservation::new(0, 0))
            .expect("pass opens");
        // No audit record is obtainable before the store transaction commits.
        let stale = CleanupCommitProof { sequence: 99 };
        assert_eq!(
            service.release_cleanup_effects(stale),
            Err(ConfigurationError::StaleCommitProof)
        );

        let proof = service
            .commit_cleanup(pass, ZoneRevision::new(11))
            .expect("commit succeeds");
        let effects = service
            .release_cleanup_effects(proof)
            .expect("release succeeds");
        assert_eq!(
            effects,
            vec![CleanupEffect::AppendCleanupAudit(ZoneRevision::new(11))]
        );
        assert_eq!(
            service
                .record_cleanup_audit_appended(ZoneRevision::new(11))
                .expect("first append"),
            ConfigurationAuditKind::ResourceCleanup
        );
        // Recovery replay of the same committed revision is refused.
        assert_eq!(
            service.record_cleanup_audit_appended(ZoneRevision::new(11)),
            Err(ConfigurationError::AuditAlreadyAppended)
        );
    }

    #[test]
    fn cleanup_for_an_untracked_resource_is_refused() {
        let mut service = service_at(1);
        assert_eq!(
            cleanup_err(service.begin_cleanup(&key("absent"), CleanupObservation::new(0, 0))),
            ConfigurationError::CleanupNotRequested
        );
        assert_eq!(
            service.record_cleanup_failure(&key("absent"), ConfigurationError::StaleCommitProof),
            Err(ConfigurationError::CleanupNotRequested)
        );
    }

    #[test]
    fn permanent_delete_failure_moves_the_generation_to_failed() {
        let mut service = service_at(1);
        let plan = planned(
            service
                .begin_activation(&bundle(2, &[]), &[owned("gone", 1)], &now())
                .expect("planning succeeds"),
        );
        let proof = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        service
            .release_activation_effects(proof)
            .expect("release succeeds");
        assert_eq!(
            service
                .record_cleanup_failure(&key("gone"), ConfigurationError::CleanupNotCompleted)
                .expect("failure recorded"),
            ConfigurationAuditKind::ResourceCleanupFailed
        );
        assert_eq!(service.phase(), GenerationPhase::Failed);
        let tracking = service
            .cleanup_tracking(&key("gone"))
            .expect("still tracked");
        assert_eq!(
            tracking.cleanup_error(),
            Some(ConfigurationError::CleanupNotCompleted)
        );
        assert_eq!(tracking.cleanup_attempt(), 1);
    }

    #[test]
    fn phase_is_pending_while_creates_are_outstanding() {
        let mut service = service_at(1);
        let plan = planned(
            service
                .begin_activation(&bundle(2, &["a"]), &[], &now())
                .expect("planning succeeds"),
        );
        let proof = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        service
            .release_activation_effects(proof)
            .expect("release succeeds");
        assert_eq!(service.phase(), GenerationPhase::Pending);
        assert_eq!(
            service.complete_intent(&key("b")),
            Err(ConfigurationError::IntentNotOutstanding)
        );
        service
            .complete_intent(&key("a"))
            .expect("intent completes");
        assert_eq!(service.phase(), GenerationPhase::Ready);
    }

    #[test]
    fn rollback_restages_a_retained_generation_and_revives_pending_deletes() {
        let mut service = ConfigurationService::empty(zone(), RetainedGenerations::default_value());
        // Generation 1 declares `link`.
        let plan = planned(
            service
                .begin_activation(&bundle(1, &["link"]), &[], &now())
                .expect("planning succeeds"),
        );
        let proof = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        service
            .release_activation_effects(proof)
            .expect("release succeeds");
        service
            .complete_intent(&key("link"))
            .expect("intent completes");

        // Generation 2 removes it.
        let plan = planned(
            service
                .begin_activation(&bundle(2, &[]), &[owned("link", 1)], &now())
                .expect("planning succeeds"),
        );
        let proof = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        service
            .release_activation_effects(proof)
            .expect("release succeeds");
        assert_eq!(service.pending_cleanup(), vec![&key("link")]);

        // Rolling back to generation 1 cancels the Delete and clears the
        // pending-delete tracking.
        let plan = planned(
            service
                .begin_rollback(&bundle(1, &["link"]), &[owned("link", 1)], &now())
                .expect("rollback plans"),
        );
        assert_eq!(
            plan.audit_kind(),
            ConfigurationAuditKind::GenerationRollback
        );
        assert!(
            plan.planned_intents()
                .contains(&ConfigurationIntent::CancelDelete(key("link")))
        );
        let proof = service
            .commit_activation(plan, &now())
            .expect("commit succeeds");
        service
            .release_activation_effects(proof)
            .expect("release succeeds");
        assert!(service.pending_cleanup().is_empty());
        assert_eq!(service.serving_generation(), Some(&digest(1)));
    }

    #[test]
    fn rollback_to_an_unretained_generation_is_refused() {
        let mut service = service_at(1);
        assert_eq!(
            activation_err(service.begin_rollback(&bundle(9, &[]), &[], &now())),
            ConfigurationError::RollbackTargetUnavailable
        );
    }

    #[test]
    fn error_labels_are_closed_and_kebab_case() {
        for error in [
            ConfigurationError::ZoneMismatch,
            ConfigurationError::DuplicateResource,
            ConfigurationError::DuplicateSpecField,
            ConfigurationError::ActivationInFlight,
            ConfigurationError::StaleCommitProof,
            ConfigurationError::ManagedByCollision,
            ConfigurationError::ConfigurationGenerationMissing,
            ConfigurationError::RollbackTargetUnavailable,
            ConfigurationError::RetentionOutOfRange,
            ConfigurationError::GenerationOrdinalExhausted,
            ConfigurationError::CleanupNotRequested,
            ConfigurationError::CleanupInFlight,
            ConfigurationError::AuditAlreadyAppended,
            ConfigurationError::CleanupNotCompleted,
            ConfigurationError::IntentNotOutstanding,
        ] {
            let label = error.label();
            assert!(!label.is_empty());
            assert!(
                label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'-'),
                "{label}"
            );
            assert_eq!(error.to_string(), label);
        }
    }

    #[test]
    fn identity_bearing_debug_output_is_redacted() {
        let rendered = format!("{:?}", key("secret-link"));
        assert!(!rendered.contains("secret-link"), "{rendered}");
        let rendered = format!("{:?}", spec("secret-mode"));
        assert!(!rendered.contains("secret-mode"), "{rendered}");
        let service = service_at(1);
        let rendered = format!("{service:?}");
        assert!(!rendered.contains("sha256:"), "{rendered}");
    }

    #[test]
    fn closed_labels_are_stable() {
        assert_eq!(ManagementAgent::Configuration.label(), "configuration");
        assert_eq!(ManagementAgent::Controller.label(), "controller");
        assert_eq!(ManagementAgent::Api.label(), "api");
        assert_eq!(GenerationPhase::Degraded.label(), "Degraded");
        assert_eq!(ActivationStatus::Quarantined.label(), "quarantined");
        assert_eq!(CleanupStep::CommitDeletion.label(), "commit-deletion");
        assert_eq!(
            ConfigurationAuditKind::GenerationActivate.label(),
            "zone-generation-activate"
        );
        assert_eq!(
            ConfigurationAuditKind::ResourceCleanup.label(),
            "zone-resource-cleanup"
        );
        assert_eq!(
            ConfigurationAuditKind::ResourceCleanupFailed.label(),
            "zone-resource-cleanup-failed"
        );
        assert_eq!(
            ConfigurationAuditKind::GenerationRollback.label(),
            "zone-generation-rollback"
        );
    }
}
