//! The semantic `ConstellationFrame` (ADR 0032). This is the codec-neutral
//! frame API: protocol codecs map bytes to/from these types, and the
//! operation/routing layer depends only on this module — never on a wire
//! encoding (`prost`, protobuf-generated types, etc.).

use crate::audit::AuditEnvelope;
use crate::error::ConstellationError;
use crate::ids::{
    IdempotencyKey, NodeId, OperationId, PrincipalId, StreamId, WorkloadId,
};
use crate::capability::Capability;
use crate::payload::OpaquePayload;
use crate::realm::RealmPath;
use crate::stream::{StreamAuthz, StreamDescriptor};
use crate::token::ProtocolToken;
use crate::trace_context::TraceContext;
use serde::{Deserialize, Deserializer, Serialize};

/// A reserved, non-secret peer-binding context for the session handshake.
/// The bootstrap surface carries only the seam: later work populates the
/// peer identity + transcript metadata so mutual auth can be added
/// without a breaking wire change. It never carries
/// secret key material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PeerContext {
    /// Stable, non-secret auth-mechanism id negotiated for the session
    /// (e.g. `none` initially; `mtls`/`relay-sas-bound` later). Bounded.
    pub auth_mechanism: ProtocolToken,
    /// The peer's authenticated principal, once a mechanism binds one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub peer_principal: Option<PrincipalId>,
    /// The peer's node id, once bound.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub peer_node: Option<NodeId>,
}

/// Negotiated wire/codec version and capability advertisement exchanged
/// at the start of a peer session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Handshake {
    /// Protocol version proposed/accepted (fail-closed on skew).
    pub protocol_version: u32,
    /// Codec id negotiated for this session (bounded token).
    pub codec_id: ProtocolToken,
    /// Reserved peer-binding seam. Absent in the bootstrap protocol.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub peer: Option<PeerContext>,
}

/// The kind of an operation (ADR 0032 examples). Closed enum; unknown
/// kinds are rejected at decode (fail-closed).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum OperationKind {
    NodeRegister,
    NodeHeartbeat,
    NodeCapabilities,
    WorkloadList,
    WorkloadStart,
    WorkloadStop,
    GuestHealth,
    ExecStart,
    ExecAttach,
    ExecLogs,
    ExecCancel,
    FileCopyStart,
    PortForwardOpen,
    DisplaySessionOpen,
}

impl OperationKind {
    /// Whether this operation kind mutates state and therefore requires an
    /// idempotency key + bounded dedup for at-least-once delivery.
    pub fn is_mutating(self) -> bool {
        matches!(
            self,
            OperationKind::WorkloadStart
                | OperationKind::WorkloadStop
                | OperationKind::ExecStart
                | OperationKind::ExecCancel
                | OperationKind::FileCopyStart
                | OperationKind::PortForwardOpen
                | OperationKind::DisplaySessionOpen
                | OperationKind::NodeRegister
        )
    }

    /// The capability the router/authz layer requires for this kind,
    /// derived from trusted code. `None` means the kind is authorized by
    /// node enrollment / session identity rather than a workload
    /// capability (node-control + read-only health). Callers MUST derive
    /// the required capability from the kind — never from a caller-supplied
    /// field.
    pub fn required_capability(self) -> Option<Capability> {
        match self {
            OperationKind::NodeRegister
            | OperationKind::NodeHeartbeat
            | OperationKind::NodeCapabilities
            | OperationKind::GuestHealth => None,
            OperationKind::WorkloadList
            | OperationKind::WorkloadStart
            | OperationKind::WorkloadStop => Some(Capability::Lifecycle),
            OperationKind::ExecStart | OperationKind::ExecAttach | OperationKind::ExecCancel => {
                Some(Capability::Exec)
            }
            OperationKind::ExecLogs => Some(Capability::Logs),
            OperationKind::FileCopyStart => Some(Capability::FileCopy),
            OperationKind::PortForwardOpen => Some(Capability::PortForward),
            OperationKind::DisplaySessionOpen => Some(Capability::WindowForwarding),
        }
    }

