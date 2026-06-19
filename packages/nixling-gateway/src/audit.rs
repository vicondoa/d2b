//! Gateway-local audit seam for ADR 0032 P0.
//!
//! The gateway orchestrator records only bounded metadata: operation id, realm,
//! principal, node, workload, authorization scope, decision, session id, and
//! lifecycle state. It never records argv, stdio/log bytes, Wayland buffers,
//! relay tokens, session secrets, socket paths, or provider error strings.

use crate::{DisplaySessionId, GatewayError, SessionState};
use nixling_constellation_core::{
    AuditEnvelope, AuthorizationScope, AuthzDecision, Capability, NodeId, OperationId, PrincipalId,
    RealmPath, WorkloadId,
};

/// Low-cardinality gateway audit event kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayAuditKind {
    /// A display-session open operation was admitted.
    DisplaySessionOpenAdmitted,
    /// A display-session open operation was denied/refused.
    DisplaySessionOpenDenied,
    /// The display session reached Running (handshake verified; bytes flowing).
    DisplaySessionRunning,
    /// The display session closed.
    DisplaySessionClosed,
}

/// Redacted metadata for one gateway audit event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayAuditEvent {
    /// Event kind.
    pub kind: GatewayAuditKind,
    /// Typed metadata envelope. Carries no payload bytes.
    pub envelope: AuditEnvelope,
    /// Correlated display session id, if the event has one.
    pub session_id: Option<DisplaySessionId>,
    /// Session state at the event boundary.
    pub state: Option<SessionState>,
    /// Stable error slug for denied/refused events. Never a raw provider string.
    pub error_slug: Option<&'static str>,
}

/// Audit sink dependency. Implementations must be fail-closed: return
/// [`GatewayError::AuditUnavailable`] when the event cannot be durably recorded.
pub trait GatewayAudit: Send + Sync {
    /// Record one event.
    fn record(&self, event: GatewayAuditEvent) -> Result<(), GatewayError>;
}

/// No-op audit sink used by tests and by production wiring until nixlingd
/// supplies a durable gateway JSONL sink. It is explicit so the dependency is
/// still wired and tests can swap in a recording sink.
#[derive(Debug, Default)]
pub struct NoopGatewayAudit;

impl GatewayAudit for NoopGatewayAudit {
    fn record(&self, _event: GatewayAuditEvent) -> Result<(), GatewayError> {
        Ok(())
    }
}

/// Build the audit envelope for display-session open/running/close events.
pub fn display_envelope(
    operation_id: OperationId,
    realm: RealmPath,
    principal: PrincipalId,
    node: NodeId,
    workload: WorkloadId,
    decision: AuthzDecision,
) -> AuditEnvelope {
    AuditEnvelope::post_auth(
        operation_id,
        realm,
        principal,
        node,
        AuthorizationScope::capability(Capability::WindowForwarding),
        decision,
    )
    .with_workload(workload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_constellation_core::{RealmId, WorkloadId};

    #[test]
    fn display_envelope_is_redacted_and_principal_consistent() {
        let realm = RealmPath::new(vec![RealmId::parse("work").unwrap()]).unwrap();
        let env = display_envelope(
            OperationId::parse("op-1").unwrap(),
            realm,
            PrincipalId::parse("alice").unwrap(),
            NodeId::parse("gateway").unwrap(),
            WorkloadId::parse("demo").unwrap(),
            AuthzDecision::Allow,
        );
        assert!(env.is_principal_consistent());
        assert_eq!(
            env.scope,
            AuthorizationScope::capability(Capability::WindowForwarding)
        );
        assert_eq!(env.workload.as_ref().map(|w| w.as_str()), Some("demo"));
        let rendered = format!("{env:?}");
        assert!(!rendered.contains("argv"));
        assert!(!rendered.contains("wayland"));
        assert!(!rendered.contains("SharedAccessKey"));
    }
}
