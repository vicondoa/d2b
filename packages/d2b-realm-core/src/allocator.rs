//! Pure local-root host-resource allocator DTOs (ADR 0043).
//!
//! This module defines the shared contract for typed host-resource leases,
//! reconciliation observations, and bounded allocator audit/metric metadata. It
//! intentionally contains no netlink, nftables, filesystem, broker, daemon, or
//! provider implementation code.

use crate::ids::{
    AllocatorLeaseId, ControllerGenerationId, CorrelationId, HostResourceId, IdempotencyKey,
    NodeId, OperationId,
};
use crate::realm::RealmPath;
use crate::trace_context::TraceContext;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;

/// Maximum resources in a single allocator lease request.
pub const MAX_ALLOCATOR_REQUEST_RESOURCES: usize = 32;
/// Maximum conflicts carried in a denial response.
pub const MAX_ALLOCATOR_CONFLICTS: usize = 16;
/// Maximum records in one reconciliation report.
pub const MAX_RECONCILIATION_RECORDS: usize = 128;
/// Maximum allocator events in one bounded metadata batch.
pub const MAX_ALLOCATOR_EVENTS: usize = 128;

/// Closed host-resource kinds the local root allocator can arbitrate.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum HostResourceKind {
    Bridge,
    Tap,
    VethPair,
    NftablesTable,
    NftablesPartition,
    CgroupSubtree,
    HostFilePartition,
    NamespaceBoundary,
}

impl HostResourceKind {
    /// Stable low-cardinality metric label for this resource kind.
    pub fn as_metric_label(self) -> &'static str {
        match self {
            Self::Bridge => "bridge",
            Self::Tap => "tap",
            Self::VethPair => "veth-pair",
            Self::NftablesTable => "nftables-table",
            Self::NftablesPartition => "nftables-partition",
            Self::CgroupSubtree => "cgroup-subtree",
            Self::HostFilePartition => "host-file-partition",
            Self::NamespaceBoundary => "namespace-boundary",
        }
    }
}

/// The realm-controller generation that owns a lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LeaseOwner {
    /// Owning realm path.
    pub realm: RealmPath,
    /// Active controller generation that requested the lease.
    pub controller_generation: ControllerGenerationId,
    /// Realm node/broker identity, when the caller has one bound.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub node: Option<NodeId>,
}

/// Shared-vs-exclusive resource semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceShareMode {
    /// No other owner may hold the same resource id.
    Exclusive,
    /// The allocator grants a partition inside a shared resource.
    SharedPartition,
}

/// Explicit acquisition order used for multi-resource requests.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ResourceAcquisitionOrder {
    /// Coarse phase. Lower phases are acquired first.
    pub phase: u16,
    /// Order within the phase. Lower ordinals are acquired first.
    pub ordinal: u16,
}

/// One resource requested from the local-root allocator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LeaseResourceRequest {
    /// Opaque requested resource/partition id.
    pub resource_id: HostResourceId,
    /// Resource kind.
    pub kind: HostResourceKind,
    /// Sharing contract.
    pub share: ResourceShareMode,
    /// Total acquisition order metadata.
    pub acquisition_order: ResourceAcquisitionOrder,
}

impl LeaseResourceRequest {
    fn acquisition_key(&self) -> ResourceAcquisitionKey {
        ResourceAcquisitionKey {
            order: self.acquisition_order,
            kind: self.kind,
            resource_id: self.resource_id.clone(),
        }
    }
}

/// Deterministic total-order key for resource acquisition.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ResourceAcquisitionKey {
    pub order: ResourceAcquisitionOrder,
    pub kind: HostResourceKind,
    pub resource_id: HostResourceId,
}

/// Mutating lease allocation request. Idempotency is mandatory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LeaseAllocationRequest {
    /// Per-attempt operation id.
    pub operation_id: OperationId,
    /// Cross-realm correlation id.
    pub correlation_id: CorrelationId,
    /// Caller-generated dedup key for at-least-once delivery.
    pub idempotency_key: IdempotencyKey,
    /// Lease owner realm/controller generation.
    pub owner: LeaseOwner,
    /// Requested host resources.
    #[schemars(length(min = 1, max = 32))]
    pub resources: Vec<LeaseResourceRequest>,
    /// Optional bounded trace metadata.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trace: Option<TraceContext>,
}

