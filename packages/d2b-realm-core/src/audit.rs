//! Audit records (ADR 0032). Every mutating operation and every stream-open
//! carries a redacted post-auth audit envelope with mandatory realm,
//! authorization scope, and principal context. Pre-auth admission failures use
//! a separate redacted admission record; only that shape may omit a principal.
//! These records carry **only bounded metadata** — never argv, stdio, log
//! bytes, Wayland buffers, secrets, or store paths.

use crate::capability::Capability;
use crate::error::ErrorKind;
use crate::ids::{
    CorrelationId, ExecutionId, NodeId, OperationId, PrincipalId, StreamId, WorkloadId,
};
use crate::realm::RealmPath;
use crate::trace_context::TraceContext;
use serde::{Deserialize, Deserializer, Serialize};

const AUDIT_HASH_PREFIX: &str = "sha256:";
const AUDIT_HASH_HEX_LEN: usize = 64;

/// Closed audit stream kinds that can carry tamper-evident hash chains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditStreamKind {
    /// Gateway-guest audit stream.
    Gateway,
    /// Remote full-node audit stream.
    RemoteNode,
    /// Local daemon audit stream.
    Daemon,
}

/// Validation error for [`AuditHash`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditHashError {
    Empty,
    BadShape,
}

impl core::fmt::Display for AuditHashError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "audit hash is empty"),
            Self::BadShape => write!(f, "audit hash must be sha256:<64 lowercase hex chars>"),
        }
    }
}

impl std::error::Error for AuditHashError {}

/// Bounded SHA-256 hash marker used by audit-chain metadata.
///
/// This pure core crate defines the contract and validation shape; daemon,
/// gateway, or provider crates compute the canonical hashes with their local
/// hashing dependencies.
#[derive(Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct AuditHash(String);

impl AuditHash {
    /// Parse `sha256:<64 lowercase hex chars>`.
    pub fn parse(raw: impl Into<String>) -> Result<Self, AuditHashError> {
        let raw = raw.into();
        if raw.is_empty() {
            return Err(AuditHashError::Empty);
        }
        let Some(hex) = raw.strip_prefix(AUDIT_HASH_PREFIX) else {
            return Err(AuditHashError::BadShape);
        };
        if hex.len() != AUDIT_HASH_HEX_LEN
            || !hex
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
        {
            return Err(AuditHashError::BadShape);
        }
        Ok(Self(raw))
    }

    /// Borrow the canonical string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for AuditHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("AuditHash(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for AuditHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(raw).map_err(serde::de::Error::custom)
    }
}

/// One link in a tamper-evident audit stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditChainLink {
    /// Monotonic stream sequence number.
    pub sequence: u64,
    /// Previous record hash, or the stream's genesis hash for sequence 0.
    pub previous_hash: AuditHash,
    /// Hash of the canonical redacted audit payload.
    pub payload_hash: AuditHash,
    /// Hash of the canonical chain record.
    pub record_hash: AuditHash,
}

impl AuditChainLink {
    /// Construct a chain link from precomputed canonical hashes.
    pub fn new(
        sequence: u64,
        previous_hash: AuditHash,
        payload_hash: AuditHash,
        record_hash: AuditHash,
    ) -> Self {
        Self {
            sequence,
            previous_hash,
            payload_hash,
            record_hash,
        }
    }

    /// Verify this link against trusted recomputed hash values.
    pub fn verify(
        &self,
        expected_previous_hash: &AuditHash,
        canonical_payload_hash: &AuditHash,
        canonical_record_hash: &AuditHash,
    ) -> AuditChainCheckResult {
        if &self.previous_hash != expected_previous_hash {
            return AuditChainCheckResult::failed(AuditChainCheckFailure::PreviousHashMismatch);
        }
        if &self.payload_hash != canonical_payload_hash {
            return AuditChainCheckResult::failed(AuditChainCheckFailure::PayloadHashMismatch);
        }
        if &self.record_hash != canonical_record_hash {
            return AuditChainCheckResult::failed(AuditChainCheckFailure::RecordHashMismatch);
        }
        AuditChainCheckResult::verified()
    }
}

/// Redacted chain metadata for a gateway, remote-node, or daemon audit event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditChainRecord {
    pub stream: AuditStreamKind,
    pub realm: RealmPath,
    pub node: NodeId,
    pub operation_id: OperationId,
    pub link: AuditChainLink,
}

impl AuditChainRecord {
    /// Construct a redacted audit-chain record.
    pub fn new(
        stream: AuditStreamKind,
        realm: RealmPath,
        node: NodeId,
        operation_id: OperationId,
        link: AuditChainLink,
    ) -> Self {
        Self {
            stream,
            realm,
            node,
            operation_id,
            link,
        }
    }

