//! Resource-create quota admission.
//!
//! This gate is pure and runs before a resource mutation is prepared.  It
//! never rewrites a caller's `quotaRef`; a dependent must be reassigned or
//! deleted by an authorized caller before a Quota can drain.

use d2b_contracts::v3::{
    QuotaSpec, ResourceTypeName,
    quota::{QuotaContractError, QuotaEnforcementPolicy},
};

/// Resource usage presented by the Zone store's read snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuotaUsage {
    /// Non-deleted resources currently counted.
    pub resources: u64,
    /// Resources of the requested type currently counted.
    pub resources_of_type: u64,
    /// Aggregate CPU units currently counted.
    pub cpu: Option<u64>,
    /// Aggregate memory MiB currently counted.
    pub memory_mib: Option<u64>,
    /// Aggregate storage GiB currently counted.
    pub storage_gib: Option<u64>,
    /// Resources currently referencing this Quota.
    pub dependents: u64,
}

/// Requested resource usage for one create admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuotaRequest {
    /// Requested CPU units.
    pub cpu: Option<u64>,
    /// Requested memory MiB.
    pub memory_mib: Option<u64>,
    /// Requested storage GiB.
    pub storage_gib: Option<u64>,
    /// Whether the new resource carries this Quota's `quotaRef`.
    pub references_quota: bool,
}

/// Result of a quota check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaDecision {
    /// Whether the create may proceed.
    pub admitted: bool,
    /// Whether a soft policy observed an overage.
    pub over_quota: bool,
    /// Current dependent count, for status reconciliation.
    pub dependent_count: u64,
}

/// Typed quota-gate refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaGateError {
    /// The request exceeds a hard ceiling.
    Exceeded,
    /// The ResourceType was not present in the quota's catalog.
    UnknownResourceType,
    /// The quota contract was malformed.
    InvalidQuota,
}

impl core::fmt::Display for QuotaGateError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Exceeded => "quota-exceeded",
            Self::UnknownResourceType => "quota-resource-type-unknown",
            Self::InvalidQuota => "quota-invalid",
        })
    }
}

impl std::error::Error for QuotaGateError {}

/// Check one create against a Quota's hard or soft ceilings.
pub fn check_create(
    quota: &QuotaSpec,
    resource_type: &ResourceTypeName,
    usage: QuotaUsage,
    request: QuotaRequest,
) -> Result<QuotaDecision, QuotaGateError> {
    let per_type = quota
        .per_type_ceilings()
        .get(resource_type)
        .and_then(|ceiling| ceiling.max_resources());
    let per_type_cpu = quota
        .per_type_ceilings()
        .get(resource_type)
        .and_then(|ceiling| ceiling.max_cpu());
    let per_type_memory = quota
        .per_type_ceilings()
        .get(resource_type)
        .and_then(|ceiling| ceiling.max_memory_mib());
    let per_type_storage = quota
        .per_type_ceilings()
        .get(resource_type)
        .and_then(|ceiling| ceiling.max_storage_gib());
    let total_over = usage.resources.saturating_add(1) > quota.ceilings().max_resources();
    let type_over = usage.resources_of_type.saturating_add(1)
        > per_type.unwrap_or(quota.ceilings().max_resources_per_type());
    let cpu_over = per_type_cpu
        .or(quota.ceilings().max_cpu())
        .zip(usage.cpu)
        .zip(request.cpu)
        .is_some_and(|((ceiling, used), requested)| used.saturating_add(requested) > ceiling);
    let memory_over = per_type_memory
        .or(quota.ceilings().max_memory_mib())
        .zip(usage.memory_mib)
        .zip(request.memory_mib)
        .is_some_and(|((ceiling, used), requested)| used.saturating_add(requested) > ceiling);
    let storage_over = per_type_storage
        .or(quota.ceilings().max_storage_gib())
        .zip(usage.storage_gib)
        .zip(request.storage_gib)
        .is_some_and(|((ceiling, used), requested)| used.saturating_add(requested) > ceiling);
    let over_quota = total_over || type_over || cpu_over || memory_over || storage_over;
    if over_quota && quota.enforcement_policy() == QuotaEnforcementPolicy::Hard {
        return Err(QuotaGateError::Exceeded);
    }
    Ok(QuotaDecision {
        admitted: true,
        over_quota,
        dependent_count: usage.dependents + u64::from(request.references_quota),
    })
}

/// Build the status dependent count without changing any dependent resource.
pub const fn dependent_count(usage: QuotaUsage) -> u64 {
    usage.dependents
}

impl From<QuotaContractError> for QuotaGateError {
    fn from(_: QuotaContractError) -> Self {
        Self::InvalidQuota
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::{
        QuotaCeilings, QuotaEnforcementPolicy, QuotaScope, QuotaSpec, QuotaTypeCeiling,
    };
    use std::collections::BTreeMap;

    fn quota(policy: QuotaEnforcementPolicy) -> QuotaSpec {
        QuotaSpec::new(
            QuotaCeilings::new(1, 1, 8, None, None, None).unwrap(),
            BTreeMap::new(),
            QuotaScope::Zone,
            policy,
        )
        .unwrap()
    }

    #[test]
    fn hard_quota_rejects_before_mutation() {
        assert_eq!(
            check_create(
                &quota(QuotaEnforcementPolicy::Hard),
                &ResourceTypeName::parse("Process").unwrap(),
                QuotaUsage {
                    resources: 1,
                    ..QuotaUsage::default()
                },
                QuotaRequest::default(),
            ),
            Err(QuotaGateError::Exceeded)
        );
    }

    #[test]
    fn soft_quota_warns_but_admits() {
        let decision = check_create(
            &quota(QuotaEnforcementPolicy::Soft),
            &ResourceTypeName::parse("Process").unwrap(),
            QuotaUsage {
                resources: 1,
                ..QuotaUsage::default()
            },
            QuotaRequest::default(),
        )
        .unwrap();
        assert!(decision.admitted);
        assert!(decision.over_quota);
    }

    #[test]
    fn per_type_ceiling_is_used_when_present() {
        let mut per_type = BTreeMap::new();
        per_type.insert(
            ResourceTypeName::parse("Process").unwrap(),
            QuotaTypeCeiling::new(Some(1), None, None, None).unwrap(),
        );
        let quota = QuotaSpec::new(
            QuotaCeilings::new(10, 10, 8, None, None, None).unwrap(),
            per_type,
            QuotaScope::Zone,
            QuotaEnforcementPolicy::Hard,
        )
        .unwrap();
        assert_eq!(
            check_create(
                &quota,
                &ResourceTypeName::parse("Process").unwrap(),
                QuotaUsage {
                    resources_of_type: 1,
                    ..QuotaUsage::default()
                },
                QuotaRequest::default(),
            ),
            Err(QuotaGateError::Exceeded)
        );
    }
}