impl<'de> Deserialize<'de> for LeaseAllocationRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            operation_id: OperationId,
            correlation_id: CorrelationId,
            idempotency_key: IdempotencyKey,
            owner: LeaseOwner,
            resources: Vec<LeaseResourceRequest>,
            #[serde(default)]
            trace: Option<TraceContext>,
        }

        let raw = Raw::deserialize(deserializer)?;
        validate_resource_requests(&raw.resources).map_err(serde::de::Error::custom)?;
        Ok(Self {
            operation_id: raw.operation_id,
            correlation_id: raw.correlation_id,
            idempotency_key: raw.idempotency_key,
            owner: raw.owner,
            resources: raw.resources,
            trace: raw.trace,
        })
    }
}

impl LeaseAllocationRequest {
    /// Resource acquisition keys in deterministic total order. Ties in the
    /// caller-supplied phase/ordinal are broken by closed resource kind and
    /// opaque resource id so every participant can sort identically.
    pub fn acquisition_order(&self) -> Vec<ResourceAcquisitionKey> {
        let mut ordered = self
            .resources
            .iter()
            .map(LeaseResourceRequest::acquisition_key)
            .collect::<Vec<_>>();
        ordered.sort();
        ordered
    }
}

/// How a granted resource is delegated to a realm broker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ResourceDelegation {
    /// Delegated opaque name/handle. It is not a raw interface/path name.
    OpaqueName { id: HostResourceId },
    /// FD handoff placeholder id used only for correlation before SCM_RIGHTS.
    FileDescriptor { id: HostResourceId },
    /// Shared-resource partition id.
    PartitionId { id: HostResourceId },
    /// Namespace-boundary handle id.
    NamespaceHandle { id: HostResourceId },
}

/// One resource granted under a lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrantedHostResource {
    pub resource_id: HostResourceId,
    pub kind: HostResourceKind,
    pub share: ResourceShareMode,
    pub delegation: ResourceDelegation,
    pub acquisition_order: ResourceAcquisitionOrder,
}

/// Allocator lease lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AllocatorLeaseState {
    Granted,
    Reconciled,
    Quarantined,
    Reclaimed,
    Denied,
}

impl AllocatorLeaseState {
    /// Terminal states cannot transition to an active lease.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Reclaimed | Self::Denied)
    }

    /// Pure DTO helper for fail-closed state-machine checks.
    pub fn can_transition_to(self, next: Self) -> bool {
        use AllocatorLeaseState as S;
        matches!(
            (self, next),
            (S::Granted, S::Reconciled)
                | (S::Granted, S::Quarantined)
                | (S::Granted, S::Reclaimed)
                | (S::Reconciled, S::Quarantined)
                | (S::Reconciled, S::Reclaimed)
                | (S::Quarantined, S::Reconciled)
                | (S::Quarantined, S::Reclaimed)
        )
    }
}

/// Persisted allocator lease record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AllocatorLease {
    pub lease_id: AllocatorLeaseId,
    pub owner: LeaseOwner,
    pub state: AllocatorLeaseState,
    #[schemars(length(min = 1, max = 32))]
    pub resources: Vec<GrantedHostResource>,
}

impl<'de> Deserialize<'de> for AllocatorLease {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            lease_id: AllocatorLeaseId,
            owner: LeaseOwner,
            state: AllocatorLeaseState,
            resources: Vec<GrantedHostResource>,
        }

        let raw = Raw::deserialize(deserializer)?;
        validate_granted_resources(&raw.resources).map_err(serde::de::Error::custom)?;
        Ok(Self {
            lease_id: raw.lease_id,
            owner: raw.owner,
            state: raw.state,
            resources: raw.resources,
        })
    }
}

