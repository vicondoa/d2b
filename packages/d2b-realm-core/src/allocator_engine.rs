//! Hermetic local-root allocator decision engine.
//!
//! The engine is deliberately pure: it reconciles typed allocator data models against
//! caller-supplied ledger, liveness, and observation adapters and emits decisions,
//! audit metadata, and low-cardinality metric samples. Adapters expose already-loaded
//! state only; the engine performs no netlink, nftables, filesystem, systemd, broker,
//! or other live host mutation.

use crate::allocator::{
    AllocatorConflict, AllocatorEventKind, AllocatorEventMetadata, AllocatorLease,
    AllocatorLeaseState, AllocatorReasonCode, GrantedHostResource, HostResourceKind,
    LeaseAllocationRequest, LeaseAllocationResponse, LeaseAllocationResult, LeaseOwner,
    MAX_ALLOCATOR_CONFLICTS, MAX_ALLOCATOR_EVENTS, MAX_ALLOCATOR_REQUEST_RESOURCES,
    MAX_RECONCILIATION_RECORDS, ObservedHostResource, ObservedResourceState,
    PersistedResourceLease, ReconciliationDecision, ReconciliationRecord, ReconciliationReport,
    ResourceAcquisitionKey, ResourceDelegation, ResourceObservationSource, ResourceShareMode,
};
use crate::ids::{AllocatorLeaseId, CorrelationId, HostResourceId, IdempotencyKey, OperationId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Allocation/reconciliation decision made by the pure engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocatorEngineDecision {
    Grant,
    DenyConflict { reason: AllocatorReasonCode },
    Reclaim { reason: AllocatorReasonCode },
    Quarantine { reason: AllocatorReasonCode },
    Preserve { reason: AllocatorReasonCode },
    Reconcile,
}

impl AllocatorEngineDecision {
    fn reason(&self) -> Option<AllocatorReasonCode> {
        match self {
            Self::Grant | Self::Reconcile => None,
            Self::DenyConflict { reason }
            | Self::Reclaim { reason }
            | Self::Quarantine { reason }
            | Self::Preserve { reason } => Some(*reason),
        }
    }

    fn event_kind(&self) -> AllocatorEventKind {
        match self {
            Self::Grant => AllocatorEventKind::Grant,
            Self::DenyConflict { .. } => AllocatorEventKind::Denial,
            Self::Reclaim { .. } => AllocatorEventKind::Reclamation,
            Self::Quarantine { .. } => AllocatorEventKind::Quarantine,
            Self::Preserve { .. } | Self::Reconcile => AllocatorEventKind::Reconciliation,
        }
    }

    fn outcome(&self) -> AllocatorEngineOutcome {
        match self {
            Self::Grant => AllocatorEngineOutcome::Granted,
            Self::DenyConflict { .. } => AllocatorEngineOutcome::Denied,
            Self::Reclaim { .. } => AllocatorEngineOutcome::Reclaimed,
            Self::Quarantine { .. } => AllocatorEngineOutcome::Quarantined,
            Self::Preserve { .. } => AllocatorEngineOutcome::Preserved,
            Self::Reconcile => AllocatorEngineOutcome::Reconciled,
        }
    }

    fn reconciliation_decision(&self) -> ReconciliationDecision {
        match self {
            Self::Reconcile => ReconciliationDecision::Reconciled,
            Self::Reclaim { reason } => ReconciliationDecision::Reclaim { reason: *reason },
            Self::Quarantine { reason } => ReconciliationDecision::Quarantine { reason: *reason },
            Self::Preserve { reason } | Self::DenyConflict { reason } => {
                ReconciliationDecision::Deny { reason: *reason }
            }
            Self::Grant => ReconciliationDecision::Reconciled,
        }
    }
}

/// Low-cardinality metric outcome label emitted by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AllocatorEngineOutcome {
    Granted,
    Denied,
    Reconciled,
    Reclaimed,
    Quarantined,
    Preserved,
    IdempotentReplay,
}

impl AllocatorEngineOutcome {
    /// Stable metric label; never contains resource ids, paths, endpoints, or
    /// other caller-controlled values.
    pub fn as_metric_label(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::Reconciled => "reconciled",
            Self::Reclaimed => "reclaimed",
            Self::Quarantined => "quarantined",
            Self::Preserved => "preserved",
            Self::IdempotentReplay => "idempotent-replay",
        }
    }
}

/// Bounded low-cardinality metric sample. Labels are closed enums only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatorMetricEvent {
    pub event: AllocatorEventKind,
    pub outcome: AllocatorEngineOutcome,
    pub resource_kind: Option<HostResourceKind>,
    pub reason: Option<AllocatorReasonCode>,
    pub count: u64,
}

impl AllocatorMetricEvent {
    fn new(
        event: AllocatorEventKind,
        outcome: AllocatorEngineOutcome,
        resource_kind: Option<HostResourceKind>,
        reason: Option<AllocatorReasonCode>,
    ) -> Self {
        Self {
            event,
            outcome,
            resource_kind,
            reason,
            count: 1,
        }
    }

    /// Labels suitable for metric export. Values are static enum labels only.
    pub fn labels(&self) -> AllocatorMetricLabels {
        AllocatorMetricLabels {
            event: self.event.as_metric_label(),
            outcome: self.outcome.as_metric_label(),
            resource_kind: self.resource_kind.map(HostResourceKind::as_metric_label),
            reason: self.reason.map(AllocatorReasonCode::as_metric_label),
        }
    }
}

/// Static labels derived from an [`AllocatorMetricEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AllocatorMetricLabels {
    pub event: &'static str,
    pub outcome: &'static str,
    pub resource_kind: Option<&'static str>,
    pub reason: Option<&'static str>,
}

/// One resource-level allocation decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatorAllocationDecision {
    pub resource_id: HostResourceId,
    pub kind: HostResourceKind,
    pub decision: AllocatorEngineDecision,
}

/// Result of one allocation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatorEngineAllocation {
    pub response: LeaseAllocationResponse,
    pub decisions: Vec<AllocatorAllocationDecision>,
    pub events: Vec<AllocatorEventMetadata>,
    pub metrics: Vec<AllocatorMetricEvent>,
    pub acquired: Vec<ResourceAcquisitionKey>,
}

/// One resource-level reconciliation decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatorReconciliationAction {
    pub resource_id: HostResourceId,
    pub kind: HostResourceKind,
    pub persisted: Option<PersistedResourceLease>,
    pub observed: ObservedHostResource,
    pub decision: AllocatorEngineDecision,
}

/// Result of reconciling observed host state against persisted leases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatorEngineReconciliation {
    pub report: ReconciliationReport,
    pub actions: Vec<AllocatorReconciliationAction>,
    pub metrics: Vec<AllocatorMetricEvent>,
}

/// Closed failure surface for allocator state access.
///
/// Variants deliberately carry no paths, lock names, generations, host state, or
/// underlying error strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocatorEngineError {
    LedgerLockUnavailable,
    LedgerIo,
    LedgerGenerationConflict,
    LedgerTampered,
}

impl std::fmt::Display for AllocatorEngineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::LedgerLockUnavailable => "allocator ledger lock unavailable",
            Self::LedgerIo => "allocator ledger I/O failed",
            Self::LedgerGenerationConflict => "allocator ledger generation changed",
            Self::LedgerTampered => "allocator ledger integrity check failed",
        })
    }
}

impl std::error::Error for AllocatorEngineError {}

/// Opaque compare-and-swap generation for one durable ledger snapshot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AllocatorLedgerGeneration(u64);

impl AllocatorLedgerGeneration {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    #[cfg(any(test, feature = "test-support"))]
    fn checked_next(self) -> Result<Self, AllocatorEngineError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(AllocatorEngineError::LedgerTampered)
    }
}

/// One consistent, generation-bound read of durable allocator state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllocatorLedgerSnapshot {
    generation: AllocatorLedgerGeneration,
    leases: Vec<AllocatorLease>,
    idempotency: Vec<AllocatorIdempotencyRecord>,
}

impl AllocatorLedgerSnapshot {
    pub fn new(
        generation: AllocatorLedgerGeneration,
        leases: Vec<AllocatorLease>,
        idempotency: Vec<AllocatorIdempotencyRecord>,
    ) -> Self {
        Self {
            generation,
            leases,
            idempotency,
        }
    }

    pub const fn generation(&self) -> AllocatorLedgerGeneration {
        self.generation
    }

    pub fn leases(&self) -> &[AllocatorLease] {
        &self.leases
    }

    pub fn idempotency_record(&self, key: &IdempotencyKey) -> Option<&AllocatorIdempotencyRecord> {
        self.idempotency.iter().find(|record| record.key() == key)
    }
}

/// Outcome type requested by an atomic allocator ledger commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocatorLedgerCommitKind {
    Grant,
    Denial,
}

/// Engine-created transaction passed to [`AllocatorLedger::commit_allocation`].
///
/// Its fields are private so adapters cannot replace the engine-owned request
/// fingerprint or grant contents. An adapter chooses a lease id while holding its
/// exclusive lock, calls [`Self::materialize`], and durably publishes the sequence,
/// lease, and idempotency record as one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatorLedgerCommit {
    expected_generation: AllocatorLedgerGeneration,
    request: LeaseAllocationRequest,
    decision: PendingAllocationDecision,
}

impl AllocatorLedgerCommit {
    pub const fn expected_generation(&self) -> AllocatorLedgerGeneration {
        self.expected_generation
    }

    pub fn idempotency_key(&self) -> &IdempotencyKey {
        &self.request.idempotency_key
    }

    pub const fn kind(&self) -> AllocatorLedgerCommitKind {
        match &self.decision {
            PendingAllocationDecision::Grant { .. } => AllocatorLedgerCommitKind::Grant,
            PendingAllocationDecision::Denial { .. } => AllocatorLedgerCommitKind::Denial,
        }
    }