    /// The audit authorization scope for this kind. Workload ops map to a
    /// capability; node-control/enrollment/health ops have their own scope
    /// so they can be audited truthfully without a synthetic capability.
    pub fn authorization_scope(self) -> crate::audit::AuthorizationScope {
        use crate::audit::AuthorizationScope;
        match self {
            OperationKind::NodeRegister => AuthorizationScope::Enrollment,
            OperationKind::NodeHeartbeat | OperationKind::NodeCapabilities => {
                AuthorizationScope::NodeControl
            }
            OperationKind::GuestHealth => AuthorizationScope::Health,
            _ => match self.required_capability() {
                Some(cap) => AuthorizationScope::capability(cap),
                None => AuthorizationScope::NodeControl,
            },
        }
    }
}

/// An operation request envelope. The operation-specific body is an
/// opaque, bounded payload that a higher layer encodes; the routing/authz
/// layer reasons over the typed envelope fields only.
///
/// The required capability is NOT a wire field — it is derived from
/// [`OperationKind::required_capability`] in trusted code so a peer cannot
/// downgrade it. The authenticated session principal MUST match
/// [`OperationRequest::principal`]; the router rejects a mismatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationRequest {
    /// Audit/correlation id (per attempt).
    pub operation_id: OperationId,
    /// Caller-generated idempotency key (required for mutating ops).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub idempotency_key: Option<IdempotencyKey>,
    /// Target realm path (most-specific-first; supports nested realms).
    pub realm: RealmPath,
    /// Target node.
    pub node: NodeId,
    /// Target workload, when applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub workload: Option<WorkloadId>,
    /// Authenticated principal (never a relay credential).
    pub principal: PrincipalId,
    /// Operation kind.
    pub kind: OperationKind,
    /// Bounded trace context.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trace: Option<TraceContext>,
    /// Opaque, bounded operation-specific body (codec-defined).
    pub body: OpaquePayload,
}

// Fail-closed decode: a mutating operation MUST carry an idempotency key,
// so an at-least-once retry can be deduped before any side effect. A
// mutating request that omits the key is rejected at the boundary.
impl<'de> Deserialize<'de> for OperationRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            operation_id: OperationId,
            #[serde(default)]
            idempotency_key: Option<IdempotencyKey>,
            realm: RealmPath,
            node: NodeId,
            #[serde(default)]
            workload: Option<WorkloadId>,
            principal: PrincipalId,
            kind: OperationKind,
            #[serde(default)]
            trace: Option<TraceContext>,
            body: OpaquePayload,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.kind.is_mutating() && raw.idempotency_key.is_none() {
            return Err(serde::de::Error::custom(
                "mutating operation requires an idempotency_key",
            ));
        }
        Ok(OperationRequest {
            operation_id: raw.operation_id,
            idempotency_key: raw.idempotency_key,
            realm: raw.realm,
            node: raw.node,
            workload: raw.workload,
            principal: raw.principal,
            kind: raw.kind,
            trace: raw.trace,
            body: raw.body,
        })
    }
}

impl OperationRequest {
    /// The capability this request requires, derived from its kind (never
    /// from a caller-supplied field).
    pub fn required_capability(&self) -> Option<Capability> {
        self.kind.required_capability()
    }

    /// Whether the dedup owner MUST reject this request when it carries no
    /// idempotency key (true for mutating kinds).
    pub fn requires_idempotency_key(&self) -> bool {
        self.kind.is_mutating()
    }

    /// The canonical, deterministic byte input the dedup owner hashes to
    /// detect a *same-key, same-request* replay vs a *same-key,
    /// different-request* conflict. It includes exactly the
    /// request-identifying fields — `kind`, `realm`, `node`, `workload`,
    /// `principal`, and `body` — and deliberately EXCLUDES `operation_id`
    /// (per-attempt), `idempotency_key`, and `trace`.
    /// The dedup owner (the gateway/router, never the provider) hashes
    /// this with a collision-resistant digest.
    pub fn dedup_fingerprint_input(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut push = |label: &str, value: &[u8]| {
            out.extend_from_slice(label.as_bytes());
            out.push(b'=');
            out.extend_from_slice(&(value.len() as u64).to_le_bytes());
            out.extend_from_slice(value);
            out.push(b'\n');
        };
        push("kind", format!("{:?}", self.kind).as_bytes());
        push("realm", self.realm.target_form().as_bytes());
        push("node", self.node.as_str().as_bytes());
        push(
            "workload",
            self.workload.as_ref().map(|w| w.as_str()).unwrap_or("").as_bytes(),
        );
        push("principal", self.principal.as_str().as_bytes());
        push("body", self.body.as_bytes());
        out
    }
}