/// Closed, low-cardinality allocator denial/conflict/quarantine reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AllocatorReasonCode {
    ResourceConflict,
    OwnershipConflict,
    AcquisitionOrderViolation,
    InvalidRequest,
    CapacityExhausted,
    DriftDetected,
    ReconcileMismatch,
    OwnerNotLive,
    PolicyDenied,
    UnsupportedKind,
    StorageContractViolation,
    KernelStateUnknown,
}

impl AllocatorReasonCode {
    /// Stable low-cardinality metric label.
    pub fn as_metric_label(self) -> &'static str {
        match self {
            Self::ResourceConflict => "resource-conflict",
            Self::OwnershipConflict => "ownership-conflict",
            Self::AcquisitionOrderViolation => "acquisition-order-violation",
            Self::InvalidRequest => "invalid-request",
            Self::CapacityExhausted => "capacity-exhausted",
            Self::DriftDetected => "drift-detected",
            Self::ReconcileMismatch => "reconcile-mismatch",
            Self::OwnerNotLive => "owner-not-live",
            Self::PolicyDenied => "policy-denied",
            Self::UnsupportedKind => "unsupported-kind",
            Self::StorageContractViolation => "storage-contract-violation",
            Self::KernelStateUnknown => "kernel-state-unknown",
        }
    }
}

/// Low-cardinality reason count; keep this small before adding new labels.
pub const ALLOCATOR_REASON_CODE_COUNT: usize = 12;

/// Conflict metadata returned on denial. Only opaque ids and closed reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AllocatorConflict {
    pub resource_id: HostResourceId,
    pub kind: HostResourceKind,
    pub reason: AllocatorReasonCode,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub existing_lease: Option<AllocatorLeaseId>,
}

/// Allocator response to a typed lease request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LeaseAllocationResponse {
    pub operation_id: OperationId,
    pub correlation_id: CorrelationId,
    pub result: LeaseAllocationResult,
}

/// Granted or fail-closed denied allocation result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "status", deny_unknown_fields)]
pub enum LeaseAllocationResult {
    Granted {
        lease: AllocatorLease,
    },
    Denied {
        reason: AllocatorReasonCode,
        #[schemars(length(max = 16))]
        conflicts: Vec<AllocatorConflict>,
    },
}

impl<'de> Deserialize<'de> for LeaseAllocationResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        const FIELDS: &[&str] = &["status", "lease", "reason", "conflicts"];

        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case")]
        enum Status {
            Granted,
            Denied,
        }

        struct ResultVisitor;

        impl<'de> serde::de::Visitor<'de> for ResultVisitor {
            type Value = LeaseAllocationResult;

            fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter.write_str("lease allocation result")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut status = None;
                let mut lease_present = false;
                let mut lease = None;
                let mut reason_present = false;
                let mut reason = None;
                let mut conflicts_present = false;
                let mut conflicts = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "status" => {
                            if status.is_some() {
                                return Err(serde::de::Error::duplicate_field("status"));
                            }
                            status = Some(map.next_value::<Status>()?);
                        }
                        "lease" => {
                            if lease_present {
                                return Err(serde::de::Error::duplicate_field("lease"));
                            }
                            lease_present = true;
                            lease = map.next_value::<Option<AllocatorLease>>()?;
                        }
                        "reason" => {
                            if reason_present {
                                return Err(serde::de::Error::duplicate_field("reason"));
                            }
                            reason_present = true;
                            reason = map.next_value::<Option<AllocatorReasonCode>>()?;
                        }
                        "conflicts" => {
                            if conflicts_present {
                                return Err(serde::de::Error::duplicate_field("conflicts"));
                            }
                            conflicts_present = true;
                            conflicts = map.next_value::<Option<Vec<AllocatorConflict>>>()?;
                        }
                        other => return Err(serde::de::Error::unknown_field(other, FIELDS)),
                    }
                }

                let status = status.ok_or_else(|| serde::de::Error::missing_field("status"))?;
                match status {
                    Status::Granted => {
                        if reason_present || conflicts_present {
                            return Err(serde::de::Error::custom(
                                "granted allocation result must not carry denial fields",
                            ));
                        }
                        if !lease_present {
                            return Err(serde::de::Error::missing_field("lease"));
                        }
                        lease
                            .map(|lease| LeaseAllocationResult::Granted { lease })
                            .ok_or_else(|| {
                                serde::de::Error::custom(
                                    "granted allocation result requires non-null lease",
                                )
                            })
                    }
                    Status::Denied => {
                        if lease_present {
                            return Err(serde::de::Error::custom(
                                "denied allocation result must not carry lease",
                            ));
                        }
                        if !reason_present {
                            return Err(serde::de::Error::missing_field("reason"));
                        }
                        let reason = reason.ok_or_else(|| {
                            serde::de::Error::custom(
                                "denied allocation result requires non-null reason",
                            )
                        })?;
                        if !conflicts_present {
                            return Err(serde::de::Error::missing_field("conflicts"));
                        }
                        let conflicts = conflicts.ok_or_else(|| {
                            serde::de::Error::custom(
                                "denied allocation result requires non-null conflicts",
                            )
                        })?;
                        validate_len(&conflicts, MAX_ALLOCATOR_CONFLICTS, "conflicts")
                            .map_err(serde::de::Error::custom)?;
                        Ok(LeaseAllocationResult::Denied { reason, conflicts })
                    }
                }
            }
        }

        deserializer.deserialize_map(ResultVisitor)
    }
}

