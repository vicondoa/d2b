//! Hermetic local-root allocator engine over fake host observations.
//!
//! The engine is deliberately pure: it reconciles typed allocator DTOs against
//! in-memory fake ledger/liveness/observation backends and emits decisions,
//! audit metadata, and low-cardinality metric samples. It performs no netlink,
//! nftables, filesystem, systemd, broker, or other live host mutation.

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

/// Result of reconciling fake observed host state against fake persisted leases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatorEngineReconciliation {
    pub report: ReconciliationReport,
    pub actions: Vec<AllocatorReconciliationAction>,
    pub metrics: Vec<AllocatorMetricEvent>,
}

/// In-memory fake observed host state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeObservedAllocatorState {
    pub resources: Vec<ObservedHostResource>,
}

impl FakeObservedAllocatorState {
    pub fn new(resources: Vec<ObservedHostResource>) -> Self {
        Self { resources }
    }
}

/// In-memory fake owner-liveness backend.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeAllocatorLiveness {
    pub live_owners: Vec<LeaseOwner>,
}

impl FakeAllocatorLiveness {
    pub fn new(live_owners: Vec<LeaseOwner>) -> Self {
        Self { live_owners }
    }

    fn is_live(&self, owner: &LeaseOwner) -> bool {
        self.live_owners.iter().any(|candidate| candidate == owner)
    }
}

/// In-memory fake allocator ledger, including idempotency replay records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeAllocatorLedger {
    pub leases: Vec<AllocatorLease>,
    idempotency: Vec<AllocatorIdempotencyRecord>,
    next_lease_sequence: u64,
}

impl Default for FakeAllocatorLedger {
    fn default() -> Self {
        Self {
            leases: Vec::new(),
            idempotency: Vec::new(),
            next_lease_sequence: 1,
        }
    }
}

impl FakeAllocatorLedger {
    pub fn new(leases: Vec<AllocatorLease>) -> Self {
        Self {
            leases,
            ..Self::default()
        }
    }

    fn remember_idempotency(
        &mut self,
        key: IdempotencyKey,
        signature: AllocationRequestSignature,
        result: LeaseAllocationResult,
    ) {
        self.idempotency.push(AllocatorIdempotencyRecord {
            key,
            signature,
            result,
        });
    }

    fn idempotency_record(&self, key: &IdempotencyKey) -> Option<&AllocatorIdempotencyRecord> {
        self.idempotency.iter().find(|record| &record.key == key)
    }