    /// Verify this record's link against trusted recomputed hash values.
    pub fn verify(
        &self,
        expected_previous_hash: &AuditHash,
        canonical_payload_hash: &AuditHash,
        canonical_record_hash: &AuditHash,
    ) -> AuditChainCheckResult {
        self.link.verify(
            expected_previous_hash,
            canonical_payload_hash,
            canonical_record_hash,
        )
    }
}

/// Closed audit-chain verification failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditChainCheckFailure {
    PreviousHashMismatch,
    PayloadHashMismatch,
    RecordHashMismatch,
}

/// Verification result for one audit-chain link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum AuditChainCheckResult {
    Verified,
    Failed { failure: AuditChainCheckFailure },
}

impl AuditChainCheckResult {
    /// Successful chain verification.
    pub fn verified() -> Self {
        Self::Verified
    }

    /// Failed chain verification.
    pub fn failed(failure: AuditChainCheckFailure) -> Self {
        Self::Failed { failure }
    }

    /// True when the chain check failed closed.
    pub fn is_fail_closed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Bounded reason for audit retention-floor status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditRetentionFloorReason {
    WindowBelowFloor,
    EvidenceMissing,
    SinkUnavailable,
}

/// Retention-floor status for an audit sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum AuditRetentionFloorStatus {
    Met,
    BelowFloor { reason: AuditRetentionFloorReason },
    Unknown { reason: AuditRetentionFloorReason },
}

impl AuditRetentionFloorStatus {
    pub fn met() -> Self {
        Self::Met
    }

    pub fn below_floor(reason: AuditRetentionFloorReason) -> Self {
        Self::BelowFloor { reason }
    }

    pub fn unknown(reason: AuditRetentionFloorReason) -> Self {
        Self::Unknown { reason }
    }

    pub fn is_fail_closed(&self) -> bool {
        !matches!(self, Self::Met)
    }
}

/// Bounded reason for audit sink health.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AuditSinkHealthReason {
    Backpressure,
    ChainVerificationFailed,
    SinkMissing,
    SinkUnavailable,
    WriteFailed,
}

/// Health state for a gateway/remote-node/daemon audit sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum AuditSinkHealth {
    Ok {
        stream: AuditStreamKind,
        retention_floor: AuditRetentionFloorStatus,
    },
    Degraded {
        stream: AuditStreamKind,
        reason: AuditSinkHealthReason,
        retention_floor: AuditRetentionFloorStatus,
    },
    Unavailable {
        stream: AuditStreamKind,
        reason: AuditSinkHealthReason,
        retention_floor: AuditRetentionFloorStatus,
    },
}

impl AuditSinkHealth {
    pub fn ok(stream: AuditStreamKind, retention_floor: AuditRetentionFloorStatus) -> Self {
        Self::Ok {
            stream,
            retention_floor,
        }
    }

    pub fn degraded(
        stream: AuditStreamKind,
        reason: AuditSinkHealthReason,
        retention_floor: AuditRetentionFloorStatus,
    ) -> Self {
        Self::Degraded {
            stream,
            reason,
            retention_floor,
        }
    }

    pub fn unavailable(
        stream: AuditStreamKind,
        reason: AuditSinkHealthReason,
        retention_floor: AuditRetentionFloorStatus,
    ) -> Self {
        Self::Unavailable {
            stream,
            reason,
            retention_floor,
        }
    }

    pub fn is_degraded(&self) -> bool {
        matches!(self, Self::Degraded { .. } | Self::Unavailable { .. })
    }

    pub fn is_fail_closed(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
            || self.retention_floor().is_fail_closed()
            || matches!(
                self,
                Self::Degraded {
                    reason: AuditSinkHealthReason::ChainVerificationFailed,
                    ..
                }
            )
    }

