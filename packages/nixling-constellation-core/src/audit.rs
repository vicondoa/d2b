//! Audit envelope (ADR 0032). Every mutating operation and every
//! stream-open carries a redacted audit context. Realm, authorization
//! scope, and (post-auth) principal are mandatory. Admission records
//! (pre-auth) may have an absent principal. The envelope carries **only
//! bounded metadata** — never argv, stdio, log bytes, Wayland buffers,
//! secrets, or store paths.

use crate::capability::Capability;
use crate::ids::{ExecutionId, NodeId, OperationId, PrincipalId, StreamId, WorkloadId};
use crate::realm::RealmPath;
use crate::trace_context::TraceContext;
use serde::{Deserialize, Deserializer, Serialize};

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

/// What an audited operation was authorized against. Workload operations
/// require a [`Capability`]; node-control, enrollment, and read-only health
/// operations are authorized by node enrollment / session identity, so they
/// have their own scope rather than a synthetic capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "scope", rename_all = "kebab-case")]
pub enum AuthorizationScope {
    /// A workload capability (lifecycle/exec/display/…).
    Capability {
        /// The required capability.
        capability: Capability,
    },
    /// Node-control (register/heartbeat/capabilities).
    NodeControl,
    /// Realm/node enrollment.
    Enrollment,
    /// Read-only health probe.
    Health,
}

impl AuthorizationScope {
    /// Build a capability scope.
    pub fn capability(capability: Capability) -> Self {
        AuthorizationScope::Capability { capability }
    }
}

/// A redacted audit envelope for one mutating operation or stream-open.
///
/// Construct via [`AuditEnvelope::admission_denial`] (pre-auth; principal
/// may be absent because no principal has been authenticated yet) or
/// [`AuditEnvelope::post_auth`] (the principal is mandatory). The
/// invariant — an `Allow` decision always names a principal — is enforced
/// at decode (fail-closed `Deserialize`), not merely documented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditEnvelope {
    /// Stable audit id for this event.
    pub operation_id: OperationId,
    /// Realm path (mandatory; supports nested realms).
    pub realm: RealmPath,
    /// Authenticated principal. Absent only for pre-auth admission
    /// denials.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub principal: Option<PrincipalId>,
    /// Target node.
    pub node: NodeId,
    /// Target workload, when the operation has one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub workload: Option<WorkloadId>,
    /// Correlated stream, for stream-open audit events.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stream: Option<StreamId>,
    /// Correlated execution, for exec audit events.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub execution: Option<ExecutionId>,
    /// Authorization scope the operation/stream required (mandatory).
    pub scope: AuthorizationScope,
    /// Authorization decision.
    pub decision: AuthzDecision,
    /// Bounded trace context for correlation.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trace: Option<TraceContext>,
}

impl AuditEnvelope {
    /// A pre-auth admission denial: no principal has been authenticated, so
    /// `principal` is absent and the decision is `Deny`.
    pub fn admission_denial(
        operation_id: OperationId,
        realm: RealmPath,
        node: NodeId,
        scope: AuthorizationScope,
    ) -> Self {
        Self {
            operation_id,
            realm,
            principal: None,
            node,
            workload: None,
            stream: None,
            execution: None,
            scope,
            decision: AuthzDecision::Deny,
            trace: None,
        }
    }

    /// A post-auth audit record: the principal is mandatory (it is a value,
    /// not an `Option`), so an authorized event can never be recorded
    /// without an accountable principal.
    pub fn post_auth(
        operation_id: OperationId,
        realm: RealmPath,
        principal: PrincipalId,
        node: NodeId,
        scope: AuthorizationScope,
        decision: AuthzDecision,
    ) -> Self {
        Self {
            operation_id,
            realm,
            principal: Some(principal),
            node,
            workload: None,
            stream: None,
            execution: None,
            scope,
            decision,
            trace: None,
        }
    }