/// An operation response envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationResponse {
    /// Correlates to the request.
    pub operation_id: OperationId,
    /// Opaque, bounded operation-specific body (codec-defined).
    pub body: OpaquePayload,
}

/// A stream-open frame: the descriptor, the operation that authorized the
/// open (so the open is bound to a single authorizing operation), and the
/// authorization context the mux MUST evaluate. Carrying [`StreamAuthz`]
/// here means a stream open is never authorized without
/// principal/realm/capability context, and every open can be audited and
/// bound to its operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamOpen {
    /// The stream id + kind.
    pub descriptor: StreamDescriptor,
    /// The authorizing operation id (binds this open to one operation).
    pub operation_id: OperationId,
    /// The authorization context (principal, realm, derived capability).
    pub authz: StreamAuthz,
}

impl StreamOpen {
    /// True iff the carried authz capability matches the descriptor kind.
    /// The mux MUST reject the open when this is false (fail-closed).
    pub fn is_consistent(&self) -> bool {
        self.authz.matches_kind(self.descriptor.kind)
    }
}

// Fail-closed decode: an inconsistent kind/capability pairing is rejected
// at the boundary, so a mux cannot be tricked into authorizing a
// downgraded capability even if it forgets to call `is_consistent`.
impl<'de> Deserialize<'de> for StreamOpen {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            descriptor: StreamDescriptor,
            operation_id: OperationId,
            authz: StreamAuthz,
        }
        let raw = Raw::deserialize(deserializer)?;
        let open = StreamOpen {
            descriptor: raw.descriptor,
            operation_id: raw.operation_id,
            authz: raw.authz,
        };
        if open.is_consistent() {
            Ok(open)
        } else {
            Err(serde::de::Error::custom(
                "stream-open authz capability does not match the descriptor kind",
            ))
        }
    }
}

/// A bounded chunk of stream data (opaque payload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamData {
    /// Stream the chunk belongs to.
    pub stream: StreamId,
    /// Opaque, bounded chunk bytes. Never logged/audited as content.
    pub data: OpaquePayload,
}

/// Close a named stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamClose {
    /// Stream being closed.
    pub stream: StreamId,
}