    pub fn retention_floor(&self) -> &AuditRetentionFloorStatus {
        match self {
            Self::Ok {
                retention_floor, ..
            }
            | Self::Degraded {
                retention_floor, ..
            }
            | Self::Unavailable {
                retention_floor, ..
            } => retention_floor,
        }
    }
}

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
    /// Cross-realm correlation id shared across route and audit hops.
    pub correlation_id: CorrelationId,
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
        correlation_id: CorrelationId,
        realm: RealmPath,
        principal: PrincipalId,
        node: NodeId,
        scope: AuthorizationScope,
        decision: AuthzDecision,
    ) -> Self {
        Self {
            operation_id,
            correlation_id,
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
            correlation_id: CorrelationId,
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
            correlation_id: raw.correlation_id,
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
    /// Cross-realm correlation id shared across route and audit hops.
    pub correlation_id: CorrelationId,
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
        correlation_id: CorrelationId,
        realm: RealmPath,
        node: NodeId,
        scope: AuthorizationScope,
        reason: ErrorKind,
    ) -> Self {
        Self {
            operation_id,
            correlation_id,
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
            correlation_id: CorrelationId,
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
            correlation_id: raw.correlation_id,
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
    use crate::ids::{CorrelationId, NodeId, OperationId, RealmId};

    fn realm(label: &str) -> RealmPath {
        RealmPath::new(vec![RealmId::parse(label).unwrap()]).unwrap()
    }

    fn hash(ch: char) -> AuditHash {
        AuditHash::parse(format!("sha256:{}", ch.to_string().repeat(64))).unwrap()
    }

    fn chain_record() -> AuditChainRecord {
        AuditChainRecord::new(
            AuditStreamKind::Gateway,
            realm("work"),
            NodeId::parse("gateway").unwrap(),
            OperationId::parse("op-chain").unwrap(),
            AuditChainLink::new(7, hash('a'), hash('b'), hash('c')),
        )
    }

    fn corr() -> CorrelationId {
        CorrelationId::parse("corr-1").unwrap()
    }

    #[test]
    fn admission_denial_record_may_omit_principal() {
        let env = AdmissionAuditRecord::denied(
            OperationId::parse("op-1").unwrap(),
            corr(),
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
            corr(),
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
        let json = "{\"operation_id\":\"op-1\",\"correlation_id\":\"corr-1\",\"realm\":[\"work\"],\"node\":\"gw\",\
                    \"scope\":{\"scope\":\"node-control\"},\"decision\":\"allow\"}";
        assert!(serde_json::from_str::<AuditEnvelope>(json).is_err());
        let json_deny = "{\"operation_id\":\"op-1\",\"correlation_id\":\"corr-1\",\"realm\":[\"work\"],\"node\":\"gw\",\
                         \"scope\":{\"scope\":\"node-control\"},\"decision\":\"deny\"}";
        assert!(serde_json::from_str::<AuditEnvelope>(json_deny).is_err());
    }

    #[test]
    fn admission_audit_rejects_allow_and_unknown_scope_fields() {
        let allow = "{\"operation_id\":\"op-1\",\"correlation_id\":\"corr-1\",\"realm\":[\"work\"],\"node\":\"gw\",\
                     \"scope\":{\"scope\":\"node-control\"},\"decision\":\"allow\",\
                     \"reason\":\"authentication-failed\"}";
        assert!(serde_json::from_str::<AdmissionAuditRecord>(allow).is_err());

        let extra_scope = "{\"operation_id\":\"op-1\",\"correlation_id\":\"corr-1\",\"realm\":[\"work\"],\"node\":\"gw\",\
                           \"scope\":{\"scope\":\"node-control\",\"extra\":true},\
                           \"decision\":\"deny\",\"reason\":\"authentication-failed\"}";
        assert!(serde_json::from_str::<AdmissionAuditRecord>(extra_scope).is_err());

        let missing_capability = "{\"operation_id\":\"op-1\",\"correlation_id\":\"corr-1\",\"realm\":[\"work\"],\
                                  \"node\":\"gw\",\"scope\":{\"scope\":\"capability\"},\
                                  \"decision\":\"deny\",\"reason\":\"authentication-failed\"}";
        assert!(serde_json::from_str::<AdmissionAuditRecord>(missing_capability).is_err());

        let capability_on_node_scope = "{\"operation_id\":\"op-1\",\"correlation_id\":\"corr-1\",\"realm\":[\"work\"],\
                                        \"node\":\"gw\",\
                                        \"scope\":{\"scope\":\"node-control\",\"capability\":\"exec\"},\
                                        \"decision\":\"deny\",\"reason\":\"authentication-failed\"}";
        assert!(serde_json::from_str::<AdmissionAuditRecord>(capability_on_node_scope).is_err());
    }

    #[test]
    fn audit_records_require_bounded_correlation_id() {
        let without_corr = "{\"operation_id\":\"op-1\",\"realm\":[\"work\"],\"principal\":\"p1\",\
                           \"node\":\"gw\",\"scope\":{\"scope\":\"node-control\"},\"decision\":\"allow\"}";
        assert!(serde_json::from_str::<AuditEnvelope>(without_corr).is_err());

        let malformed_corr = "{\"operation_id\":\"op-1\",\"correlation_id\":\"../secret\",\
                              \"realm\":[\"work\"],\"principal\":\"p1\",\"node\":\"gw\",\
                              \"scope\":{\"scope\":\"node-control\"},\"decision\":\"allow\"}";
        assert!(serde_json::from_str::<AuditEnvelope>(malformed_corr).is_err());

        let admission_without_corr = "{\"operation_id\":\"op-1\",\"realm\":[\"work\"],\
                                     \"node\":\"gw\",\"scope\":{\"scope\":\"enrollment\"},\
                                     \"decision\":\"deny\",\"reason\":\"authentication-failed\"}";
        assert!(serde_json::from_str::<AdmissionAuditRecord>(admission_without_corr).is_err());
    }

    #[test]
    fn audit_hash_rejects_malformed_or_secret_shaped_values() {
        assert_eq!(AuditHash::parse("").unwrap_err(), AuditHashError::Empty);
        assert_eq!(
            AuditHash::parse("secret-token").unwrap_err(),
            AuditHashError::BadShape
        );
        assert_eq!(
            AuditHash::parse(
                "sha256:ABCDEF0000000000000000000000000000000000000000000000000000000000"
            )
            .unwrap_err(),
            AuditHashError::BadShape
        );
        assert_eq!(
            AuditHash::parse(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaagaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            )
            .unwrap_err(),
            AuditHashError::BadShape
        );
        assert_eq!(
            AuditHash::parse("sha256:aaaaaaaa").unwrap_err(),
            AuditHashError::BadShape
        );
        assert_eq!(hash('f').as_str().len(), "sha256:".len() + 64);
    }

    #[test]
    fn audit_chain_verify_detects_tampered_hashes() {
        let record = chain_record();
        let previous = hash('a');
        let payload = hash('b');
        let record_hash = hash('c');
        assert_eq!(
            record.verify(&previous, &payload, &record_hash),
            AuditChainCheckResult::verified()
        );

        assert_eq!(
            record.verify(&hash('d'), &payload, &record_hash),
            AuditChainCheckResult::failed(AuditChainCheckFailure::PreviousHashMismatch)
        );
        assert_eq!(
            record.verify(&previous, &hash('d'), &record_hash),
            AuditChainCheckResult::failed(AuditChainCheckFailure::PayloadHashMismatch)
        );
        let failed = record.verify(&previous, &payload, &hash('d'));
        assert_eq!(
            failed,
            AuditChainCheckResult::failed(AuditChainCheckFailure::RecordHashMismatch)
        );
        assert!(failed.is_fail_closed());
    }

    #[test]
    fn audit_chain_record_serialization_is_redacted_and_strict() {
        const SENTINEL: &str = "SharedAccessKey=secret-token /nix/store/leak argv env";
        let json = serde_json::to_string(&chain_record()).unwrap();
        assert!(!json.contains(SENTINEL));
        assert!(!json.contains("secret-token"));
        assert!(!json.contains("/nix/store"));
        assert!(json.contains("\"stream\":\"gateway\""));

        let with_unknown = json.replacen("\"stream\"", "\"unexpected\":true,\"stream\"", 1);
        assert!(serde_json::from_str::<AuditChainRecord>(&with_unknown).is_err());
    }

    #[test]
    fn audit_sink_health_reports_degraded_and_fail_closed_states() {
        let ok = AuditSinkHealth::ok(AuditStreamKind::Daemon, AuditRetentionFloorStatus::met());
        assert!(!ok.is_degraded());
        assert!(!ok.is_fail_closed());

        let degraded = AuditSinkHealth::degraded(
            AuditStreamKind::Gateway,
            AuditSinkHealthReason::Backpressure,
            AuditRetentionFloorStatus::met(),
        );
        assert!(degraded.is_degraded());
        assert!(!degraded.is_fail_closed());

        let chain_failed = AuditSinkHealth::degraded(
            AuditStreamKind::Gateway,
            AuditSinkHealthReason::ChainVerificationFailed,
            AuditRetentionFloorStatus::met(),
        );
        assert!(chain_failed.is_fail_closed());

        let unavailable = AuditSinkHealth::unavailable(
            AuditStreamKind::RemoteNode,
            AuditSinkHealthReason::SinkMissing,
            AuditRetentionFloorStatus::unknown(AuditRetentionFloorReason::EvidenceMissing),
        );
        assert!(unavailable.is_degraded());
        assert!(unavailable.is_fail_closed());
    }

    #[test]
    fn retention_floor_below_floor_is_fail_closed_and_redacted() {
        let below =
            AuditRetentionFloorStatus::below_floor(AuditRetentionFloorReason::WindowBelowFloor);
        assert!(below.is_fail_closed());

        let health = AuditSinkHealth::unavailable(
            AuditStreamKind::Gateway,
            AuditSinkHealthReason::WriteFailed,
            below,
        );
        let json = serde_json::to_string(&health).unwrap();
        assert!(json.contains("\"status\":\"unavailable\""));
        assert!(json.contains("\"status\":\"belowFloor\""));
        assert!(!json.contains("SharedAccessKey"));
        assert!(!json.contains("token"));
        assert!(!json.contains("/var/lib"));
    }
}