    /// Attach a correlated workload (builder style).
    pub fn with_workload(mut self, workload: WorkloadId) -> Self {
        self.workload = Some(workload);
        self
    }

    /// Attach a correlated stream (builder style).
    pub fn with_stream(mut self, stream: StreamId) -> Self {
        self.stream = Some(stream);
        self
    }

    /// Attach a correlated execution (builder style).
    pub fn with_execution(mut self, execution: ExecutionId) -> Self {
        self.execution = Some(execution);
        self
    }

    /// Attach a trace context (builder style).
    pub fn with_trace(mut self, trace: TraceContext) -> Self {
        self.trace = Some(trace);
        self
    }

    /// The audit invariant: an `Allow` decision MUST name a principal.
    pub fn is_principal_consistent(&self) -> bool {
        match self.decision {
            AuthzDecision::Allow => self.principal.is_some(),
            AuthzDecision::Deny => true,
        }
    }
}

// Fail-closed decode: an `Allow` audit record without an accountable
// principal is rejected at the boundary.
impl<'de> Deserialize<'de> for AuditEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            operation_id: OperationId,
            realm: RealmPath,
            #[serde(default)]
            principal: Option<PrincipalId>,
            node: NodeId,
            #[serde(default)]
            workload: Option<WorkloadId>,
            #[serde(default)]
            stream: Option<StreamId>,
            #[serde(default)]
            execution: Option<ExecutionId>,
            scope: AuthorizationScope,
            decision: AuthzDecision,
            #[serde(default)]
            trace: Option<TraceContext>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let env = AuditEnvelope {
            operation_id: raw.operation_id,
            realm: raw.realm,
            principal: raw.principal,
            node: raw.node,
            workload: raw.workload,
            stream: raw.stream,
            execution: raw.execution,
            scope: raw.scope,
            decision: raw.decision,
            trace: raw.trace,
        };
        if env.is_principal_consistent() {
            Ok(env)
        } else {
            Err(serde::de::Error::custom(
                "audit record with decision=allow must name a principal",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{NodeId, OperationId, RealmId};

    fn realm(label: &str) -> RealmPath {
        RealmPath::new(vec![RealmId::parse(label).unwrap()]).unwrap()
    }

    #[test]
    fn admission_denial_may_omit_principal() {
        let env = AuditEnvelope::admission_denial(
            OperationId::parse("op-1").unwrap(),
            realm("work"),
            NodeId::parse("gw").unwrap(),
            AuthorizationScope::capability(Capability::Lifecycle),
        );
        let json = serde_json::to_string(&env).unwrap();
        assert!(!json.contains("principal"));
        assert!(json.contains("\"decision\":\"deny\""));
        assert!(env.is_principal_consistent());
    }

    #[test]
    fn node_control_op_audits_without_a_capability() {
        let env = AuditEnvelope::post_auth(
            OperationId::parse("op-nr").unwrap(),
            realm("work"),
            PrincipalId::parse("principal-1").unwrap(),
            NodeId::parse("gw").unwrap(),
            AuthorizationScope::NodeControl,
            AuthzDecision::Allow,
        );
        assert!(env.is_principal_consistent());
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"scope\":\"node-control\""));
    }

    #[test]
    fn deserialize_rejects_allow_without_principal() {
        // Allow without principal is rejected fail-closed at decode.
        let json = "{\"operation_id\":\"op-1\",\"realm\":[\"work\"],\"node\":\"gw\",\
                    \"scope\":{\"scope\":\"node-control\"},\"decision\":\"allow\"}";
        assert!(serde_json::from_str::<AuditEnvelope>(json).is_err());
        // The same record as a Deny decodes fine.
        let json_deny = "{\"operation_id\":\"op-1\",\"realm\":[\"work\"],\"node\":\"gw\",\
                         \"scope\":{\"scope\":\"node-control\"},\"decision\":\"deny\"}";
        assert!(serde_json::from_str::<AuditEnvelope>(json_deny).is_ok());
    }
}
