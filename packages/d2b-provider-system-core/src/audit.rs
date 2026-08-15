//! Redacted Host/User reconciliation audit records.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Reconciled ResourceType owned by system-core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum ReconciledResourceType {
    /// Host resource.
    Host,
    /// User resource.
    User,
}

/// Bounded reconciliation outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReconcileOutcome {
    /// Status converged.
    Converged,
    /// Status is degraded.
    Degraded,
    /// Reconcile failed.
    Failed,
}

/// One redacted ResourceReconciled event.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ResourceReconciledAudit {
    /// Fixed audit record class.
    pub record_class: &'static str,
    /// ResourceType, never the raw name.
    pub resource_type: ReconciledResourceType,
    /// SHA-256 of type/name identity.
    pub resource_name_digest: String,
    /// Stable outcome.
    pub outcome: ReconcileOutcome,
    /// Exact hyphenated Zone handler name.
    pub handler: &'static str,
    /// Bounded generic condition summary.
    pub conditions_summary: &'static str,
}

impl core::fmt::Debug for ResourceReconciledAudit {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ResourceReconciledAudit(<redacted>)")
    }
}

impl ResourceReconciledAudit {
    /// Build a Host reconciliation record.
    pub fn host(
        resource_name: &str,
        outcome: ReconcileOutcome,
        conditions_summary: &'static str,
    ) -> Self {
        Self::new(
            ReconciledResourceType::Host,
            resource_name,
            outcome,
            crate::SYSTEM_CORE_HOST_HANDLER,
            conditions_summary,
        )
    }

    /// Build a User reconciliation record.
    pub fn user(
        resource_name: &str,
        outcome: ReconcileOutcome,
        conditions_summary: &'static str,
    ) -> Self {
        Self::new(
            ReconciledResourceType::User,
            resource_name,
            outcome,
            crate::SYSTEM_CORE_USER_HANDLER,
            conditions_summary,
        )
    }

    fn new(
        resource_type: ReconciledResourceType,
        resource_name: &str,
        outcome: ReconcileOutcome,
        handler: &'static str,
        conditions_summary: &'static str,
    ) -> Self {
        let type_name = match resource_type {
            ReconciledResourceType::Host => "Host",
            ReconciledResourceType::User => "User",
        };
        let mut hasher = Sha256::new();
        hasher.update(type_name.as_bytes());
        hasher.update(b"/");
        hasher.update(resource_name.as_bytes());
        let digest = hasher.finalize();
        let resource_name_digest = format!("sha256:{digest:x}");
        Self {
            record_class: "resource-reconciled",
            resource_type,
            resource_name_digest,
            outcome,
            handler,
            conditions_summary,
        }
    }
}