/// Bounded allocator audit/metric event kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AllocatorEventKind {
    Grant,
    Denial,
    Conflict,
    Reconciliation,
    Reclamation,
    Quarantine,
}

impl AllocatorEventKind {
    /// Stable low-cardinality metric label.
    pub fn as_metric_label(self) -> &'static str {
        match self {
            Self::Grant => "grant",
            Self::Denial => "denial",
            Self::Conflict => "conflict",
            Self::Reconciliation => "reconciliation",
            Self::Reclamation => "reclamation",
            Self::Quarantine => "quarantine",
        }
    }
}

/// Bounded allocator event metadata. Carries no paths, interface names,
/// command output, or credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AllocatorEventMetadata {
    pub operation_id: OperationId,
    pub correlation_id: CorrelationId,
    pub owner: LeaseOwner,
    pub event: AllocatorEventKind,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resource_kind: Option<HostResourceKind>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<AllocatorReasonCode>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trace: Option<TraceContext>,
}

/// Persisted resource lease metadata from the allocator ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PersistedResourceLease {
    pub lease_id: AllocatorLeaseId,
    pub owner: LeaseOwner,
    pub state: AllocatorLeaseState,
}

/// Contract-only source for observed host state. These values describe which
/// subsystem produced an observation; they do not imply an implementation here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceObservationSource {
    KernelNetlink,
    NftablesApi,
    CgroupFs,
    HostFilesystem,
    NamespaceRegistry,
    AllocatorLedger,
}

/// Kernel/host-observed state for one resource id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ObservedResourceState {
    Present,
    Missing,
    ForeignOwner,
    Ambiguous,
    Inaccessible,
}

/// Host-resource observation from the live system side of reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservedHostResource {
    pub resource_id: HostResourceId,
    pub kind: HostResourceKind,
    pub source: ResourceObservationSource,
    pub state: ObservedResourceState,
}

/// Reconciliation decision for a persisted/live observation pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "decision", deny_unknown_fields)]
pub enum ReconciliationDecision {
    Reconciled,
    Quarantine { reason: AllocatorReasonCode },
    Reclaim { reason: AllocatorReasonCode },
    Deny { reason: AllocatorReasonCode },
}

impl ReconciliationDecision {
    /// True when this decision prevents reuse until remediation.
    pub fn is_fail_closed(&self) -> bool {
        matches!(
            self,
            Self::Quarantine { .. } | Self::Deny { .. } | Self::Reclaim { .. }
        )
    }
}