    /// Materialize the exact engine-owned result and idempotency record.
    ///
    /// Grant commits require exactly one adapter-reserved lease id. Denial commits
    /// require none.
    pub fn materialize(
        &self,
        lease_id: Option<AllocatorLeaseId>,
    ) -> Result<AllocatorLedgerCommitResult, AllocatorEngineError> {
        let result = match (&self.decision, lease_id) {
            (PendingAllocationDecision::Grant { resources }, Some(lease_id)) => {
                LeaseAllocationResult::Granted {
                    lease: AllocatorLease {
                        lease_id,
                        owner: self.request.owner.clone(),
                        state: AllocatorLeaseState::Granted,
                        resources: resources.clone(),
                    },
                }
            }
            (PendingAllocationDecision::Denial { reason, conflicts }, None) => {
                LeaseAllocationResult::Denied {
                    reason: *reason,
                    conflicts: conflicts.clone(),
                }
            }
            _ => return Err(AllocatorEngineError::LedgerTampered),
        };
        Ok(AllocatorLedgerCommitResult {
            idempotency: AllocatorIdempotencyRecord::from_request(&self.request, result.clone()),
            result,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingAllocationDecision {
    Grant {
        resources: Vec<GrantedHostResource>,
    },
    Denial {
        reason: AllocatorReasonCode,
        conflicts: Vec<AllocatorConflict>,
    },
}

/// Exact result durably published by an allocator ledger transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatorLedgerCommitResult {
    result: LeaseAllocationResult,
    idempotency: AllocatorIdempotencyRecord,
}

impl AllocatorLedgerCommitResult {
    pub fn result(&self) -> &LeaseAllocationResult {
        &self.result
    }

    pub fn idempotency_record(&self) -> &AllocatorIdempotencyRecord {
        &self.idempotency
    }
}

/// Durable allocator state required by [`LocalRootAllocatorEngine`].
///
/// `load` must return one consistent snapshot. `commit_allocation` must acquire the
/// adapter's exclusive lock, compare `expected_generation`, and publish all changes
/// atomically. A grant reserves its lease id, advances the sequence/generation,
/// inserts the lease, and stores the idempotency record in the same durable commit.
/// On any error, it must expose either the complete old state or the complete new
/// state, never an intermediate state.
pub trait AllocatorLedger: Send + Sync {
    fn load(&self) -> Result<AllocatorLedgerSnapshot, AllocatorEngineError>;

    fn commit_allocation(
        &mut self,
        commit: AllocatorLedgerCommit,
    ) -> Result<AllocatorLedgerCommitResult, AllocatorEngineError>;
}

/// Already-observed host resource state required by the allocator engine.
///
/// Observation and host I/O happen before this adapter is passed to the engine.
pub trait ObservedAllocatorState: Send + Sync {
    fn resources(&self) -> &[ObservedHostResource];
}

/// Controller-generation liveness snapshot required by the allocator engine.
pub trait AllocatorLiveness: Send + Sync {
    fn is_live(&self, owner: &LeaseOwner) -> bool;
}

/// Opaque engine-owned idempotency record stored by an [`AllocatorLedger`].
///
/// Adapters may retain and return this value, but request fingerprint construction
/// and comparison remain engine-owned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocatorIdempotencyRecord {
    key: IdempotencyKey,
    signature: AllocationRequestSignature,
    result: LeaseAllocationResult,
}

impl AllocatorIdempotencyRecord {
    /// Construct the canonical record for a completed request.
    pub fn from_request(request: &LeaseAllocationRequest, result: LeaseAllocationResult) -> Self {
        Self {
            key: request.idempotency_key.clone(),
            signature: AllocationRequestSignature::from_request(request),
            result,
        }
    }

    /// Key used by ledger adapters to index this record.
    pub fn key(&self) -> &IdempotencyKey {
        &self.key
    }

    /// Previously committed result. Adapters persist it as opaque engine-owned
    /// state and must not reinterpret it.
    pub fn result(&self) -> &LeaseAllocationResult {
        &self.result
    }
}

/// In-memory observed-state adapter for tests and support tooling.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeObservedAllocatorState {
    pub resources: Vec<ObservedHostResource>,
}

#[cfg(any(test, feature = "test-support"))]
impl FakeObservedAllocatorState {
    pub fn new(resources: Vec<ObservedHostResource>) -> Self {
        Self { resources }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ObservedAllocatorState for FakeObservedAllocatorState {
    fn resources(&self) -> &[ObservedHostResource] {
        &self.resources
    }
}

/// In-memory liveness adapter for tests and support tooling.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeAllocatorLiveness {
    pub live_owners: Vec<LeaseOwner>,
}

#[cfg(any(test, feature = "test-support"))]
impl FakeAllocatorLiveness {
    pub fn new(live_owners: Vec<LeaseOwner>) -> Self {
        Self { live_owners }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl AllocatorLiveness for FakeAllocatorLiveness {
    fn is_live(&self, owner: &LeaseOwner) -> bool {
        self.live_owners.iter().any(|candidate| candidate == owner)
    }
}

/// In-memory ledger adapter for tests and support tooling.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeAllocatorLedger {
    pub leases: Vec<AllocatorLease>,
    idempotency: Vec<AllocatorIdempotencyRecord>,
    next_lease_sequence: u64,
    generation: AllocatorLedgerGeneration,
}

#[cfg(any(test, feature = "test-support"))]
impl Default for FakeAllocatorLedger {
    fn default() -> Self {
        Self {
            leases: Vec::new(),
            idempotency: Vec::new(),
            next_lease_sequence: 1,
            generation: AllocatorLedgerGeneration::default(),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl FakeAllocatorLedger {
    pub fn new(leases: Vec<AllocatorLease>) -> Self {
        Self {
            leases,
            ..Self::default()
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl AllocatorLedger for FakeAllocatorLedger {
    fn load(&self) -> Result<AllocatorLedgerSnapshot, AllocatorEngineError> {
        Ok(AllocatorLedgerSnapshot::new(
            self.generation,
            self.leases.clone(),
            self.idempotency.clone(),
        ))
    }

    fn commit_allocation(
        &mut self,
        commit: AllocatorLedgerCommit,
    ) -> Result<AllocatorLedgerCommitResult, AllocatorEngineError> {
        if commit.expected_generation() != self.generation
            || self
                .idempotency
                .iter()
                .any(|record| record.key() == commit.idempotency_key())
        {
            return Err(AllocatorEngineError::LedgerGenerationConflict);
        }

        let mut next_sequence = self.next_lease_sequence;
        let mut leases = self.leases.clone();
        let mut idempotency = self.idempotency.clone();
        let lease_id = if commit.kind() == AllocatorLedgerCommitKind::Grant {
            let existing = leases
                .iter()
                .map(|lease| lease.lease_id.as_str())
                .collect::<BTreeSet<_>>();
            Some(loop {
                let candidate = format!("lease-engine-{next_sequence}");
                next_sequence = next_sequence
                    .checked_add(1)
                    .ok_or(AllocatorEngineError::LedgerTampered)?;
                if existing.contains(candidate.as_str()) {
                    continue;
                }
                break AllocatorLeaseId::parse(candidate)
                    .map_err(|_| AllocatorEngineError::LedgerTampered)?;
            })
        } else {
            None
        };
        let committed = commit.materialize(lease_id)?;
        if let LeaseAllocationResult::Granted { lease } = committed.result() {
            leases.push(lease.clone());
        }
        idempotency.push(committed.idempotency_record().clone());
        let generation = self.generation.checked_next()?;

        self.leases = leases;
        self.idempotency = idempotency;
        self.next_lease_sequence = next_sequence;
        self.generation = generation;
        Ok(committed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AllocationRequestSignature {
    owner: LeaseOwner,
    resources: Vec<RequestedResourceSignature>,
}

impl AllocationRequestSignature {
    fn from_request(request: &LeaseAllocationRequest) -> Self {
        let mut resources = request
            .resources
            .iter()
            .map(|resource| RequestedResourceSignature {
                key: ResourceAcquisitionKey {
                    order: resource.acquisition_order,
                    kind: resource.kind,
                    resource_id: resource.resource_id.clone(),
                },
                share: resource.share,
            })
            .collect::<Vec<_>>();
        resources.sort();
        Self {
            owner: request.owner.clone(),
            resources,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestedResourceSignature {
    key: ResourceAcquisitionKey,
    share: ResourceShareMode,
}

impl PartialOrd for RequestedResourceSignature {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RequestedResourceSignature {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key
            .cmp(&other.key)
            .then_with(|| share_rank(self.share).cmp(&share_rank(other.share)))
    }
}

/// Pure allocator engine over explicitly injected state adapters.
///
/// No fake adapter defaults exist on the production surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRootAllocatorEngine<L, O, V>
where
    L: AllocatorLedger,
    O: ObservedAllocatorState,
    V: AllocatorLiveness,
{
    allocator_owner: LeaseOwner,
    ledger: L,
    observed: O,
    liveness: V,
}

impl<L, O, V> LocalRootAllocatorEngine<L, O, V>
where
    L: AllocatorLedger,
    O: ObservedAllocatorState,
    V: AllocatorLiveness,
{
    pub fn new(allocator_owner: LeaseOwner, ledger: L, observed: O, liveness: V) -> Self {
        Self {
            allocator_owner,
            ledger,
            observed,
            liveness,
        }
    }

    pub fn ledger(&self) -> &L {
        &self.ledger
    }

    pub fn observed(&self) -> &O {
        &self.observed
    }

    pub fn liveness(&self) -> &V {
        &self.liveness
    }

    pub fn into_adapters(self) -> (L, O, V) {
        (self.ledger, self.observed, self.liveness)
    }

    /// Allocate a typed lease request without touching the live host.
    ///
    /// A successful grant is returned only after the ledger has durably committed
    /// its lease id reservation, lease, idempotency record, and generation change.
    pub fn allocate(
        &mut self,
        request: LeaseAllocationRequest,
    ) -> Result<AllocatorEngineAllocation, AllocatorEngineError> {
        let snapshot = self.ledger.load()?;
        validate_snapshot(&snapshot)?;
        let acquired = request.acquisition_order();
        let signature = AllocationRequestSignature::from_request(&request);

        if let Some(record) = snapshot.idempotency_record(&request.idempotency_key) {
            if record.key == request.idempotency_key && record.signature == signature {
                let metric = metric_from_replay_result(&record.result);
                let response = LeaseAllocationResponse {
                    operation_id: request.operation_id.clone(),
                    correlation_id: request.correlation_id.clone(),
                    result: record.result.clone(),
                };
                return Ok(AllocatorEngineAllocation {
                    response,
                    decisions: Vec::new(),
                    events: Vec::new(),
                    metrics: vec![metric],
                    acquired,
                });
            }

            return self.deny_request(
                &snapshot,
                &request,
                acquired,
                AllocatorReasonCode::InvalidRequest,
                Vec::new(),
                false,
            );
        }

        if !self.liveness.is_live(&request.owner) {
            return self.deny_request(
                &snapshot,
                &request,
                acquired,
                AllocatorReasonCode::OwnerNotLive,
                Vec::new(),
                true,
            );
        }

        if request.resources.is_empty() || request.resources.len() > MAX_ALLOCATOR_REQUEST_RESOURCES
        {
            return self.deny_request(
                &snapshot,
                &request,
                acquired,
                AllocatorReasonCode::InvalidRequest,
                Vec::new(),
                true,
            );
        }

        if has_duplicate_requested_resources(&request) {
            return self.deny_request(
                &snapshot,
                &request,
                acquired,
                AllocatorReasonCode::InvalidRequest,
                Vec::new(),
                true,
            );
        }

        let conflicts = self.allocation_conflicts(&request, snapshot.leases());
        if !conflicts.is_empty() {
            return self.deny_request(
                &snapshot,
                &request,
                acquired,
                AllocatorReasonCode::ResourceConflict,
                conflicts,
                true,
            );
        }

        let expected_resources = grant_resources_in_order(&request);
        let commit = AllocatorLedgerCommit {
            expected_generation: snapshot.generation(),
            request: request.clone(),
            decision: PendingAllocationDecision::Grant {
                resources: expected_resources.clone(),
            },
        };
        let committed = self.ledger.commit_allocation(commit)?;
        let LeaseAllocationResult::Granted { lease } = committed.result() else {
            return Err(AllocatorEngineError::LedgerTampered);
        };
        if lease.owner != request.owner
            || lease.state != AllocatorLeaseState::Granted
            || lease.resources != expected_resources
            || snapshot
                .leases()
                .iter()
                .any(|existing| existing.lease_id == lease.lease_id)
        {
            return Err(AllocatorEngineError::LedgerTampered);
        }
        let result = LeaseAllocationResult::Granted {
            lease: lease.clone(),
        };
        if committed.idempotency_record()
            != &AllocatorIdempotencyRecord::from_request(&request, result.clone())
        {
            return Err(AllocatorEngineError::LedgerTampered);
        }

        let decisions = request
            .resources
            .iter()
            .map(|resource| AllocatorAllocationDecision {
                resource_id: resource.resource_id.clone(),
                kind: resource.kind,
                decision: AllocatorEngineDecision::Grant,
            })
            .collect::<Vec<_>>();
        let events = bounded_events(decisions.iter().map(|decision| {
            event_metadata(
                request.operation_id.clone(),
                request.correlation_id.clone(),
                request.owner.clone(),
                &decision.decision,
                Some(decision.kind),
                request.trace.clone(),
            )
        }));
        let metrics = bounded_metrics(
            decisions
                .iter()
                .map(|decision| metric_from_decision(&decision.decision, Some(decision.kind))),
        );
        let response = LeaseAllocationResponse {
            operation_id: request.operation_id,
            correlation_id: request.correlation_id,
            result,
        };

        Ok(AllocatorEngineAllocation {
            response,
            decisions,
            events,
            metrics,
            acquired,
        })
    }

    /// Reconcile observed host state against persisted leases.
    pub fn reconcile(
        &self,
        operation_id: OperationId,
        correlation_id: CorrelationId,
    ) -> Result<AllocatorEngineReconciliation, AllocatorEngineError> {
        let snapshot = self.ledger.load()?;
        validate_snapshot(&snapshot)?;
        let mut resources = BTreeMap::<ResourceKey, ResourcePair>::new();

        for lease in snapshot.leases() {
            for resource in &lease.resources {
                let key = ResourceKey::new(resource.kind, resource.resource_id.clone());
                resources
                    .entry(key)
                    .or_default()
                    .persisted
                    .push(PersistedResourceLease {
                        lease_id: lease.lease_id.clone(),
                        owner: lease.owner.clone(),
                        state: lease.state,
                    });
            }
        }

        for observed in self.observed.resources() {
            let key = ResourceKey::new(observed.kind, observed.resource_id.clone());
            resources.entry(key).or_default().observed = Some(observed.clone());
        }

        let mut actions = Vec::new();
        for (key, pair) in resources {
            let observed = pair.observed.unwrap_or_else(|| ObservedHostResource {
                resource_id: key.resource_id.clone(),
                kind: key.kind,
                source: ResourceObservationSource::AllocatorLedger,
                state: ObservedResourceState::Missing,
            });
            let persisted = pair.persisted.first().cloned();
            let decision = self.reconciliation_decision(&pair.persisted, &observed);
            actions.push(AllocatorReconciliationAction {
                resource_id: key.resource_id,
                kind: key.kind,
                persisted,
                observed,
                decision,
            });
        }

        let records = actions
            .iter()
            .take(MAX_RECONCILIATION_RECORDS)
            .map(|action| ReconciliationRecord {
                resource_id: action.resource_id.clone(),
                kind: action.kind,
                persisted: action.persisted.clone(),
                observed: action.observed.clone(),
                decision: action.decision.reconciliation_decision(),
            })
            .collect::<Vec<_>>();

        let events = bounded_events(actions.iter().map(|action| {
            let owner = action
                .persisted
                .as_ref()
                .map(|persisted| persisted.owner.clone())
                .unwrap_or_else(|| self.allocator_owner.clone());
            event_metadata(
                operation_id.clone(),
                correlation_id.clone(),
                owner,
                &action.decision,
                Some(action.kind),
                None,
            )
        }));
        let metrics = bounded_metrics(
            actions
                .iter()
                .map(|action| metric_from_decision(&action.decision, Some(action.kind))),
        );
        let report = ReconciliationReport {
            operation_id,
            correlation_id,
            records,
            events,
        };

        Ok(AllocatorEngineReconciliation {
            report,
            actions,
            metrics,
        })
    }

    fn allocation_conflicts(
        &self,
        request: &LeaseAllocationRequest,
        leases: &[AllocatorLease],
    ) -> Vec<AllocatorConflict> {
        let mut conflicts = Vec::new();
        for resource in request.acquisition_order() {
            if let Some(conflict) = self.persisted_conflict(leases, &request.owner, &resource) {
                conflicts.push(conflict);
            }
            if let Some(conflict) = self.observed_conflict(&resource) {
                conflicts.push(conflict);
            }
            if conflicts.len() >= MAX_ALLOCATOR_CONFLICTS {
                break;
            }
        }
        conflicts
    }

    fn persisted_conflict(
        &self,
        leases: &[AllocatorLease],
        owner: &LeaseOwner,
        resource: &ResourceAcquisitionKey,
    ) -> Option<AllocatorConflict> {
        leases
            .iter()
            .filter(|lease| !lease.state.is_terminal())
            .find_map(|lease| {
                lease
                    .resources
                    .iter()
                    .any(|granted| {
                        granted.kind == resource.kind && granted.resource_id == resource.resource_id
                    })
                    .then(|| AllocatorConflict {
                        resource_id: resource.resource_id.clone(),
                        kind: resource.kind,
                        reason: if &lease.owner == owner {
                            AllocatorReasonCode::OwnershipConflict
                        } else {
                            AllocatorReasonCode::ResourceConflict
                        },
                        existing_lease: Some(lease.lease_id.clone()),
                    })
            })
    }

    fn observed_conflict(&self, resource: &ResourceAcquisitionKey) -> Option<AllocatorConflict> {
        self.observed
            .resources()
            .iter()
            .find(|observed| {
                observed.kind == resource.kind && observed.resource_id == resource.resource_id
            })
            .and_then(|observed| match observed.state {
                ObservedResourceState::Missing => None,
                ObservedResourceState::Present => Some(AllocatorReasonCode::ResourceConflict),
                ObservedResourceState::ForeignOwner => Some(AllocatorReasonCode::OwnershipConflict),
                ObservedResourceState::Ambiguous | ObservedResourceState::Inaccessible => {
                    Some(AllocatorReasonCode::KernelStateUnknown)
                }
            })
            .map(|reason| AllocatorConflict {
                resource_id: resource.resource_id.clone(),
                kind: resource.kind,
                reason,
                existing_lease: None,
            })
    }

    fn reconciliation_decision(
        &self,
        persisted: &[PersistedResourceLease],
        observed: &ObservedHostResource,
    ) -> AllocatorEngineDecision {
        if persisted.len() > 1 {
            return AllocatorEngineDecision::Quarantine {
                reason: AllocatorReasonCode::StorageContractViolation,
            };
        }

        let Some(persisted) = persisted.first() else {
            return match observed.state {
                ObservedResourceState::Missing => AllocatorEngineDecision::Reconcile,
                ObservedResourceState::Present => AllocatorEngineDecision::Preserve {
                    reason: AllocatorReasonCode::ResourceConflict,
                },
                ObservedResourceState::ForeignOwner => AllocatorEngineDecision::Preserve {
                    reason: AllocatorReasonCode::OwnershipConflict,
                },
                ObservedResourceState::Ambiguous | ObservedResourceState::Inaccessible => {
                    AllocatorEngineDecision::Quarantine {
                        reason: AllocatorReasonCode::KernelStateUnknown,
                    }
                }
            };
        };

        if persisted.state.is_terminal() {
            return match observed.state {
                ObservedResourceState::Missing => AllocatorEngineDecision::Reconcile,
                ObservedResourceState::Present | ObservedResourceState::ForeignOwner => {
                    AllocatorEngineDecision::Preserve {
                        reason: AllocatorReasonCode::OwnershipConflict,
                    }
                }
                ObservedResourceState::Ambiguous | ObservedResourceState::Inaccessible => {
                    AllocatorEngineDecision::Quarantine {
                        reason: AllocatorReasonCode::KernelStateUnknown,
                    }
                }
            };
        }

        if !self.liveness.is_live(&persisted.owner) {
            return AllocatorEngineDecision::Reclaim {
                reason: AllocatorReasonCode::OwnerNotLive,
            };
        }

        match observed.state {
            ObservedResourceState::Present => AllocatorEngineDecision::Reconcile,
            ObservedResourceState::Missing => AllocatorEngineDecision::Reclaim {
                reason: AllocatorReasonCode::DriftDetected,
            },
            ObservedResourceState::ForeignOwner => AllocatorEngineDecision::Quarantine {
                reason: AllocatorReasonCode::DriftDetected,
            },
            ObservedResourceState::Ambiguous | ObservedResourceState::Inaccessible => {
                AllocatorEngineDecision::Quarantine {
                    reason: AllocatorReasonCode::KernelStateUnknown,
                }
            }
        }
    }

    fn deny_request(
        &mut self,
        snapshot: &AllocatorLedgerSnapshot,
        request: &LeaseAllocationRequest,
        acquired: Vec<ResourceAcquisitionKey>,
        reason: AllocatorReasonCode,
        conflicts: Vec<AllocatorConflict>,
        persist: bool,
    ) -> Result<AllocatorEngineAllocation, AllocatorEngineError> {
        let conflicts = conflicts
            .into_iter()
            .take(MAX_ALLOCATOR_CONFLICTS)
            .collect::<Vec<_>>();
        let decisions = if conflicts.is_empty() {
            request
                .resources
                .iter()
                .map(|resource| AllocatorAllocationDecision {
                    resource_id: resource.resource_id.clone(),
                    kind: resource.kind,
                    decision: AllocatorEngineDecision::DenyConflict { reason },
                })
                .collect::<Vec<_>>()
        } else {
            conflicts
                .iter()
                .map(|conflict| AllocatorAllocationDecision {
                    resource_id: conflict.resource_id.clone(),
                    kind: conflict.kind,
                    decision: AllocatorEngineDecision::DenyConflict {
                        reason: conflict.reason,
                    },
                })
                .collect::<Vec<_>>()
        };
        let events = bounded_events(decisions.iter().map(|decision| {
            event_metadata(
                request.operation_id.clone(),
                request.correlation_id.clone(),
                request.owner.clone(),
                &decision.decision,
                Some(decision.kind),
                request.trace.clone(),
            )
        }));
        let metrics = bounded_metrics(
            decisions
                .iter()
                .map(|decision| metric_from_decision(&decision.decision, Some(decision.kind))),
        );
        let result = LeaseAllocationResult::Denied { reason, conflicts };
        if persist {
            let commit = AllocatorLedgerCommit {
                expected_generation: snapshot.generation(),
                request: request.clone(),
                decision: match &result {
                    LeaseAllocationResult::Denied { reason, conflicts } => {
                        PendingAllocationDecision::Denial {
                            reason: *reason,
                            conflicts: conflicts.clone(),
                        }
                    }
                    LeaseAllocationResult::Granted { .. } => unreachable!(),
                },
            };
            let committed = self.ledger.commit_allocation(commit)?;
            if committed.result() != &result
                || committed.idempotency_record()
                    != &AllocatorIdempotencyRecord::from_request(request, result.clone())
            {
                return Err(AllocatorEngineError::LedgerTampered);
            }
        }
        let response = LeaseAllocationResponse {
            operation_id: request.operation_id.clone(),
            correlation_id: request.correlation_id.clone(),
            result,
        };
        Ok(AllocatorEngineAllocation {
            response,
            decisions,
            events,
            metrics,
            acquired,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ResourceKey {
    kind: HostResourceKind,
    resource_id: HostResourceId,
}

impl ResourceKey {
    fn new(kind: HostResourceKind, resource_id: HostResourceId) -> Self {
        Self { kind, resource_id }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResourcePair {
    persisted: Vec<PersistedResourceLease>,
    observed: Option<ObservedHostResource>,
}

fn validate_snapshot(snapshot: &AllocatorLedgerSnapshot) -> Result<(), AllocatorEngineError> {
    let mut lease_ids = BTreeSet::new();
    if snapshot
        .leases
        .iter()
        .any(|lease| !lease_ids.insert(lease.lease_id.clone()))
    {
        return Err(AllocatorEngineError::LedgerTampered);
    }

    let mut idempotency_keys = BTreeSet::new();
    for record in &snapshot.idempotency {
        if !idempotency_keys.insert(record.key.clone()) {
            return Err(AllocatorEngineError::LedgerTampered);
        }
        if let LeaseAllocationResult::Granted { lease } = &record.result {
            let Some(persisted) = snapshot
                .leases
                .iter()
                .find(|candidate| candidate.lease_id == lease.lease_id)
            else {
                return Err(AllocatorEngineError::LedgerTampered);
            };
            if persisted != lease {
                return Err(AllocatorEngineError::LedgerTampered);
            }
            let mut granted_signature = lease
                .resources
                .iter()
                .map(|resource| RequestedResourceSignature {
                    key: ResourceAcquisitionKey {
                        order: resource.acquisition_order,
                        kind: resource.kind,
                        resource_id: resource.resource_id.clone(),
                    },
                    share: resource.share,
                })
                .collect::<Vec<_>>();
            granted_signature.sort();
            if record.signature.owner != lease.owner
                || record.signature.resources != granted_signature
            {
                return Err(AllocatorEngineError::LedgerTampered);
            }
        }
    }
    Ok(())
}

fn has_duplicate_requested_resources(request: &LeaseAllocationRequest) -> bool {
    let mut seen = BTreeSet::new();
    request
        .resources
        .iter()
        .any(|resource| !seen.insert(resource.resource_id.clone()))
}

fn grant_resources_in_order(request: &LeaseAllocationRequest) -> Vec<GrantedHostResource> {
    let by_id = request
        .resources
        .iter()
        .map(|resource| (resource.resource_id.clone(), resource))
        .collect::<BTreeMap<_, _>>();
    request
        .acquisition_order()
        .into_iter()
        .filter_map(|key| {
            by_id
                .get(&key.resource_id)
                .map(|resource| GrantedHostResource {
                    resource_id: resource.resource_id.clone(),
                    kind: resource.kind,
                    share: resource.share,
                    delegation: delegation_for(
                        resource.kind,
                        resource.share,
                        &resource.resource_id,
                    ),
                    acquisition_order: resource.acquisition_order,
                })
        })
        .collect()
}

fn delegation_for(
    kind: HostResourceKind,
    share: ResourceShareMode,
    resource_id: &HostResourceId,
) -> ResourceDelegation {
    if share == ResourceShareMode::SharedPartition {
        return ResourceDelegation::PartitionId {
            id: resource_id.clone(),
        };
    }
    match kind {
        HostResourceKind::NftablesPartition | HostResourceKind::HostFilePartition => {
            ResourceDelegation::PartitionId {
                id: resource_id.clone(),
            }
        }
        HostResourceKind::NamespaceBoundary => ResourceDelegation::NamespaceHandle {
            id: resource_id.clone(),
        },
        HostResourceKind::Bridge
        | HostResourceKind::Tap
        | HostResourceKind::VethPair
        | HostResourceKind::NftablesTable
        | HostResourceKind::CgroupSubtree => ResourceDelegation::OpaqueName {
            id: resource_id.clone(),
        },
    }
}

fn share_rank(share: ResourceShareMode) -> u8 {
    match share {
        ResourceShareMode::Exclusive => 0,
        ResourceShareMode::SharedPartition => 1,
    }
}

fn event_metadata(
    operation_id: OperationId,
    correlation_id: CorrelationId,
    owner: LeaseOwner,
    decision: &AllocatorEngineDecision,
    resource_kind: Option<HostResourceKind>,
    trace: Option<crate::trace_context::TraceContext>,
) -> AllocatorEventMetadata {
    AllocatorEventMetadata {
        operation_id,
        correlation_id,
        owner,
        event: decision.event_kind(),
        resource_kind,
        reason: decision.reason(),
        trace,
    }
}

fn metric_from_decision(
    decision: &AllocatorEngineDecision,
    resource_kind: Option<HostResourceKind>,
) -> AllocatorMetricEvent {
    AllocatorMetricEvent::new(
        decision.event_kind(),
        decision.outcome(),
        resource_kind,
        decision.reason(),
    )
}

fn metric_from_replay_result(result: &LeaseAllocationResult) -> AllocatorMetricEvent {
    match result {
        LeaseAllocationResult::Granted { lease } => AllocatorMetricEvent::new(
            AllocatorEventKind::Grant,
            AllocatorEngineOutcome::IdempotentReplay,
            lease.resources.first().map(|resource| resource.kind),
            None,
        ),
        LeaseAllocationResult::Denied { reason, conflicts } => AllocatorMetricEvent::new(
            AllocatorEventKind::Denial,
            AllocatorEngineOutcome::IdempotentReplay,
            conflicts.first().map(|conflict| conflict.kind),
            Some(*reason),
        ),
    }
}

fn bounded_events(
    events: impl IntoIterator<Item = AllocatorEventMetadata>,
) -> Vec<AllocatorEventMetadata> {
    events.into_iter().take(MAX_ALLOCATOR_EVENTS).collect()
}

fn bounded_metrics(
    metrics: impl IntoIterator<Item = AllocatorMetricEvent>,
) -> Vec<AllocatorMetricEvent> {
    let mut aggregated = BTreeMap::<AllocatorMetricLabels, AllocatorMetricEvent>::new();
    for metric in metrics {
        aggregated
            .entry(metric.labels())
            .and_modify(|existing| {
                existing.count = existing.count.saturating_add(metric.count);
            })
            .or_insert(metric);
    }
    aggregated
        .into_values()
        .take(MAX_ALLOCATOR_EVENTS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocator::{
        MAX_ALLOCATOR_REQUEST_RESOURCES, MAX_RECONCILIATION_RECORDS, ResourceAcquisitionOrder,
        ResourceShareMode,
    };
    use crate::ids::{ControllerGenerationId, NodeId, RealmId};
    use crate::realm::RealmPath;
    use std::sync::{Arc, Mutex};

    fn id(raw: &str) -> HostResourceId {
        HostResourceId::parse(raw).unwrap()
    }

    fn lease_id(raw: &str) -> AllocatorLeaseId {
        AllocatorLeaseId::parse(raw).unwrap()
    }

    fn op(raw: &str) -> OperationId {
        OperationId::parse(raw).unwrap()
    }

    fn corr(raw: &str) -> CorrelationId {
        CorrelationId::parse(raw).unwrap()
    }

    fn idem(raw: &str) -> IdempotencyKey {
        IdempotencyKey::parse(raw).unwrap()
    }

    fn owner(raw: &str) -> LeaseOwner {
        LeaseOwner {
            realm: RealmPath::new(vec![RealmId::parse(raw).unwrap()]).unwrap(),
            controller_generation: ControllerGenerationId::parse("gen-1").unwrap(),
            node: Some(NodeId::parse("realm-node").unwrap()),
        }
    }

    fn request_resource(
        raw: &str,
        kind: HostResourceKind,
        phase: u16,
        ordinal: u16,
    ) -> crate::allocator::LeaseResourceRequest {
        crate::allocator::LeaseResourceRequest {
            resource_id: id(raw),
            kind,
            share: ResourceShareMode::Exclusive,
            acquisition_order: ResourceAcquisitionOrder { phase, ordinal },
        }
    }

    fn request(
        operation_id: &str,
        correlation_id: &str,
        idempotency_key: &str,
        owner: LeaseOwner,
        resources: Vec<crate::allocator::LeaseResourceRequest>,
    ) -> LeaseAllocationRequest {
        LeaseAllocationRequest {
            operation_id: op(operation_id),
            correlation_id: corr(correlation_id),
            idempotency_key: idem(idempotency_key),
            owner,
            resources,
            trace: None,
        }
    }

    fn granted(raw: &str, kind: HostResourceKind) -> GrantedHostResource {
        GrantedHostResource {
            resource_id: id(raw),
            kind,
            share: ResourceShareMode::Exclusive,
            delegation: ResourceDelegation::OpaqueName { id: id(raw) },
            acquisition_order: ResourceAcquisitionOrder {
                phase: 1,
                ordinal: 0,
            },
        }
    }

    fn lease(
        raw: &str,
        owner: LeaseOwner,
        state: AllocatorLeaseState,
        resources: Vec<GrantedHostResource>,
    ) -> AllocatorLease {
        AllocatorLease {
            lease_id: lease_id(raw),
            owner,
            state,
            resources,
        }
    }

    fn observed(
        raw: &str,
        kind: HostResourceKind,
        state: ObservedResourceState,
    ) -> ObservedHostResource {
        ObservedHostResource {
            resource_id: id(raw),
            kind,
            source: ResourceObservationSource::KernelNetlink,
            state,
        }
    }

    #[derive(Debug, Default)]
    struct CustomLedger {
        leases: Vec<AllocatorLease>,
        idempotency: Vec<AllocatorIdempotencyRecord>,
        next_sequence: u64,
        generation: AllocatorLedgerGeneration,
    }

    impl CustomLedger {
        fn with_next_sequence(next_sequence: u64) -> Self {
            Self {
                next_sequence,
                ..Self::default()
            }
        }
    }

    impl AllocatorLedger for CustomLedger {
        fn load(&self) -> Result<AllocatorLedgerSnapshot, AllocatorEngineError> {
            Ok(AllocatorLedgerSnapshot::new(
                self.generation,
                self.leases.clone(),
                self.idempotency.clone(),
            ))
        }

        fn commit_allocation(
            &mut self,
            commit: AllocatorLedgerCommit,
        ) -> Result<AllocatorLedgerCommitResult, AllocatorEngineError> {
            if commit.expected_generation() != self.generation {
                return Err(AllocatorEngineError::LedgerGenerationConflict);
            }
            let lease_id = (commit.kind() == AllocatorLedgerCommitKind::Grant).then(|| {
                AllocatorLeaseId::parse(format!("lease-custom-{}", self.next_sequence))
                    .expect("custom test lease id")
            });
            let committed = commit.materialize(lease_id)?;
            let mut leases = self.leases.clone();
            if let LeaseAllocationResult::Granted { lease } = committed.result() {
                leases.push(lease.clone());
            }
            let mut idempotency = self.idempotency.clone();
            idempotency.push(committed.idempotency_record().clone());
            let generation = self.generation.checked_next()?;

            self.leases = leases;
            self.idempotency = idempotency;
            self.generation = generation;
            if matches!(committed.result(), LeaseAllocationResult::Granted { .. }) {
                self.next_sequence += 1;
            }
            Ok(committed)
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CommitFailureStage {
        AcquireLock,
        ReserveLeaseId,
        WriteLease,
        WriteIdempotency,
        DurableSync,
        AfterDurableCommit,
    }

    #[derive(Debug)]
    struct FaultInjectingLedger {
        leases: Vec<AllocatorLease>,
        idempotency: Vec<AllocatorIdempotencyRecord>,
        next_sequence: u64,
        generation: AllocatorLedgerGeneration,
        failure: Option<CommitFailureStage>,
    }

    impl FaultInjectingLedger {
        fn new(failure: CommitFailureStage) -> Self {
            Self {
                leases: Vec::new(),
                idempotency: Vec::new(),
                next_sequence: 1,
                generation: AllocatorLedgerGeneration::default(),
                failure: Some(failure),
            }
        }

        fn fail_at(&mut self, stage: CommitFailureStage) -> bool {
            if self.failure == Some(stage) {
                self.failure = None;
                true
            } else {
                false
            }
        }
    }

    impl AllocatorLedger for FaultInjectingLedger {
        fn load(&self) -> Result<AllocatorLedgerSnapshot, AllocatorEngineError> {
            Ok(AllocatorLedgerSnapshot::new(
                self.generation,
                self.leases.clone(),
                self.idempotency.clone(),
            ))
        }

        fn commit_allocation(
            &mut self,
            commit: AllocatorLedgerCommit,
        ) -> Result<AllocatorLedgerCommitResult, AllocatorEngineError> {
            if self.fail_at(CommitFailureStage::AcquireLock) {
                return Err(AllocatorEngineError::LedgerLockUnavailable);
            }
            if commit.expected_generation() != self.generation {
                return Err(AllocatorEngineError::LedgerGenerationConflict);
            }

            let mut next_sequence = self.next_sequence;
            let mut leases = self.leases.clone();
            let mut idempotency = self.idempotency.clone();
            let lease_id = if commit.kind() == AllocatorLedgerCommitKind::Grant {
                let lease_id =
                    AllocatorLeaseId::parse(format!("lease-fault-{next_sequence}")).unwrap();
                next_sequence += 1;
                if self.fail_at(CommitFailureStage::ReserveLeaseId) {
                    return Err(AllocatorEngineError::LedgerIo);
                }
                Some(lease_id)
            } else {
                None
            };
            let committed = commit.materialize(lease_id)?;
            if let LeaseAllocationResult::Granted { lease } = committed.result() {
                leases.push(lease.clone());
            }
            if self.fail_at(CommitFailureStage::WriteLease) {
                return Err(AllocatorEngineError::LedgerIo);
            }
            idempotency.push(committed.idempotency_record().clone());
            if self.fail_at(CommitFailureStage::WriteIdempotency) {
                return Err(AllocatorEngineError::LedgerIo);
            }
            let generation = self.generation.checked_next()?;
            if self.fail_at(CommitFailureStage::DurableSync) {
                return Err(AllocatorEngineError::LedgerIo);
            }

            self.leases = leases;
            self.idempotency = idempotency;
            self.next_sequence = next_sequence;
            self.generation = generation;
            if self.fail_at(CommitFailureStage::AfterDurableCommit) {
                return Err(AllocatorEngineError::LedgerIo);
            }
            Ok(committed)
        }
    }

    #[derive(Debug)]
    struct ErrorLedger(AllocatorEngineError);

    impl AllocatorLedger for ErrorLedger {
        fn load(&self) -> Result<AllocatorLedgerSnapshot, AllocatorEngineError> {
            Err(self.0)
        }

        fn commit_allocation(
            &mut self,
            _commit: AllocatorLedgerCommit,
        ) -> Result<AllocatorLedgerCommitResult, AllocatorEngineError> {
            panic!("commit must not run after a failed read")
        }
    }

    #[derive(Debug, Default)]
    struct SharedLedgerState {
        leases: Vec<AllocatorLease>,
        idempotency: Vec<AllocatorIdempotencyRecord>,
        next_sequence: u64,
        generation: AllocatorLedgerGeneration,
    }

    #[derive(Debug)]
    struct OfdStyleLedger {
        shared: Arc<Mutex<SharedLedgerState>>,
        stale_read: Option<AllocatorLedgerSnapshot>,
    }

    impl OfdStyleLedger {
        fn new(shared: Arc<Mutex<SharedLedgerState>>) -> Self {
            Self {
                shared,
                stale_read: None,
            }
        }

        fn with_stale_read(
            shared: Arc<Mutex<SharedLedgerState>>,
            stale_read: AllocatorLedgerSnapshot,
        ) -> Self {
            Self {
                shared,
                stale_read: Some(stale_read),
            }
        }
    }

    impl AllocatorLedger for OfdStyleLedger {
        fn load(&self) -> Result<AllocatorLedgerSnapshot, AllocatorEngineError> {
            if let Some(snapshot) = &self.stale_read {
                return Ok(snapshot.clone());
            }
            let state = self
                .shared
                .lock()
                .map_err(|_| AllocatorEngineError::LedgerLockUnavailable)?;
            Ok(AllocatorLedgerSnapshot::new(
                state.generation,
                state.leases.clone(),
                state.idempotency.clone(),
            ))
        }

        fn commit_allocation(
            &mut self,
            commit: AllocatorLedgerCommit,
        ) -> Result<AllocatorLedgerCommitResult, AllocatorEngineError> {
            let mut state = self
                .shared
                .lock()
                .map_err(|_| AllocatorEngineError::LedgerLockUnavailable)?;
            if commit.expected_generation() != state.generation {
                return Err(AllocatorEngineError::LedgerGenerationConflict);
            }
            let next_sequence = if commit.kind() == AllocatorLedgerCommitKind::Grant {
                Some(
                    state
                        .next_sequence
                        .checked_add(1)
                        .ok_or(AllocatorEngineError::LedgerTampered)?,
                )
            } else {
                None
            };
            let lease_id = next_sequence.map(|sequence| {
                AllocatorLeaseId::parse(format!("lease-shared-{sequence}")).unwrap()
            });
            let committed = commit.materialize(lease_id)?;
            let mut leases = state.leases.clone();
            if let LeaseAllocationResult::Granted { lease } = committed.result() {
                leases.push(lease.clone());
            }
            let mut idempotency = state.idempotency.clone();
            idempotency.push(committed.idempotency_record().clone());
            let generation = state.generation.checked_next()?;
            state.leases = leases;
            state.idempotency = idempotency;
            state.generation = generation;
            if let Some(next_sequence) = next_sequence {
                state.next_sequence = next_sequence;
            }
            Ok(committed)
        }
    }

    #[derive(Debug, Default)]
    struct CustomObserved {
        resources: Vec<ObservedHostResource>,
    }

    impl ObservedAllocatorState for CustomObserved {
        fn resources(&self) -> &[ObservedHostResource] {
            &self.resources
        }
    }

    #[derive(Debug, Default)]
    struct CustomLiveness {
        live: Vec<LeaseOwner>,
    }

    impl AllocatorLiveness for CustomLiveness {
        fn is_live(&self, owner: &LeaseOwner) -> bool {
            self.live.iter().any(|candidate| candidate == owner)
        }
    }

    type FakeEngine = LocalRootAllocatorEngine<
        FakeAllocatorLedger,
        FakeObservedAllocatorState,
        FakeAllocatorLiveness,
    >;

    fn engine(
        owner: LeaseOwner,
        leases: Vec<AllocatorLease>,
        observations: Vec<ObservedHostResource>,
        live_owners: Vec<LeaseOwner>,
    ) -> FakeEngine {
        LocalRootAllocatorEngine::new(
            owner,
            FakeAllocatorLedger::new(leases),
            FakeObservedAllocatorState::new(observations),
            FakeAllocatorLiveness::new(live_owners),
        )
    }

    #[test]
    fn fake_support_adapters_conform_and_engine_is_send_sync() {
        fn assert_ledger<T: AllocatorLedger>() {}
        fn assert_observed<T: ObservedAllocatorState>() {}
        fn assert_liveness<T: AllocatorLiveness>() {}
        fn assert_send_sync<T: Send + Sync>() {}

        assert_ledger::<FakeAllocatorLedger>();
        assert_observed::<FakeObservedAllocatorState>();
        assert_liveness::<FakeAllocatorLiveness>();
        assert_send_sync::<FakeEngine>();
        assert_send_sync::<LocalRootAllocatorEngine<CustomLedger, CustomObserved, CustomLiveness>>(
        );
    }

    #[test]
    fn commit_failures_before_durability_leave_no_partial_state_and_retry_cleanly() {
        let failure_stages = [
            CommitFailureStage::AcquireLock,
            CommitFailureStage::ReserveLeaseId,
            CommitFailureStage::WriteLease,
            CommitFailureStage::WriteIdempotency,
            CommitFailureStage::DurableSync,
        ];

        for stage in failure_stages {
            let lease_owner = owner("work");
            let allocation_request = request(
                "op-1",
                "corr-1",
                "idem-1",
                lease_owner.clone(),
                vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
            );
            let mut engine = LocalRootAllocatorEngine::new(
                owner("root"),
                FaultInjectingLedger::new(stage),
                CustomObserved::default(),
                CustomLiveness {
                    live: vec![lease_owner],
                },
            );

            let error = engine.allocate(allocation_request.clone()).unwrap_err();
            assert_eq!(
                error,
                if stage == CommitFailureStage::AcquireLock {
                    AllocatorEngineError::LedgerLockUnavailable
                } else {
                    AllocatorEngineError::LedgerIo
                }
            );
            assert!(engine.ledger().leases.is_empty());
            assert!(engine.ledger().idempotency.is_empty());
            assert_eq!(engine.ledger().next_sequence, 1);
            assert_eq!(
                engine.ledger().generation,
                AllocatorLedgerGeneration::default()
            );

            let retry = engine.allocate(allocation_request).unwrap();
            let LeaseAllocationResult::Granted { lease } = retry.response.result else {
                panic!("retry after rolled-back commit must grant");
            };
            assert_eq!(lease.lease_id, lease_id("lease-fault-1"));
            assert_eq!(engine.ledger().leases.len(), 1);
            assert_eq!(engine.ledger().idempotency.len(), 1);
        }
    }

    #[test]
    fn lost_commit_ack_returns_error_then_replays_the_durable_grant() {
        let lease_owner = owner("work");
        let allocation_request = request(
            "op-1",
            "corr-1",
            "idem-1",
            lease_owner.clone(),
            vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
        );
        let mut engine = LocalRootAllocatorEngine::new(
            owner("root"),
            FaultInjectingLedger::new(CommitFailureStage::AfterDurableCommit),
            CustomObserved::default(),
            CustomLiveness {
                live: vec![lease_owner],
            },
        );

        assert_eq!(
            engine.allocate(allocation_request.clone()).unwrap_err(),
            AllocatorEngineError::LedgerIo
        );
        assert_eq!(engine.ledger().leases.len(), 1);
        assert_eq!(engine.ledger().idempotency.len(), 1);
        assert_eq!(engine.ledger().next_sequence, 2);
        assert_eq!(
            engine.ledger().generation,
            AllocatorLedgerGeneration::new(1)
        );

        let replay = engine.allocate(allocation_request).unwrap();
        let LeaseAllocationResult::Granted { lease } = replay.response.result else {
            panic!("retry must replay the durably committed grant");
        };
        assert_eq!(lease.lease_id, lease_id("lease-fault-1"));
        assert_eq!(
            replay.metrics[0].outcome,
            AllocatorEngineOutcome::IdempotentReplay
        );
        assert_eq!(engine.ledger().leases.len(), 1);
    }

    #[test]
    fn denial_is_not_returned_until_its_idempotency_record_is_durable() {
        let lease_owner = owner("work");
        let allocation_request = request(
            "op-1",
            "corr-1",
            "idem-1",
            lease_owner.clone(),
            vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
        );
        let mut engine = LocalRootAllocatorEngine::new(
            owner("root"),
            FaultInjectingLedger::new(CommitFailureStage::WriteIdempotency),
            CustomObserved {
                resources: vec![observed(
                    "bridge-1",
                    HostResourceKind::Bridge,
                    ObservedResourceState::ForeignOwner,
                )],
            },
            CustomLiveness {
                live: vec![lease_owner],
            },
        );

        assert_eq!(
            engine.allocate(allocation_request.clone()).unwrap_err(),
            AllocatorEngineError::LedgerIo
        );
        assert!(engine.ledger().idempotency.is_empty());
        assert_eq!(
            engine
                .allocate(allocation_request.clone())
                .unwrap()
                .response
                .result,
            engine.allocate(allocation_request).unwrap().response.result
        );
        assert_eq!(engine.ledger().idempotency.len(), 1);
    }

    #[test]
    fn fallible_reads_propagate_closed_errors_without_a_response() {
        for error in [
            AllocatorEngineError::LedgerLockUnavailable,
            AllocatorEngineError::LedgerIo,
            AllocatorEngineError::LedgerGenerationConflict,
            AllocatorEngineError::LedgerTampered,
        ] {
            let lease_owner = owner("work");
            let allocation_request = request(
                "op-1",
                "corr-1",
                "idem-1",
                lease_owner.clone(),
                vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
            );
            let mut engine = LocalRootAllocatorEngine::new(
                owner("root"),
                ErrorLedger(error),
                CustomObserved::default(),
                CustomLiveness {
                    live: vec![lease_owner],
                },
            );
            assert_eq!(engine.allocate(allocation_request).unwrap_err(), error);
            assert!(!error.to_string().contains("bridge"));
        }

        let engine = LocalRootAllocatorEngine::new(
            owner("root"),
            ErrorLedger(AllocatorEngineError::LedgerIo),
            CustomObserved::default(),
            CustomLiveness::default(),
        );
        assert_eq!(
            engine.reconcile(op("op-1"), corr("corr-1")).unwrap_err(),
            AllocatorEngineError::LedgerIo
        );
    }

    #[test]
    fn snapshot_tamper_is_rejected_before_commit() {
        let lease_owner = owner("work");
        let missing_lease = lease(
            "lease-missing",
            lease_owner.clone(),
            AllocatorLeaseState::Granted,
            vec![granted("bridge-1", HostResourceKind::Bridge)],
        );
        let allocation_request = request(
            "op-1",
            "corr-1",
            "idem-1",
            lease_owner.clone(),
            vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
        );
        let record = AllocatorIdempotencyRecord::from_request(
            &allocation_request,
            LeaseAllocationResult::Granted {
                lease: missing_lease,
            },
        );
        let mut ledger = CustomLedger::default();
        ledger.idempotency.push(record);
        let mut engine = LocalRootAllocatorEngine::new(
            owner("root"),
            ledger,
            CustomObserved::default(),
            CustomLiveness {
                live: vec![lease_owner],
            },
        );

        assert_eq!(
            engine.allocate(allocation_request).unwrap_err(),
            AllocatorEngineError::LedgerTampered
        );
        assert!(engine.ledger().leases.is_empty());
    }

    #[test]
    fn idempotency_record_serializes_for_durable_restart_replay() {
        let allocation_request = request(
            "op-1",
            "corr-1",
            "idem-1",
            owner("work"),
            vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
        );
        let result = LeaseAllocationResult::Denied {
            reason: AllocatorReasonCode::PolicyDenied,
            conflicts: Vec::new(),
        };
        let record = AllocatorIdempotencyRecord::from_request(&allocation_request, result.clone());

        let encoded = serde_json::to_string(&record).unwrap();
        let decoded: AllocatorIdempotencyRecord = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, record);
        assert_eq!(decoded.result(), &result);
        assert!(
            serde_json::from_str::<AllocatorIdempotencyRecord>(&encoded.replacen(
                '{',
                "{\"unexpected\":true,",
                1
            ))
            .is_err()
        );
    }

    #[test]
    fn ofd_style_stale_generation_conflict_never_returns_granted() {
        let shared = Arc::new(Mutex::new(SharedLedgerState::default()));
        let stale_snapshot = OfdStyleLedger::new(shared.clone()).load().unwrap();
        let lease_owner = owner("work");
        let live = CustomLiveness {
            live: vec![lease_owner.clone()],
        };
        let mut first = LocalRootAllocatorEngine::new(
            owner("root"),
            OfdStyleLedger::with_stale_read(shared.clone(), stale_snapshot.clone()),
            CustomObserved::default(),
            CustomLiveness {
                live: live.live.clone(),
            },
        );
        let mut concurrent = LocalRootAllocatorEngine::new(
            owner("root"),
            OfdStyleLedger::with_stale_read(shared.clone(), stale_snapshot),
            CustomObserved::default(),
            live,
        );

        first
            .allocate(request(
                "op-1",
                "corr-1",
                "idem-1",
                lease_owner.clone(),
                vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
            ))
            .unwrap();
        let error = concurrent
            .allocate(request(
                "op-2",
                "corr-2",
                "idem-2",
                lease_owner.clone(),
                vec![request_resource("bridge-2", HostResourceKind::Bridge, 1, 0)],
            ))
            .unwrap_err();
        assert_eq!(error, AllocatorEngineError::LedgerGenerationConflict);
        assert_eq!(shared.lock().unwrap().leases.len(), 1);

        let mut retry = LocalRootAllocatorEngine::new(
            owner("root"),
            OfdStyleLedger::new(shared.clone()),
            CustomObserved::default(),
            CustomLiveness {
                live: vec![lease_owner.clone()],
            },
        );
        let allocation = retry
            .allocate(request(
                "op-3",
                "corr-3",
                "idem-2",
                lease_owner,
                vec![request_resource("bridge-2", HostResourceKind::Bridge, 1, 0)],
            ))
            .unwrap();
        assert!(matches!(
            allocation.response.result,
            LeaseAllocationResult::Granted { .. }
        ));
        assert_eq!(shared.lock().unwrap().leases.len(), 2);
    }

    #[test]
    fn custom_adapters_preserve_replay_generation_and_restart_semantics() {
        let allocator_owner = owner("root");
        let lease_owner = owner("work");
        let initial = request(
            "op-1",
            "corr-1",
            "idem-1",
            lease_owner.clone(),
            vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
        );
        let mut engine = LocalRootAllocatorEngine::new(
            allocator_owner.clone(),
            CustomLedger::with_next_sequence(41),
            CustomObserved::default(),
            CustomLiveness {
                live: vec![lease_owner.clone()],
            },
        );

        let first = engine.allocate(initial.clone()).unwrap();
        let LeaseAllocationResult::Granted { lease } = &first.response.result else {
            panic!("expected custom adapter grant");
        };
        assert_eq!(lease.lease_id, lease_id("lease-custom-41"));

        let (ledger, _, _) = engine.into_adapters();
        let mut restarted = LocalRootAllocatorEngine::new(
            allocator_owner,
            ledger,
            CustomObserved {
                resources: vec![observed(
                    "bridge-1",
                    HostResourceKind::Bridge,
                    ObservedResourceState::Present,
                )],
            },
            CustomLiveness {
                live: vec![lease_owner.clone()],
            },
        );
        let mut replay_request = initial.clone();
        replay_request.operation_id = op("op-2");
        replay_request.correlation_id = corr("corr-2");
        let replay = restarted.allocate(replay_request).unwrap();
        assert_eq!(replay.response.result, first.response.result);
        assert_eq!(
            replay.metrics[0].outcome,
            AllocatorEngineOutcome::IdempotentReplay
        );
        let next = restarted
            .allocate(request(
                "op-next",
                "corr-next",
                "idem-next",
                lease_owner.clone(),
                vec![request_resource("bridge-2", HostResourceKind::Bridge, 1, 0)],
            ))
            .unwrap();
        let LeaseAllocationResult::Granted { lease } = next.response.result else {
            panic!("expected post-restart grant");
        };
        assert_eq!(lease.lease_id, lease_id("lease-custom-42"));

        let reconciliation = restarted.reconcile(op("op-3"), corr("corr-3")).unwrap();
        assert_eq!(
            reconciliation.actions[0].decision,
            AllocatorEngineDecision::Reconcile
        );
        let changed = restarted
            .allocate(request(
                "op-changed",
                "corr-changed",
                "idem-1",
                lease_owner.clone(),
                vec![request_resource("bridge-2", HostResourceKind::Bridge, 1, 0)],
            ))
            .unwrap();
        assert!(matches!(
            changed.response.result,
            LeaseAllocationResult::Denied {
                reason: AllocatorReasonCode::InvalidRequest,
                ..
            }
        ));

        let (ledger, observed, _) = restarted.into_adapters();
        let mut generation_changed = lease_owner;
        generation_changed.controller_generation = ControllerGenerationId::parse("gen-2").unwrap();
        let restarted_without_owner = LocalRootAllocatorEngine::new(
            owner("root"),
            ledger,
            observed,
            CustomLiveness {
                live: vec![generation_changed],
            },
        );
        let reconciliation = restarted_without_owner
            .reconcile(op("op-4"), corr("corr-4"))
            .unwrap();
        assert_eq!(
            reconciliation.actions[0].decision,
            AllocatorEngineDecision::Reclaim {
                reason: AllocatorReasonCode::OwnerNotLive
            }
        );
    }

    #[test]
    fn grants_resources_in_total_acquisition_order() {
        let owner = owner("work");
        let mut engine = engine(owner.clone(), Vec::new(), Vec::new(), vec![owner.clone()]);
        let allocation = engine
            .allocate(request(
                "op-1",
                "corr-1",
                "idem-1",
                owner,
                vec![
                    request_resource("tap-1", HostResourceKind::Tap, 3, 0),
                    request_resource("bridge-1", HostResourceKind::Bridge, 1, 5),
                    request_resource("cgroup-1", HostResourceKind::CgroupSubtree, 1, 5),
                    request_resource("nft-1", HostResourceKind::NftablesTable, 1, 1),
                ],
            ))
            .unwrap();

        let LeaseAllocationResult::Granted { lease } = allocation.response.result else {
            panic!("expected grant");
        };
        assert_eq!(
            allocation
                .acquired
                .iter()
                .map(|key| key.resource_id.as_str())
                .collect::<Vec<_>>(),
            vec!["nft-1", "bridge-1", "cgroup-1", "tap-1"]
        );
        assert_eq!(
            lease
                .resources
                .iter()
                .map(|resource| resource.resource_id.as_str())
                .collect::<Vec<_>>(),
            vec!["nft-1", "bridge-1", "cgroup-1", "tap-1"]
        );
        assert!(
            allocation
                .decisions
                .iter()
                .all(|decision| decision.decision == AllocatorEngineDecision::Grant)
        );
    }

    #[test]
    fn denies_conflict_with_existing_persisted_lease() {
        let requester = owner("work");
        let other = owner("home");
        let mut engine = engine(
            requester.clone(),
            vec![lease(
                "lease-1",
                other.clone(),
                AllocatorLeaseState::Granted,
                vec![granted("bridge-1", HostResourceKind::Bridge)],
            )],
            Vec::new(),
            vec![requester.clone(), other],
        );

        let allocation = engine
            .allocate(request(
                "op-1",
                "corr-1",
                "idem-1",
                requester,
                vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
            ))
            .unwrap();

        let LeaseAllocationResult::Denied { reason, conflicts } = allocation.response.result else {
            panic!("expected denial");
        };
        assert_eq!(reason, AllocatorReasonCode::ResourceConflict);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].existing_lease, Some(lease_id("lease-1")));
        assert_eq!(
            allocation.decisions[0].decision,
            AllocatorEngineDecision::DenyConflict {
                reason: AllocatorReasonCode::ResourceConflict
            }
        );
    }

    #[test]
    fn denies_observed_foreign_resource_during_allocation() {
        let requester = owner("work");
        let mut engine = engine(
            requester.clone(),
            Vec::new(),
            vec![observed(
                "bridge-foreign",
                HostResourceKind::Bridge,
                ObservedResourceState::ForeignOwner,
            )],
            vec![requester.clone()],
        );

        let allocation = engine
            .allocate(request(
                "op-1",
                "corr-1",
                "idem-1",
                requester,
                vec![request_resource(
                    "bridge-foreign",
                    HostResourceKind::Bridge,
                    1,
                    0,
                )],
            ))
            .unwrap();

        let LeaseAllocationResult::Denied { reason, conflicts } = allocation.response.result else {
            panic!("expected denial");
        };
        assert_eq!(reason, AllocatorReasonCode::ResourceConflict);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].existing_lease, None);
        assert_eq!(conflicts[0].reason, AllocatorReasonCode::OwnershipConflict);
        assert_eq!(
            allocation.decisions[0].decision,
            AllocatorEngineDecision::DenyConflict {
                reason: AllocatorReasonCode::OwnershipConflict
            }
        );
    }

    #[test]
    fn rejects_duplicate_resources_in_direct_engine_request() {
        let owner = owner("work");
        let duplicate = request_resource("bridge-1", HostResourceKind::Bridge, 1, 0);
        let mut engine = engine(owner.clone(), Vec::new(), Vec::new(), vec![owner.clone()]);

        let allocation = engine
            .allocate(request(
                "op-1",
                "corr-1",
                "idem-1",
                owner,
                vec![duplicate.clone(), duplicate],
            ))
            .unwrap();

        let LeaseAllocationResult::Denied { reason, conflicts } = allocation.response.result else {
            panic!("expected denial");
        };
        assert_eq!(reason, AllocatorReasonCode::InvalidRequest);
        assert!(conflicts.is_empty());
        assert!(engine.ledger().leases.is_empty());
    }

    #[test]
    fn replays_idempotent_denial_with_current_correlation() {
        let requester = owner("work");
        let other = owner("home");
        let mut engine = engine(
            requester.clone(),
            vec![lease(
                "lease-1",
                other.clone(),
                AllocatorLeaseState::Granted,
                vec![granted("bridge-1", HostResourceKind::Bridge)],
            )],
            Vec::new(),
            vec![requester.clone(), other],
        );
        let resource = request_resource("bridge-1", HostResourceKind::Bridge, 1, 0);

        let first = engine
            .allocate(request(
                "op-1",
                "corr-1",
                "idem-denied",
                requester.clone(),
                vec![resource.clone()],
            ))
            .unwrap();
        let second = engine
            .allocate(request(
                "op-2",
                "corr-2",
                "idem-denied",
                requester,
                vec![resource],
            ))
            .unwrap();

        assert_eq!(second.response.operation_id, op("op-2"));
        assert_eq!(second.response.correlation_id, corr("corr-2"));
        assert_eq!(first.response.result, second.response.result);
        assert_eq!(second.metrics[0].event, AllocatorEventKind::Denial);
        assert_eq!(
            second.metrics[0].outcome,
            AllocatorEngineOutcome::IdempotentReplay
        );
    }

    #[test]
    fn replays_idempotent_duplicate_with_current_correlation() {
        let owner = owner("work");
        let resource = request_resource("bridge-1", HostResourceKind::Bridge, 1, 0);
        let mut engine = engine(owner.clone(), Vec::new(), Vec::new(), vec![owner.clone()]);
        let first = engine
            .allocate(request(
                "op-1",
                "corr-1",
                "idem-1",
                owner.clone(),
                vec![resource.clone()],
            ))
            .unwrap();
        let second = engine
            .allocate(request("op-2", "corr-2", "idem-1", owner, vec![resource]))
            .unwrap();

        assert_eq!(second.response.operation_id, op("op-2"));
        assert_eq!(second.response.correlation_id, corr("corr-2"));
        assert_eq!(first.response.result, second.response.result);
        assert_eq!(
            second.metrics[0].outcome,
            AllocatorEngineOutcome::IdempotentReplay
        );
        assert_eq!(engine.ledger().leases.len(), 1);
    }

    #[test]
    fn rejects_reused_idempotency_key_for_different_request() {
        let owner = owner("work");
        let mut engine = engine(owner.clone(), Vec::new(), Vec::new(), vec![owner.clone()]);
        let _ = engine
            .allocate(request(
                "op-1",
                "corr-1",
                "idem-1",
                owner.clone(),
                vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
            ))
            .unwrap();
        let second = engine
            .allocate(request(
                "op-2",
                "corr-2",
                "idem-1",
                owner,
                vec![request_resource("bridge-2", HostResourceKind::Bridge, 1, 0)],
            ))
            .unwrap();

        let LeaseAllocationResult::Denied { reason, conflicts } = second.response.result else {
            panic!("expected denial");
        };
        assert_eq!(reason, AllocatorReasonCode::InvalidRequest);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn reclaims_stale_persisted_lease_when_observed_missing() {
        let owner = owner("work");
        let engine = engine(
            owner.clone(),
            vec![lease(
                "lease-1",
                owner.clone(),
                AllocatorLeaseState::Granted,
                vec![granted("bridge-1", HostResourceKind::Bridge)],
            )],
            Vec::new(),
            vec![owner],
        );

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1")).unwrap();

        assert_eq!(
            reconciliation.actions[0].decision,
            AllocatorEngineDecision::Reclaim {
                reason: AllocatorReasonCode::DriftDetected
            }
        );
        assert_eq!(
            reconciliation.report.records[0].decision,
            ReconciliationDecision::Reclaim {
                reason: AllocatorReasonCode::DriftDetected
            }
        );
    }

    #[test]
    fn reconciles_live_persisted_lease_when_observed_present() {
        let owner = owner("work");
        let engine = engine(
            owner.clone(),
            vec![lease(
                "lease-1",
                owner.clone(),
                AllocatorLeaseState::Granted,
                vec![granted("bridge-1", HostResourceKind::Bridge)],
            )],
            vec![observed(
                "bridge-1",
                HostResourceKind::Bridge,
                ObservedResourceState::Present,
            )],
            vec![owner],
        );

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1")).unwrap();

        assert_eq!(
            reconciliation.actions[0].decision,
            AllocatorEngineDecision::Reconcile
        );
        assert_eq!(
            reconciliation.report.records[0].decision,
            ReconciliationDecision::Reconciled
        );
        assert_eq!(
            reconciliation.metrics[0].outcome,
            AllocatorEngineOutcome::Reconciled
        );
        assert_eq!(reconciliation.metrics[0].labels().event, "reconciliation");
    }

    #[test]
    fn preserves_observed_foreign_resource_without_persisted_lease() {
        let owner = owner("work");
        let engine = engine(
            owner.clone(),
            Vec::new(),
            vec![observed(
                "bridge-foreign",
                HostResourceKind::Bridge,
                ObservedResourceState::ForeignOwner,
            )],
            vec![owner],
        );

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1")).unwrap();

        assert_eq!(
            reconciliation.actions[0].decision,
            AllocatorEngineDecision::Preserve {
                reason: AllocatorReasonCode::OwnershipConflict
            }
        );
        assert_eq!(
            reconciliation.report.records[0].decision,
            ReconciliationDecision::Deny {
                reason: AllocatorReasonCode::OwnershipConflict
            }
        );
    }

    #[test]
    fn owner_liveness_placeholder_reclaims_after_broker_death() {
        let owner = owner("work");
        let engine = engine(
            owner.clone(),
            vec![lease(
                "lease-1",
                owner,
                AllocatorLeaseState::Granted,
                vec![granted("bridge-1", HostResourceKind::Bridge)],
            )],
            vec![observed(
                "bridge-1",
                HostResourceKind::Bridge,
                ObservedResourceState::Present,
            )],
            Vec::new(),
        );

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1")).unwrap();

        assert_eq!(
            reconciliation.actions[0].decision,
            AllocatorEngineDecision::Reclaim {
                reason: AllocatorReasonCode::OwnerNotLive
            }
        );
    }

    #[test]
    fn quarantines_foreign_takeover_of_persisted_resource() {
        let owner = owner("work");
        let engine = engine(
            owner.clone(),
            vec![lease(
                "lease-1",
                owner.clone(),
                AllocatorLeaseState::Granted,
                vec![granted("bridge-1", HostResourceKind::Bridge)],
            )],
            vec![observed(
                "bridge-1",
                HostResourceKind::Bridge,
                ObservedResourceState::ForeignOwner,
            )],
            vec![owner],
        );

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1")).unwrap();

        assert_eq!(
            reconciliation.actions[0].decision,
            AllocatorEngineDecision::Quarantine {
                reason: AllocatorReasonCode::DriftDetected
            }
        );
        assert_eq!(
            reconciliation.metrics[0].outcome,
            AllocatorEngineOutcome::Quarantined
        );
    }

    #[test]
    fn quarantines_duplicate_persisted_leases_for_same_resource() {
        let owner = owner("work");
        let engine = engine(
            owner.clone(),
            vec![
                lease(
                    "lease-1",
                    owner.clone(),
                    AllocatorLeaseState::Granted,
                    vec![granted("bridge-1", HostResourceKind::Bridge)],
                ),
                lease(
                    "lease-2",
                    owner.clone(),
                    AllocatorLeaseState::Granted,
                    vec![granted("bridge-1", HostResourceKind::Bridge)],
                ),
            ],
            vec![observed(
                "bridge-1",
                HostResourceKind::Bridge,
                ObservedResourceState::Present,
            )],
            vec![owner],
        );

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1")).unwrap();

        assert_eq!(
            reconciliation.actions[0].decision,
            AllocatorEngineDecision::Quarantine {
                reason: AllocatorReasonCode::StorageContractViolation
            }
        );
    }

    #[test]
    fn event_and_metric_batches_are_bounded_and_low_cardinality() {
        let owner = owner("work");
        let mut engine = engine(owner.clone(), Vec::new(), Vec::new(), vec![owner.clone()]);
        let resources = (0..MAX_ALLOCATOR_REQUEST_RESOURCES)
            .map(|idx| {
                request_resource(
                    &format!("bridge-sensitive-{idx}"),
                    HostResourceKind::Bridge,
                    1,
                    idx as u16,
                )
            })
            .collect::<Vec<_>>();
        let allocation = engine
            .allocate(request("op-1", "corr-1", "idem-1", owner, resources))
            .unwrap();

        assert!(allocation.events.len() <= MAX_ALLOCATOR_EVENTS);
        assert!(allocation.metrics.len() <= MAX_ALLOCATOR_EVENTS);
        let labels = allocation.metrics[0].labels();
        let rendered = format!("{labels:?}");
        assert!(rendered.contains("grant"));
        assert!(rendered.contains("granted"));
        assert!(!rendered.contains("bridge-sensitive"));
        assert!(
            !allocation
                .events
                .iter()
                .any(|event| format!("{event:?}").contains("bridge-sensitive"))
        );
    }

    #[test]
    fn reconciliation_report_events_and_metrics_are_bounded() {
        let owner = owner("work");
        let observations = (0..(MAX_RECONCILIATION_RECORDS + 8))
            .map(|idx| {
                observed(
                    &format!("bridge-sensitive-{idx}"),
                    HostResourceKind::Bridge,
                    ObservedResourceState::Present,
                )
            })
            .collect::<Vec<_>>();
        let engine = engine(owner.clone(), Vec::new(), observations, vec![owner]);

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1")).unwrap();

        assert_eq!(
            reconciliation.report.records.len(),
            MAX_RECONCILIATION_RECORDS
        );
        assert!(reconciliation.report.events.len() <= MAX_ALLOCATOR_EVENTS);
        assert!(reconciliation.metrics.len() <= MAX_ALLOCATOR_EVENTS);
        let labels = reconciliation.metrics[0].labels();
        let rendered = format!("{labels:?}");
        assert!(rendered.contains("preserved"));
        assert!(!rendered.contains("bridge-sensitive"));
        assert!(
            !reconciliation
                .report
                .events
                .iter()
                .any(|event| format!("{event:?}").contains("bridge-sensitive"))
        );
    }

    #[test]
    fn bounded_metrics_aggregates_repeated_labels() {
        let metrics = bounded_metrics(vec![
            AllocatorMetricEvent::new(
                AllocatorEventKind::Grant,
                AllocatorEngineOutcome::Granted,
                Some(HostResourceKind::Bridge),
                None,
            ),
            AllocatorMetricEvent::new(
                AllocatorEventKind::Denial,
                AllocatorEngineOutcome::Denied,
                Some(HostResourceKind::Tap),
                Some(AllocatorReasonCode::ResourceConflict),
            ),
            AllocatorMetricEvent::new(
                AllocatorEventKind::Grant,
                AllocatorEngineOutcome::Granted,
                Some(HostResourceKind::Bridge),
                None,
            ),
        ]);

        assert_eq!(metrics.len(), 2);
        let grant = metrics
            .iter()
            .find(|metric| metric.labels().event == "grant")
            .unwrap();
        assert_eq!(grant.count, 2);
        assert_eq!(grant.resource_kind, Some(HostResourceKind::Bridge));
        assert_eq!(grant.reason, None);
    }

    #[test]
    fn bounded_metrics_truncates_after_aggregation_in_label_order() {
        let duplicate_grants = (0..(MAX_ALLOCATOR_EVENTS + 8)).map(|_| {
            AllocatorMetricEvent::new(
                AllocatorEventKind::Grant,
                AllocatorEngineOutcome::Granted,
                Some(HostResourceKind::Bridge),
                None,
            )
        });
        let unique_denials = [
            HostResourceKind::Bridge,
            HostResourceKind::CgroupSubtree,
            HostResourceKind::HostFilePartition,
            HostResourceKind::NamespaceBoundary,
            HostResourceKind::NftablesPartition,
            HostResourceKind::NftablesTable,
            HostResourceKind::Tap,
            HostResourceKind::VethPair,
        ]
        .into_iter()
        .map(|kind| {
            AllocatorMetricEvent::new(
                AllocatorEventKind::Denial,
                AllocatorEngineOutcome::Denied,
                Some(kind),
                Some(AllocatorReasonCode::ResourceConflict),
            )
        });
        let metrics = bounded_metrics(duplicate_grants.chain(unique_denials));

        assert_eq!(metrics.len(), 9);
        assert_eq!(metrics[8].labels().event, "grant");
        assert_eq!(metrics[8].count, (MAX_ALLOCATOR_EVENTS + 8) as u64);
        assert!(
            metrics
                .windows(2)
                .all(|window| window[0].labels() <= window[1].labels())
        );
    }
}
