//! Audit envelope (ADR 0032). Every mutating operation and every
//! stream-open carries a redacted audit context. Realm, capability, and
//! (post-auth) principal are mandatory. Admission records (pre-auth) may
//! have an absent principal. The envelope carries **only bounded
//! metadata** — never argv, stdio, log bytes, Wayland buffers, secrets,
//! or store paths.

use crate::capability::Capability;
use crate::ids::{NodeId, OperationId, PrincipalId, RealmId, WorkloadId};
use crate::trace_context::TraceContext;
use serde::{Deserialize, Serialize};

/// The authorization decision recorded for an operation/stream.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AuthzDecision {
    /// The operation/stream was authorized.
    Allow,
    /// The operation/stream was refused (admission denial).
    Deny,
}

/// A redacted audit envelope for one mutating operation or stream-open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditEnvelope {
    /// Audit/correlation id.
    pub operation_id: OperationId,
    /// Realm (mandatory).
    pub realm: RealmId,
    /// Authenticated principal. Absent only for pre-auth admission
    /// denials.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub principal: Option<PrincipalId>,
    /// Target node.
    pub node: NodeId,
    /// Target workload, when the operation has one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub workload: Option<WorkloadId>,
    /// Capability the operation/stream required (mandatory).
    pub capability: Capability,
    /// Authorization decision.
    pub decision: AuthzDecision,
    /// Bounded trace context for correlation.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trace: Option<TraceContext>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{NodeId, OperationId, RealmId};

    #[test]
    fn admission_denial_may_omit_principal() {
        let env = AuditEnvelope {
            operation_id: OperationId::parse("op-1").unwrap(),
            realm: RealmId::parse("work").unwrap(),
            principal: None,
            node: NodeId::parse("gw").unwrap(),
            workload: None,
            capability: Capability::Lifecycle,
            decision: AuthzDecision::Deny,
            trace: None,
        };
        let json = serde_json::to_string(&env).unwrap();
        // Absent principal is omitted, mandatory fields are present.
        assert!(!json.contains("principal"));
        assert!(json.contains("\"realm\":\"work\""));
        assert!(json.contains("\"decision\":\"deny\""));
    }
}
