//! The semantic `ConstellationFrame` (ADR 0032). This is the codec-neutral
//! frame API: protocol codecs map bytes to/from these types, and the
//! operation/routing layer depends only on this module — never on a wire
//! encoding (`prost`, protobuf-generated types, etc.).

use crate::audit::AuditEnvelope;
use crate::error::ConstellationError;
use crate::ids::{
    IdempotencyKey, NodeId, OperationId, PrincipalId, RealmId, StreamId, WorkloadId,
};
use crate::capability::Capability;
use crate::stream::StreamDescriptor;
use crate::trace_context::TraceContext;
use serde::{Deserialize, Serialize};

/// Negotiated wire/codec version and capability advertisement exchanged
/// at the start of a peer session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Handshake {
    /// Protocol version proposed/accepted (fail-closed on skew).
    pub protocol_version: u32,
    /// Codec id negotiated for this session.
    pub codec_id: String,
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
}

/// An operation request envelope. The operation-specific body is an
/// opaque, bounded payload that a higher layer encodes; the routing/authz
/// layer reasons over the typed envelope fields only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationRequest {
    /// Audit/correlation id.
    pub operation_id: OperationId,
    /// Caller-generated idempotency key (present for mutating ops).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub idempotency_key: Option<IdempotencyKey>,
    /// Target realm.
    pub realm: RealmId,
    /// Target node.
    pub node: NodeId,
    /// Target workload, when applicable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub workload: Option<WorkloadId>,
    /// Authenticated principal (never a relay credential).
    pub principal: PrincipalId,
    /// Operation kind.
    pub kind: OperationKind,
    /// Capability the operation requires.
    pub capability_required: Capability,
    /// Bounded trace context.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trace: Option<TraceContext>,
    /// Opaque, bounded operation-specific body (codec-defined).
    pub body: Vec<u8>,
}

/// An operation response envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationResponse {
    /// Correlates to the request.
    pub operation_id: OperationId,
    /// Opaque, bounded operation-specific body (codec-defined).
    pub body: Vec<u8>,
}

/// The semantic frame exchanged over a constellation peer session. The
/// codec layer maps bytes to/from this enum; the operation layer never
/// depends on the encoding.
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
    /// Open a named stream.
    StreamOpen(StreamDescriptor),
    /// A bounded chunk of stream data (opaque payload).
    StreamData {
        /// Stream the chunk belongs to.
        stream: StreamId,
        /// Opaque, bounded chunk bytes. Never logged/audited as content.
        data: Vec<u8>,
    },
    /// Close a named stream.
    StreamClose {
        /// Stream being closed.
        stream: StreamId,
    },
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
}