/// The semantic frame exchanged over a constellation peer session. The
/// codec layer maps bytes to/from this enum; the operation layer never
/// depends on the encoding. Every variant wraps a `deny_unknown_fields`
/// struct so a peer cannot smuggle extra fields past the decoder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "frame", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ConstellationFrame {
    /// Session handshake (version + codec negotiation).
    Handshake(Handshake),
    /// An operation request.
    OperationRequest(OperationRequest),
    /// An operation response.
    OperationResponse(OperationResponse),
    /// Open a named stream (descriptor + authorization context).
    StreamOpen(StreamOpen),
    /// A bounded chunk of stream data (opaque payload).
    StreamData(StreamData),
    /// Close a named stream.
    StreamClose(StreamClose),
    /// A typed error frame.
    TypedError(ConstellationError),
    /// A redacted admission/audit frame.
    AdmissionAudit(AuditEnvelope),
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trips_through_semantic_api() {
        let frame = ConstellationFrame::TypedError(ConstellationError::capability_denied(
            Capability::WindowForwarding,
        ));
        let json = serde_json::to_string(&frame).unwrap();
        let back: ConstellationFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(frame, back);
    }

    #[test]
    fn mutating_kinds_are_flagged() {
        assert!(OperationKind::WorkloadStart.is_mutating());
        assert!(OperationKind::ExecStart.is_mutating());
        assert!(!OperationKind::WorkloadList.is_mutating());
        assert!(!OperationKind::GuestHealth.is_mutating());
    }

    #[test]
    fn authorization_scope_maps_node_and_health_ops() {
        use crate::audit::AuthorizationScope;
        assert_eq!(
            OperationKind::NodeRegister.authorization_scope(),
            AuthorizationScope::Enrollment
        );
        assert_eq!(
            OperationKind::NodeHeartbeat.authorization_scope(),
            AuthorizationScope::NodeControl
        );
        assert_eq!(
            OperationKind::NodeCapabilities.authorization_scope(),
            AuthorizationScope::NodeControl
        );
        assert_eq!(
            OperationKind::GuestHealth.authorization_scope(),
            AuthorizationScope::Health
        );
        assert_eq!(
            OperationKind::ExecStart.authorization_scope(),
            AuthorizationScope::capability(Capability::Exec)
        );
    }

    #[test]
    fn stream_open_decode_rejects_inconsistent_authz() {
        // A Display descriptor paired with a downgraded Clipboard authz must
        // fail to decode, both as a StreamOpen and inside a frame.
        let forged = "{\"descriptor\":{\"id\":\"s1\",\"kind\":\"display\"},\
                      \"operation_id\":\"op1\",\
                      \"authz\":{\"principal\":\"p1\",\"realm\":[\"local\"],\"capability\":\"clipboard\"}}";
        assert!(serde_json::from_str::<StreamOpen>(forged).is_err());
        let framed = format!("{{\"frame\":\"stream-open\",{}}}", &forged[1..]);
        assert!(serde_json::from_str::<ConstellationFrame>(&framed).is_err());
        // The consistent pairing decodes.
        let ok = "{\"descriptor\":{\"id\":\"s1\",\"kind\":\"display\"},\
                   \"operation_id\":\"op1\",\
                   \"authz\":{\"principal\":\"p1\",\"realm\":[\"local\"],\"capability\":\"window-forwarding\"}}";
        assert!(serde_json::from_str::<StreamOpen>(ok).is_ok());
    }

    #[test]
    fn operation_request_decode_requires_idempotency_key_for_mutating() {
        // WorkloadStart is mutating: omitting the key fails closed.
        let no_key = "{\"operation_id\":\"op1\",\"realm\":[\"work\"],\"node\":\"n1\",\
                      \"principal\":\"p1\",\"kind\":\"workload-start\",\"body\":[]}";
        assert!(serde_json::from_str::<OperationRequest>(no_key).is_err());
        // With a key it decodes.
        let with_key = "{\"operation_id\":\"op1\",\"idempotency_key\":\"k1\",\
                        \"realm\":[\"work\"],\"node\":\"n1\",\"principal\":\"p1\",\
                        \"kind\":\"workload-start\",\"body\":[]}";
        assert!(serde_json::from_str::<OperationRequest>(with_key).is_ok());
        // A non-mutating op needs no key.
        let read = "{\"operation_id\":\"op1\",\"realm\":[\"work\"],\"node\":\"n1\",\
                    \"principal\":\"p1\",\"kind\":\"workload-list\",\"body\":[]}";
        assert!(serde_json::from_str::<OperationRequest>(read).is_ok());
    }

    #[test]
    fn handshake_codec_id_is_bounded_at_decode() {
        let ok = "{\"protocol_version\":1,\"codec_id\":\"protobuf.v1\"}";
        assert!(serde_json::from_str::<Handshake>(ok).is_ok());
        let overlong = format!(
            "{{\"protocol_version\":1,\"codec_id\":\"{}\"}}",
            "x".repeat(200)
        );
        assert!(serde_json::from_str::<Handshake>(&overlong).is_err());
    }

    #[test]
    fn stream_frames_reject_unknown_fields() {
        // Valid stream-data / stream-close frames decode.
        let data = "{\"frame\":\"stream-data\",\"stream\":\"s1\",\"data\":[1,2,3]}";
        assert!(serde_json::from_str::<ConstellationFrame>(data).is_ok());
        let close = "{\"frame\":\"stream-close\",\"stream\":\"s1\"}";
        assert!(serde_json::from_str::<ConstellationFrame>(close).is_ok());
        // Extra peer-supplied fields are rejected (deny_unknown_fields).
        let data_extra =
            "{\"frame\":\"stream-data\",\"stream\":\"s1\",\"data\":[1],\"evil\":true}";
        assert!(serde_json::from_str::<ConstellationFrame>(data_extra).is_err());
        let close_extra = "{\"frame\":\"stream-close\",\"stream\":\"s1\",\"evil\":true}";
        assert!(serde_json::from_str::<ConstellationFrame>(close_extra).is_err());
    }
}
