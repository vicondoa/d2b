//! Audit records (ADR 0032). Every mutating operation and every stream-open
//! carries a redacted post-auth audit envelope with mandatory realm,
//! authorization scope, and principal context. Pre-auth admission failures use
//! a separate redacted admission record; only that shape may omit a principal.
//! These records carry **only bounded metadata** — never argv, stdio, log
//! bytes, Wayland buffers, secrets, or store paths.

use crate::capability::Capability;
use crate::error::ErrorKind;
use crate::ids::{ExecutionId, NodeId, OperationId, PrincipalId, StreamId, WorkloadId};
use crate::realm::RealmPath;
use crate::trace_context::TraceContext;
use serde::{Deserialize, Deserializer, Serialize};

/// The authorization decision recorded for an operation/stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
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

impl<'de> Deserialize<'de> for AuthorizationScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case")]
        enum ScopeTag {
            Capability,
            NodeControl,
            Enrollment,
            Health,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            scope: ScopeTag,
            #[serde(default)]
            capability: Option<Capability>,
        }

        let raw = Raw::deserialize(deserializer)?;
        match raw.scope {
            ScopeTag::Capability => raw
                .capability
                .map(AuthorizationScope::capability)
                .ok_or_else(|| {
                    serde::de::Error::custom("capability authorization scope needs capability")
                }),
            ScopeTag::NodeControl => {
                reject_scope_capability(raw.capability, AuthorizationScope::NodeControl)
            }
            ScopeTag::Enrollment => {
                reject_scope_capability(raw.capability, AuthorizationScope::Enrollment)
            }
            ScopeTag::Health => reject_scope_capability(raw.capability, AuthorizationScope::Health),
        }
    }
}

fn reject_scope_capability<E>(
    capability: Option<Capability>,
    scope: AuthorizationScope,
) -> Result<AuthorizationScope, E>
where
    E: serde::de::Error,
{
    if capability.is_some() {
        Err(E::custom(
            "non-capability authorization scope must not carry capability",
        ))
    } else {
        Ok(scope)
    }
}

/// A redacted post-auth audit envelope for one mutating operation or
/// stream-open.
///
/// Construct via [`AuditEnvelope::post_auth`]. The principal is a mandatory
/// value, not an `Option`, so an operation/stream audit event can never be
/// recorded without an accountable authenticated principal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditEnvelope {
    /// Stable audit id for this event.
    pub operation_id: OperationId,
    /// Realm path (mandatory; supports nested realms).
    pub realm: RealmPath,
    /// Authenticated principal (never a relay credential).
    pub principal: PrincipalId,
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
    /// A post-auth audit record: the principal is mandatory, so an event can
    /// never be recorded without an accountable principal.
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
            principal,
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

    /// Compatibility helper for callers that check the old Option-based
    /// invariant. Post-auth envelopes are always principal-consistent because
    /// the principal is mandatory by type.
    pub fn is_principal_consistent(&self) -> bool {
        true
    }
}

// Fail-closed decode: post-auth audit records require an accountable
// principal at the boundary. Pre-auth denial records use
// [`AdmissionAuditRecord`] instead.
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
            principal: PrincipalId,
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
        Ok(env)
    }
}

/// A redacted pre-auth/session-admission denial record. This is separate from
/// [`AuditEnvelope`] because no authenticated principal may exist yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdmissionAuditRecord {
    /// Stable audit/correlation id for this event.
    pub operation_id: OperationId,
    /// Realm path for the attempted admission.
    pub realm: RealmPath,
    /// Authenticated principal, when admission failed after a principal was
    /// bound. Absent for pre-auth denials.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub principal: Option<PrincipalId>,
    /// Node that denied admission.
    pub node: NodeId,
    /// Authorization/admission scope that was evaluated.
    pub scope: AuthorizationScope,
    /// Admission decision. Admission records are denials only.
    pub decision: AuthzDecision,
    /// Closed, redacted reason code.
    pub reason: ErrorKind,
    /// Bounded trace context for correlation.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trace: Option<TraceContext>,
}

impl AdmissionAuditRecord {
    /// Build a pre-auth/session-admission denial.
    pub fn denied(
        operation_id: OperationId,
        realm: RealmPath,
        node: NodeId,
        scope: AuthorizationScope,
        reason: ErrorKind,
    ) -> Self {
        Self {
            operation_id,
            realm,
            principal: None,
            node,
            scope,
            decision: AuthzDecision::Deny,
            reason,
            trace: None,
        }
    }