    fn next_lease_id(&mut self) -> AllocatorLeaseId {
        let existing = self
            .leases
            .iter()
            .map(|lease| lease.lease_id.as_str())
            .collect::<BTreeSet<_>>();
        loop {
            let candidate = format!("lease-engine-{}", self.next_lease_sequence);
            self.next_lease_sequence += 1;
            if existing.contains(candidate.as_str()) {
                continue;
            }
            return AllocatorLeaseId::parse(candidate).expect("generated lease id is valid");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AllocatorIdempotencyRecord {
    key: IdempotencyKey,
    signature: AllocationRequestSignature,
    result: LeaseAllocationResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Pure allocator engine using fake ledger, observed-state, and liveness
/// backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRootAllocatorEngine {
    allocator_owner: LeaseOwner,
    pub ledger: FakeAllocatorLedger,
    pub observed: FakeObservedAllocatorState,
    pub liveness: FakeAllocatorLiveness,
}

impl LocalRootAllocatorEngine {
    pub fn new(
        allocator_owner: LeaseOwner,
        ledger: FakeAllocatorLedger,
        observed: FakeObservedAllocatorState,
        liveness: FakeAllocatorLiveness,
    ) -> Self {
        Self {
            allocator_owner,
            ledger,
            observed,
            liveness,
        }
    }

    /// Allocate a typed lease request without touching the live host.
    pub fn allocate(&mut self, request: LeaseAllocationRequest) -> AllocatorEngineAllocation {
        let acquired = request.acquisition_order();
        let signature = AllocationRequestSignature::from_request(&request);

        if let Some(record) = self.ledger.idempotency_record(&request.idempotency_key) {
            if record.signature == signature {
                let metric = metric_from_replay_result(&record.result);
                let response = LeaseAllocationResponse {
                    operation_id: request.operation_id.clone(),
                    correlation_id: request.correlation_id.clone(),
                    result: record.result.clone(),
                };
                return AllocatorEngineAllocation {
                    response,
                    decisions: Vec::new(),
                    events: Vec::new(),
                    metrics: vec![metric],
                    acquired,
                };
            }

            return self.deny_request(
                &request,
                acquired,
                AllocatorReasonCode::InvalidRequest,
                Vec::new(),
            );
        }

        if !self.liveness.is_live(&request.owner) {
            return self.deny_request(
                &request,
                acquired,
                AllocatorReasonCode::OwnerNotLive,
                Vec::new(),
            );
        }

        if request.resources.is_empty() || request.resources.len() > MAX_ALLOCATOR_REQUEST_RESOURCES
        {
            return self.deny_request(
                &request,
                acquired,
                AllocatorReasonCode::InvalidRequest,
                Vec::new(),
            );
        }

        if has_duplicate_requested_resources(&request) {
            return self.deny_request(
                &request,
                acquired,
                AllocatorReasonCode::InvalidRequest,
                Vec::new(),
            );
        }

        let conflicts = self.allocation_conflicts(&request);
        if !conflicts.is_empty() {
            return self.deny_request(
                &request,
                acquired,
                AllocatorReasonCode::ResourceConflict,
                conflicts,
            );
        }

        let lease = AllocatorLease {
            lease_id: self.ledger.next_lease_id(),
            owner: request.owner.clone(),
            state: AllocatorLeaseState::Granted,
            resources: grant_resources_in_order(&request),
        };
        self.ledger.leases.push(lease.clone());

        let result = LeaseAllocationResult::Granted { lease };
        self.ledger.remember_idempotency(
            request.idempotency_key.clone(),
            signature,
            result.clone(),
        );

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

        AllocatorEngineAllocation {
            response,
            decisions,
            events,
            metrics,
            acquired,
        }
    }

    /// Reconcile fake observed host state against fake persisted leases.
    pub fn reconcile(
        &self,
        operation_id: OperationId,
        correlation_id: CorrelationId,
    ) -> AllocatorEngineReconciliation {
        let mut resources = BTreeMap::<ResourceKey, ResourcePair>::new();

        for lease in &self.ledger.leases {
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

        for observed in &self.observed.resources {
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

        AllocatorEngineReconciliation {
            report,
            actions,
            metrics,
        }
    }

    fn allocation_conflicts(&self, request: &LeaseAllocationRequest) -> Vec<AllocatorConflict> {
        let mut conflicts = Vec::new();
        for resource in request.acquisition_order() {
            if let Some(conflict) = self.persisted_conflict(&request.owner, &resource) {
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
        owner: &LeaseOwner,
        resource: &ResourceAcquisitionKey,
    ) -> Option<AllocatorConflict> {
        self.ledger
            .leases
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
            .resources
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
        request: &LeaseAllocationRequest,
        acquired: Vec<ResourceAcquisitionKey>,
        reason: AllocatorReasonCode,
        conflicts: Vec<AllocatorConflict>,
    ) -> AllocatorEngineAllocation {
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
        if self
            .ledger
            .idempotency_record(&request.idempotency_key)
            .is_none()
        {
            self.ledger.remember_idempotency(
                request.idempotency_key.clone(),
                AllocationRequestSignature::from_request(request),
                result.clone(),
            );
        }
        let response = LeaseAllocationResponse {
            operation_id: request.operation_id.clone(),
            correlation_id: request.correlation_id.clone(),
            result,
        };
        AllocatorEngineAllocation {
            response,
            decisions,
            events,
            metrics,
            acquired,
        }
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

    fn engine(
        owner: LeaseOwner,
        leases: Vec<AllocatorLease>,
        observations: Vec<ObservedHostResource>,
        live_owners: Vec<LeaseOwner>,
    ) -> LocalRootAllocatorEngine {
        LocalRootAllocatorEngine::new(
            owner,
            FakeAllocatorLedger::new(leases),
            FakeObservedAllocatorState::new(observations),
            FakeAllocatorLiveness::new(live_owners),
        )
    }

    #[test]
    fn grants_resources_in_total_acquisition_order() {
        let owner = owner("work");
        let mut engine = engine(owner.clone(), Vec::new(), Vec::new(), vec![owner.clone()]);
        let allocation = engine.allocate(request(
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
        ));

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

        let allocation = engine.allocate(request(
            "op-1",
            "corr-1",
            "idem-1",
            requester,
            vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
        ));

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

        let allocation = engine.allocate(request(
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
        ));

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

        let allocation = engine.allocate(request(
            "op-1",
            "corr-1",
            "idem-1",
            owner,
            vec![duplicate.clone(), duplicate],
        ));

        let LeaseAllocationResult::Denied { reason, conflicts } = allocation.response.result else {
            panic!("expected denial");
        };
        assert_eq!(reason, AllocatorReasonCode::InvalidRequest);
        assert!(conflicts.is_empty());
        assert!(engine.ledger.leases.is_empty());
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

        let first = engine.allocate(request(
            "op-1",
            "corr-1",
            "idem-denied",
            requester.clone(),
            vec![resource.clone()],
        ));
        let second = engine.allocate(request(
            "op-2",
            "corr-2",
            "idem-denied",
            requester,
            vec![resource],
        ));

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
        let first = engine.allocate(request(
            "op-1",
            "corr-1",
            "idem-1",
            owner.clone(),
            vec![resource.clone()],
        ));
        let second = engine.allocate(request("op-2", "corr-2", "idem-1", owner, vec![resource]));

        assert_eq!(second.response.operation_id, op("op-2"));
        assert_eq!(second.response.correlation_id, corr("corr-2"));
        assert_eq!(first.response.result, second.response.result);
        assert_eq!(
            second.metrics[0].outcome,
            AllocatorEngineOutcome::IdempotentReplay
        );
        assert_eq!(engine.ledger.leases.len(), 1);
    }

    #[test]
    fn rejects_reused_idempotency_key_for_different_request() {
        let owner = owner("work");
        let mut engine = engine(owner.clone(), Vec::new(), Vec::new(), vec![owner.clone()]);
        let _ = engine.allocate(request(
            "op-1",
            "corr-1",
            "idem-1",
            owner.clone(),
            vec![request_resource("bridge-1", HostResourceKind::Bridge, 1, 0)],
        ));
        let second = engine.allocate(request(
            "op-2",
            "corr-2",
            "idem-1",
            owner,
            vec![request_resource("bridge-2", HostResourceKind::Bridge, 1, 0)],
        ));

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

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1"));

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

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1"));

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

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1"));

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

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1"));

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

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1"));

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

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1"));

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
        let allocation = engine.allocate(request("op-1", "corr-1", "idem-1", owner, resources));

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

        let reconciliation = engine.reconcile(op("op-1"), corr("corr-1"));

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