impl<'de> Deserialize<'de> for ReconciliationDecision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        const FIELDS: &[&str] = &["decision", "reason"];

        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case")]
        enum Decision {
            Reconciled,
            Quarantine,
            Reclaim,
            Deny,
        }

        struct DecisionVisitor;

        impl<'de> serde::de::Visitor<'de> for DecisionVisitor {
            type Value = ReconciliationDecision;

            fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter.write_str("reconciliation decision")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut decision = None;
                let mut reason_present = false;
                let mut reason = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "decision" => {
                            if decision.is_some() {
                                return Err(serde::de::Error::duplicate_field("decision"));
                            }
                            decision = Some(map.next_value::<Decision>()?);
                        }
                        "reason" => {
                            if reason_present {
                                return Err(serde::de::Error::duplicate_field("reason"));
                            }
                            reason_present = true;
                            reason = map.next_value::<Option<AllocatorReasonCode>>()?;
                        }
                        other => return Err(serde::de::Error::unknown_field(other, FIELDS)),
                    }
                }

                let decision =
                    decision.ok_or_else(|| serde::de::Error::missing_field("decision"))?;
                match decision {
                    Decision::Reconciled => {
                        if reason_present {
                            Err(serde::de::Error::custom(
                                "reconciled decision must not carry reason",
                            ))
                        } else {
                            Ok(ReconciliationDecision::Reconciled)
                        }
                    }
                    Decision::Quarantine => reason
                        .map(|reason| ReconciliationDecision::Quarantine { reason })
                        .ok_or_else(|| {
                            serde::de::Error::custom("quarantine decision requires non-null reason")
                        }),
                    Decision::Reclaim => reason
                        .map(|reason| ReconciliationDecision::Reclaim { reason })
                        .ok_or_else(|| {
                            serde::de::Error::custom("reclaim decision requires non-null reason")
                        }),
                    Decision::Deny => reason
                        .map(|reason| ReconciliationDecision::Deny { reason })
                        .ok_or_else(|| {
                            serde::de::Error::custom("deny decision requires non-null reason")
                        }),
                }
            }
        }

        deserializer.deserialize_map(DecisionVisitor)
    }
}

/// One reconciliation record distinguishes persisted lease state from
/// live kernel/host-observed state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationRecord {
    pub resource_id: HostResourceId,
    pub kind: HostResourceKind,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub persisted: Option<PersistedResourceLease>,
    pub observed: ObservedHostResource,
    pub decision: ReconciliationDecision,
}

/// Bounded reconciliation report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationReport {
    pub operation_id: OperationId,
    pub correlation_id: CorrelationId,
    #[schemars(length(max = 128))]
    pub records: Vec<ReconciliationRecord>,
    #[schemars(length(max = 128))]
    pub events: Vec<AllocatorEventMetadata>,
}

impl<'de> Deserialize<'de> for ReconciliationReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            operation_id: OperationId,
            correlation_id: CorrelationId,
            records: Vec<ReconciliationRecord>,
            events: Vec<AllocatorEventMetadata>,
        }

        let raw = Raw::deserialize(deserializer)?;
        validate_len(&raw.records, MAX_RECONCILIATION_RECORDS, "records")
            .map_err(serde::de::Error::custom)?;
        validate_len(&raw.events, MAX_ALLOCATOR_EVENTS, "events")
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            operation_id: raw.operation_id,
            correlation_id: raw.correlation_id,
            records: raw.records,
            events: raw.events,
        })
    }
}

fn validate_resource_requests(resources: &[LeaseResourceRequest]) -> Result<(), &'static str> {
    validate_len_non_empty(
        resources,
        MAX_ALLOCATOR_REQUEST_RESOURCES,
        "allocator resource requests",
    )?;
    let mut ids = BTreeSet::new();
    for resource in resources {
        if !ids.insert(resource.resource_id.clone()) {
            return Err("duplicate resource id in allocator request");
        }
    }
    Ok(())
}

fn validate_granted_resources(resources: &[GrantedHostResource]) -> Result<(), &'static str> {
    validate_len_non_empty(
        resources,
        MAX_ALLOCATOR_REQUEST_RESOURCES,
        "granted allocator resources",
    )?;
    let mut ids = BTreeSet::new();
    for resource in resources {
        if !ids.insert(resource.resource_id.clone()) {
            return Err("duplicate granted resource id in allocator lease");
        }
    }
    Ok(())
}