    /// Attach a principal when the denial happened after principal binding.
    pub fn with_principal(mut self, principal: PrincipalId) -> Self {
        self.principal = Some(principal);
        self
    }

    /// Attach a trace context (builder style).
    pub fn with_trace(mut self, trace: TraceContext) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Admission records are denial records only.
    pub fn is_admission_denial(&self) -> bool {
        self.decision == AuthzDecision::Deny
    }
}

impl<'de> Deserialize<'de> for AdmissionAuditRecord {
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
            scope: AuthorizationScope,
            decision: AuthzDecision,
            reason: ErrorKind,
            #[serde(default)]
            trace: Option<TraceContext>,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.decision != AuthzDecision::Deny {
            return Err(serde::de::Error::custom(
                "admission audit records must be denials",
            ));
        }
        Ok(Self {
            operation_id: raw.operation_id,
            realm: raw.realm,
            principal: raw.principal,
            node: raw.node,
            scope: raw.scope,
            decision: raw.decision,
            reason: raw.reason,
            trace: raw.trace,
        })
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
    fn admission_denial_record_may_omit_principal() {
        let env = AdmissionAuditRecord::denied(
            OperationId::parse("op-1").unwrap(),
            realm("work"),
            NodeId::parse("gw").unwrap(),
            AuthorizationScope::capability(Capability::Lifecycle),
            ErrorKind::AuthenticationFailed,
        );
        let json = serde_json::to_string(&env).unwrap();
        assert!(!json.contains("principal"));
        assert!(json.contains("\"decision\":\"deny\""));
        assert!(env.is_admission_denial());
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
        assert_eq!(env.principal.as_str(), "principal-1");
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"scope\":\"node-control\""));
    }

    #[test]
    fn deserialize_rejects_audit_envelope_without_principal() {
        let json = "{\"operation_id\":\"op-1\",\"realm\":[\"work\"],\"node\":\"gw\",\
                    \"scope\":{\"scope\":\"node-control\"},\"decision\":\"allow\"}";
        assert!(serde_json::from_str::<AuditEnvelope>(json).is_err());
        let json_deny = "{\"operation_id\":\"op-1\",\"realm\":[\"work\"],\"node\":\"gw\",\
                         \"scope\":{\"scope\":\"node-control\"},\"decision\":\"deny\"}";
        assert!(serde_json::from_str::<AuditEnvelope>(json_deny).is_err());
    }

    #[test]
    fn admission_audit_rejects_allow_and_unknown_scope_fields() {
        let allow = "{\"operation_id\":\"op-1\",\"realm\":[\"work\"],\"node\":\"gw\",\
                     \"scope\":{\"scope\":\"node-control\"},\"decision\":\"allow\",\
                     \"reason\":\"authentication-failed\"}";
        assert!(serde_json::from_str::<AdmissionAuditRecord>(allow).is_err());

        let extra_scope = "{\"operation_id\":\"op-1\",\"realm\":[\"work\"],\"node\":\"gw\",\
                           \"scope\":{\"scope\":\"node-control\",\"extra\":true},\
                           \"decision\":\"deny\",\"reason\":\"authentication-failed\"}";
        assert!(serde_json::from_str::<AdmissionAuditRecord>(extra_scope).is_err());

        let missing_capability = "{\"operation_id\":\"op-1\",\"realm\":[\"work\"],\
                                  \"node\":\"gw\",\"scope\":{\"scope\":\"capability\"},\
                                  \"decision\":\"deny\",\"reason\":\"authentication-failed\"}";
        assert!(serde_json::from_str::<AdmissionAuditRecord>(missing_capability).is_err());

        let capability_on_node_scope = "{\"operation_id\":\"op-1\",\"realm\":[\"work\"],\
                                        \"node\":\"gw\",\
                                        \"scope\":{\"scope\":\"node-control\",\"capability\":\"exec\"},\
                                        \"decision\":\"deny\",\"reason\":\"authentication-failed\"}";
        assert!(serde_json::from_str::<AdmissionAuditRecord>(capability_on_node_scope).is_err());
    }
}