fn validate_len<T>(items: &[T], max: usize, label: &'static str) -> Result<(), &'static str> {
    if items.len() > max {
        Err(match label {
            "conflicts" => "too many allocator conflicts",
            "records" => "too many reconciliation records",
            "events" => "too many allocator events",
            _ => "too many allocator items",
        })
    } else {
        Ok(())
    }
}

fn validate_len_non_empty<T>(
    items: &[T],
    max: usize,
    label: &'static str,
) -> Result<(), &'static str> {
    if items.is_empty() {
        Err(match label {
            "allocator resource requests" => "allocator request must include at least one resource",
            "granted allocator resources" => "allocator lease must include at least one resource",
            _ => "allocator list must not be empty",
        })
    } else {
        validate_len(items, max, label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::RealmId;
    use serde_json::json;
    use std::collections::BTreeSet;

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

    fn owner() -> LeaseOwner {
        LeaseOwner {
            realm: RealmPath::new(vec![RealmId::parse("work").unwrap()]).unwrap(),
            controller_generation: ControllerGenerationId::parse("gen-1").unwrap(),
            node: Some(NodeId::parse("realm-node").unwrap()),
        }
    }

    fn request_resource(raw: &str, kind: HostResourceKind, phase: u16) -> LeaseResourceRequest {
        LeaseResourceRequest {
            resource_id: id(raw),
            kind,
            share: ResourceShareMode::Exclusive,
            acquisition_order: ResourceAcquisitionOrder { phase, ordinal: 0 },
        }
    }

    fn granted_resource(raw: &str, kind: HostResourceKind) -> GrantedHostResource {
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

    fn allocation_request_value(resources: Vec<serde_json::Value>) -> serde_json::Value {
        json!({
            "operation_id": "op-1",
            "correlation_id": "corr-1",
            "idempotency_key": "idem-1",
            "owner": {
                "realm": ["work"],
                "controller_generation": "gen-1",
                "node": "realm-node"
            },
            "resources": resources
        })
    }

    fn resource_value(raw: &str) -> serde_json::Value {
        json!({
            "resource_id": raw,
            "kind": "bridge",
            "share": "exclusive",
            "acquisition_order": { "phase": 1, "ordinal": 0 }
        })
    }

    #[test]
    fn allocation_request_decode_is_strict_and_bounded() {
        let mut ok = allocation_request_value(vec![resource_value("bridge-1")]);
        assert!(serde_json::from_value::<LeaseAllocationRequest>(ok.clone()).is_ok());

        ok["unexpected"] = json!(true);
        assert!(serde_json::from_value::<LeaseAllocationRequest>(ok).is_err());

        let empty = allocation_request_value(Vec::new());
        assert!(serde_json::from_value::<LeaseAllocationRequest>(empty).is_err());

        let too_many = allocation_request_value(
            (0..=MAX_ALLOCATOR_REQUEST_RESOURCES)
                .map(|i| resource_value(&format!("bridge-{i}")))
                .collect(),
        );
        assert!(serde_json::from_value::<LeaseAllocationRequest>(too_many).is_err());

        let duplicate =
            allocation_request_value(vec![resource_value("bridge-1"), resource_value("bridge-1")]);
        assert!(serde_json::from_value::<LeaseAllocationRequest>(duplicate).is_err());
    }

    #[test]
    fn debug_keeps_resource_and_lease_ids_actionable() {
        let lease = AllocatorLease {
            lease_id: lease_id("lease-home-1"),
            owner: owner(),
            state: AllocatorLeaseState::Granted,
            resources: vec![granted_resource("bridge-home-1", HostResourceKind::Bridge)],
        };
        let debug = format!("{lease:?}");
        assert!(debug.contains("AllocatorLeaseId(\"lease-home-1\")"));
        assert!(debug.contains("HostResourceId(\"bridge-home-1\")"));
    }

    #[test]
    fn reason_codes_are_low_cardinality_static_labels() {
        let reasons = [
            AllocatorReasonCode::ResourceConflict,
            AllocatorReasonCode::OwnershipConflict,
            AllocatorReasonCode::AcquisitionOrderViolation,
            AllocatorReasonCode::InvalidRequest,
            AllocatorReasonCode::CapacityExhausted,
            AllocatorReasonCode::DriftDetected,
            AllocatorReasonCode::ReconcileMismatch,
            AllocatorReasonCode::OwnerNotLive,
            AllocatorReasonCode::PolicyDenied,
            AllocatorReasonCode::UnsupportedKind,
            AllocatorReasonCode::StorageContractViolation,
            AllocatorReasonCode::KernelStateUnknown,
        ];
        assert_eq!(reasons.len(), ALLOCATOR_REASON_CODE_COUNT);
        assert!(ALLOCATOR_REASON_CODE_COUNT <= 16);
        let labels = reasons
            .iter()
            .map(|reason| reason.as_metric_label())
            .collect::<BTreeSet<_>>();
        assert_eq!(labels.len(), ALLOCATOR_REASON_CODE_COUNT);
        assert!(labels.iter().all(|label| !label.is_empty()));
    }

    #[test]
    fn total_acquisition_order_is_stable() {
        let request = LeaseAllocationRequest {
            operation_id: op("op-1"),
            correlation_id: corr("corr-1"),
            idempotency_key: IdempotencyKey::parse("idem-1").unwrap(),
            owner: owner(),
            resources: vec![
                request_resource("tap-1", HostResourceKind::Tap, 2),
                request_resource("bridge-1", HostResourceKind::Bridge, 1),
                request_resource("cgroup-1", HostResourceKind::CgroupSubtree, 1),
            ],
            trace: None,
        };

        let ordered = request.acquisition_order();
        assert_eq!(
            ordered
                .iter()
                .map(|key| key.resource_id.as_str())
                .collect::<Vec<_>>(),
            vec!["bridge-1", "cgroup-1", "tap-1"]
        );

        let mut reversed = request.clone();
        reversed.resources.reverse();
        assert_eq!(request.acquisition_order(), reversed.acquisition_order());
    }

    #[test]
    fn lease_state_transitions_fail_closed() {
        assert!(AllocatorLeaseState::Granted.can_transition_to(AllocatorLeaseState::Reconciled));
        assert!(
            AllocatorLeaseState::Reconciled.can_transition_to(AllocatorLeaseState::Quarantined)
        );
        assert!(AllocatorLeaseState::Quarantined.can_transition_to(AllocatorLeaseState::Reclaimed));
        assert!(!AllocatorLeaseState::Denied.can_transition_to(AllocatorLeaseState::Granted));
        assert!(!AllocatorLeaseState::Reclaimed.can_transition_to(AllocatorLeaseState::Reconciled));
        assert!(AllocatorLeaseState::Denied.is_terminal());
        assert!(AllocatorLeaseState::Reclaimed.is_terminal());
    }

    #[test]
    fn denied_result_is_strict_and_conflict_bounded() {
        let ok = json!({
            "status": "denied",
            "reason": "resource-conflict",
            "conflicts": [{
                "resource_id": "bridge-1",
                "kind": "bridge",
                "reason": "resource-conflict",
                "existing_lease": "lease-1"
            }]
        });
        assert!(serde_json::from_value::<LeaseAllocationResult>(ok).is_ok());

        let no_reason = json!({"status": "denied", "conflicts": []});
        assert!(serde_json::from_value::<LeaseAllocationResult>(no_reason).is_err());

        let contradictory = json!({
            "status": "granted",
            "lease": {
                "lease_id": "lease-1",
                "owner": {
                    "realm": ["work"],
                    "controller_generation": "gen-1",
                    "node": "realm-node"
                },
                "state": "granted",
                "resources": [{
                    "resource_id": "bridge-1",
                    "kind": "bridge",
                    "share": "exclusive",
                    "delegation": { "kind": "opaque-name", "id": "bridge-1" },
                    "acquisition_order": { "phase": 1, "ordinal": 0 }
                }]
            },
            "conflicts": []
        });
        assert!(serde_json::from_value::<LeaseAllocationResult>(contradictory).is_err());

        let contradictory_null = json!({
            "status": "granted",
            "lease": {
                "lease_id": "lease-1",
                "owner": {
                    "realm": ["work"],
                    "controller_generation": "gen-1",
                    "node": "realm-node"
                },
                "state": "granted",
                "resources": [{
                    "resource_id": "bridge-1",
                    "kind": "bridge",
                    "share": "exclusive",
                    "delegation": { "kind": "opaque-name", "id": "bridge-1" },
                    "acquisition_order": { "phase": 1, "ordinal": 0 }
                }]
            },
            "conflicts": null
        });
        assert!(serde_json::from_value::<LeaseAllocationResult>(contradictory_null).is_err());

        let denied_null_conflicts = json!({
            "status": "denied",
            "reason": "resource-conflict",
            "conflicts": null
        });
        assert!(serde_json::from_value::<LeaseAllocationResult>(denied_null_conflicts).is_err());

        let denied_missing_conflicts = json!({
            "status": "denied",
            "reason": "resource-conflict"
        });
        assert!(serde_json::from_value::<LeaseAllocationResult>(denied_missing_conflicts).is_err());

        let conflicts = (0..=MAX_ALLOCATOR_CONFLICTS)
            .map(|i| {
                json!({
                    "resource_id": format!("bridge-{i}"),
                    "kind": "bridge",
                    "reason": "resource-conflict"
                })
            })
            .collect::<Vec<_>>();
        let too_many = json!({
            "status": "denied",
            "reason": "resource-conflict",
            "conflicts": conflicts
        });
        assert!(serde_json::from_value::<LeaseAllocationResult>(too_many).is_err());
    }

    #[test]
    fn reconciliation_keeps_persisted_and_observed_state_distinct() {
        let record = ReconciliationRecord {
            resource_id: id("bridge-1"),
            kind: HostResourceKind::Bridge,
            persisted: Some(PersistedResourceLease {
                lease_id: lease_id("lease-1"),
                owner: owner(),
                state: AllocatorLeaseState::Granted,
            }),
            observed: ObservedHostResource {
                resource_id: id("bridge-1"),
                kind: HostResourceKind::Bridge,
                source: ResourceObservationSource::KernelNetlink,
                state: ObservedResourceState::ForeignOwner,
            },
            decision: ReconciliationDecision::Quarantine {
                reason: AllocatorReasonCode::DriftDetected,
            },
        };

        assert!(record.decision.is_fail_closed());
        let encoded = serde_json::to_value(&record).unwrap();
        assert!(encoded.get("persisted").is_some());
        assert!(encoded.get("observed").is_some());
        assert_eq!(encoded["observed"]["source"], "kernel-netlink");
    }

    #[test]
    fn reconciliation_decision_rejects_contradictory_fields() {
        let contradictory = json!({
            "decision": "reconciled",
            "reason": "drift-detected"
        });
        assert!(serde_json::from_value::<ReconciliationDecision>(contradictory).is_err());

        let null_reason = json!({
            "decision": "reconciled",
            "reason": null
        });
        assert!(serde_json::from_value::<ReconciliationDecision>(null_reason).is_err());

        let quarantine_missing_reason = json!({"decision": "quarantine"});
        assert!(
            serde_json::from_value::<ReconciliationDecision>(quarantine_missing_reason).is_err()
        );
    }

    #[test]
    fn reconciliation_report_is_bounded() {
        let record = ReconciliationRecord {
            resource_id: id("bridge-1"),
            kind: HostResourceKind::Bridge,
            persisted: None,
            observed: ObservedHostResource {
                resource_id: id("bridge-1"),
                kind: HostResourceKind::Bridge,
                source: ResourceObservationSource::KernelNetlink,
                state: ObservedResourceState::Missing,
            },
            decision: ReconciliationDecision::Reclaim {
                reason: AllocatorReasonCode::OwnerNotLive,
            },
        };
        let report = ReconciliationReport {
            operation_id: op("op-1"),
            correlation_id: corr("corr-1"),
            records: vec![record.clone(); MAX_RECONCILIATION_RECORDS + 1],
            events: Vec::new(),
        };
        let encoded = serde_json::to_value(report).unwrap();
        assert!(serde_json::from_value::<ReconciliationReport>(encoded).is_err());
    }
}
