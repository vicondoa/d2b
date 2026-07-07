//! Protobuf `ProtocolCodec` implementation for ADR 0032 constellation frames.

use d2b_contracts::MAX_FRAME_SIZE;
use d2b_realm_core::{
    AdmissionAuditRecord, AuthorizationScope, AuthzDecision, Capability, CapabilityNegotiation,
    CapabilitySet, ConstellationError, ConstellationFrame, CorrelationId, ErrorKind, Handshake,
    HandshakeAccepted, HandshakeRejected, HandshakeRejectedReason, IdempotencyKey, NodeId,
    OpaquePayload, OperationId, OperationKind, OperationRequest, OperationResponse, PeerContext,
    PrincipalId, ProtocolToken, RealmId, RealmPath, StreamAuthz, StreamChannel, StreamClose,
    StreamCloseReason, StreamCursor, StreamData, StreamDescriptor, StreamFlow, StreamId,
    StreamKind, StreamOpen, StreamResume, TraceContext, WorkloadId,
};
use d2b_realm_provider::ProtocolCodec;
use prost::Message;

/// Stable codec id negotiated by ADR 0032 protobuf peers.
pub const CODEC_ID: &str = "protobuf.v1";

/// Deterministic fingerprint for the hand-authored prost schema in this crate.
pub const SCHEMA_FINGERPRINT: &str = "pb.v1:f12:h6:op19:resp3:sk13:err20:audit12:cap21";

/// A prost-backed constellation frame codec.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProtobufCodec;

impl ProtobufCodec {
    /// Construct a protobuf constellation codec.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ProtocolCodec for ProtobufCodec {
    fn codec_id(&self) -> &str {
        CODEC_ID
    }

    fn encode_frame(&self, frame: &ConstellationFrame) -> Result<Vec<u8>, ConstellationError> {
        Ok(encode_proto_frame(frame)?.encode_to_vec())
    }

    fn decode_frame(&self, bytes: &[u8]) -> Result<ConstellationFrame, ConstellationError> {
        if bytes.len() > MAX_FRAME_SIZE {
            return Err(frame_too_large(format!(
                "protobuf frame exceeds maximum size: {len} > {MAX_FRAME_SIZE}",
                len = bytes.len()
            )));
        }
        let frame = ProtoFrame::decode(bytes)
            .map_err(|err| malformed(format!("protobuf frame decode failed: {err}")))?;
        decode_proto_frame(frame)
    }

    fn schema_fingerprint(&self) -> String {
        SCHEMA_FINGERPRINT.to_owned()
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoFrame {
    #[prost(
        oneof = "proto_frame::Body",
        tags = "1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12"
    )]
    body: Option<proto_frame::Body>,
}

mod proto_frame {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub(super) enum Body {
        #[prost(message, tag = "1")]
        Handshake(super::ProtoHandshake),
        #[prost(message, tag = "2")]
        OperationRequest(super::ProtoOperationRequest),
        #[prost(message, tag = "3")]
        OperationResponse(super::ProtoOperationResponse),
        #[prost(message, tag = "4")]
        StreamOpen(super::ProtoStreamOpen),
        #[prost(message, tag = "5")]
        StreamData(super::ProtoStreamData),
        #[prost(message, tag = "6")]
        StreamClose(super::ProtoStreamClose),
        #[prost(message, tag = "7")]
        TypedError(super::ProtoError),
        #[prost(message, tag = "8")]
        AdmissionAudit(super::ProtoAuditEnvelope),
        #[prost(message, tag = "9")]
        StreamFlow(super::ProtoStreamFlow),
        #[prost(message, tag = "10")]
        HandshakeAccepted(super::ProtoHandshakeAccepted),
        #[prost(message, tag = "11")]
        HandshakeRejected(super::ProtoHandshakeRejected),
        #[prost(message, tag = "12")]
        StreamResume(super::ProtoStreamResume),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoHandshake {
    #[prost(uint32, optional, tag = "1")]
    protocol_version: Option<u32>,
    #[prost(string, tag = "2")]
    codec_id: String,
    #[prost(message, optional, tag = "3")]
    peer: Option<ProtoPeerContext>,
    #[prost(string, tag = "4")]
    schema_fingerprint: String,
    #[prost(int32, repeated, tag = "5")]
    capabilities: Vec<i32>,
    #[prost(uint32, tag = "6")]
    capability_schema_version: u32,
    #[prost(string, tag = "7")]
    capability_fingerprint: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoHandshakeAccepted {
    #[prost(message, optional, tag = "1")]
    selected: Option<ProtoHandshake>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoHandshakeRejected {
    #[prost(int32, tag = "1")]
    reason: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoPeerContext {
    #[prost(string, tag = "1")]
    auth_mechanism: String,
    #[prost(string, optional, tag = "2")]
    peer_principal: Option<String>,
    #[prost(string, optional, tag = "3")]
    peer_node: Option<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoOperationRequest {
    #[prost(string, tag = "1")]
    operation_id: String,
    #[prost(string, optional, tag = "2")]
    idempotency_key: Option<String>,
    #[prost(string, repeated, tag = "3")]
    realm: Vec<String>,
    #[prost(string, tag = "4")]
    node: String,
    #[prost(string, optional, tag = "5")]
    workload: Option<String>,
    #[prost(string, tag = "6")]
    principal: String,
    #[prost(int32, tag = "7")]
    kind: i32,
    #[prost(message, optional, tag = "8")]
    trace: Option<ProtoTraceContext>,
    #[prost(message, optional, tag = "9")]
    body: Option<ProtoPayload>,
    #[prost(string, tag = "10")]
    correlation_id: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoOperationResponse {
    #[prost(string, tag = "1")]
    operation_id: String,
    #[prost(message, optional, tag = "2")]
    body: Option<ProtoPayload>,
    #[prost(string, tag = "3")]
    correlation_id: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoStreamOpen {
    #[prost(message, optional, tag = "1")]
    descriptor: Option<ProtoStreamDescriptor>,
    #[prost(string, tag = "2")]
    operation_id: String,
    #[prost(message, optional, tag = "3")]
    authz: Option<ProtoStreamAuthz>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoStreamDescriptor {
    #[prost(string, tag = "1")]
    id: String,
    #[prost(int32, tag = "2")]
    kind: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoStreamAuthz {
    #[prost(string, tag = "1")]
    principal: String,
    #[prost(string, repeated, tag = "2")]
    realm: Vec<String>,
    #[prost(int32, tag = "3")]
    capability: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoStreamData {
    #[prost(string, tag = "1")]
    stream: String,
    #[prost(message, optional, tag = "2")]
    data: Option<ProtoPayload>,
    #[prost(uint64, tag = "3")]
    sequence: u64,
    #[prost(int32, tag = "4")]
    channel: i32,
    #[prost(string, optional, tag = "5")]
    cursor: Option<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoStreamFlow {
    #[prost(string, tag = "1")]
    stream: String,
    #[prost(uint32, tag = "2")]
    credits: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoStreamClose {
    #[prost(string, tag = "1")]
    stream: String,
    #[prost(int32, tag = "2")]
    reason: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoStreamResume {
    #[prost(string, tag = "1")]
    stream: String,
    #[prost(string, tag = "2")]
    cursor: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoError {
    #[prost(int32, tag = "1")]
    kind: i32,
    #[prost(int32, optional, tag = "2")]
    capability: Option<i32>,
    #[prost(string, optional, tag = "3")]
    message: Option<String>,
    #[prost(string, optional, tag = "4")]
    negotiated_capability_fingerprint: Option<String>,
    #[prost(string, optional, tag = "5")]
    correlation_id: Option<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoAuditEnvelope {
    #[prost(string, tag = "1")]
    operation_id: String,
    #[prost(string, repeated, tag = "2")]
    realm: Vec<String>,
    #[prost(string, optional, tag = "3")]
    principal: Option<String>,
    #[prost(string, tag = "4")]
    node: String,
    #[prost(string, optional, tag = "5")]
    workload: Option<String>,
    #[prost(string, optional, tag = "6")]
    stream: Option<String>,
    #[prost(string, optional, tag = "7")]
    execution: Option<String>,
    #[prost(message, optional, tag = "8")]
    scope: Option<ProtoAuthorizationScope>,
    #[prost(int32, tag = "9")]
    decision: i32,
    #[prost(message, optional, tag = "10")]
    trace: Option<ProtoTraceContext>,
    #[prost(int32, optional, tag = "11")]
    reason: Option<i32>,
    #[prost(string, tag = "12")]
    correlation_id: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoAuthorizationScope {
    #[prost(oneof = "proto_authorization_scope::Scope", tags = "1, 2, 3, 4")]
    scope: Option<proto_authorization_scope::Scope>,
}

mod proto_authorization_scope {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub(super) enum Scope {
        #[prost(int32, tag = "1")]
        Capability(i32),
        #[prost(message, tag = "2")]
        NodeControl(super::ProtoUnit),
        #[prost(message, tag = "3")]
        Enrollment(super::ProtoUnit),
        #[prost(message, tag = "4")]
        Health(super::ProtoUnit),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoTraceContext {
    #[prost(string, tag = "1")]
    trace_id: String,
    #[prost(string, tag = "2")]
    span_id: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoPayload {
    #[prost(bytes = "vec", tag = "1")]
    bytes: Vec<u8>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ProtoUnit {}

fn encode_proto_frame(frame: &ConstellationFrame) -> Result<ProtoFrame, ConstellationError> {
    let body = match frame {
        ConstellationFrame::Handshake(frame) => {
            proto_frame::Body::Handshake(encode_handshake(frame)?)
        }
        ConstellationFrame::HandshakeAccepted(frame) => {
            proto_frame::Body::HandshakeAccepted(encode_handshake_accepted(frame)?)
        }
        ConstellationFrame::HandshakeRejected(frame) => {
            proto_frame::Body::HandshakeRejected(encode_handshake_rejected(frame)?)
        }
        ConstellationFrame::OperationRequest(frame) => {
            proto_frame::Body::OperationRequest(encode_operation_request(frame)?)
        }
        ConstellationFrame::OperationResponse(frame) => {
            proto_frame::Body::OperationResponse(encode_operation_response(frame))
        }
        ConstellationFrame::StreamOpen(frame) => {
            proto_frame::Body::StreamOpen(encode_stream_open(frame)?)
        }
        ConstellationFrame::StreamData(frame) => {
            proto_frame::Body::StreamData(encode_stream_data(frame))
        }
        ConstellationFrame::StreamFlow(frame) => {
            proto_frame::Body::StreamFlow(encode_stream_flow(frame))
        }
        ConstellationFrame::StreamClose(frame) => {
            proto_frame::Body::StreamClose(encode_stream_close(frame))
        }
        ConstellationFrame::StreamResume(frame) => {
            proto_frame::Body::StreamResume(encode_stream_resume(frame))
        }
        ConstellationFrame::TypedError(frame) => {
            proto_frame::Body::TypedError(encode_error(frame)?)
        }
        ConstellationFrame::AdmissionAudit(frame) => {
            proto_frame::Body::AdmissionAudit(encode_admission_audit(frame)?)
        }
    };
    Ok(ProtoFrame { body: Some(body) })
}

fn decode_proto_frame(frame: ProtoFrame) -> Result<ConstellationFrame, ConstellationError> {
    match frame
        .body
        .ok_or_else(|| malformed("protobuf frame body is missing"))?
    {
        proto_frame::Body::Handshake(frame) => {
            decode_handshake(frame).map(ConstellationFrame::Handshake)
        }
        proto_frame::Body::HandshakeAccepted(frame) => {
            decode_handshake_accepted(frame).map(ConstellationFrame::HandshakeAccepted)
        }
        proto_frame::Body::HandshakeRejected(frame) => {
            decode_handshake_rejected(frame).map(ConstellationFrame::HandshakeRejected)
        }
        proto_frame::Body::OperationRequest(frame) => {
            decode_operation_request(frame).map(ConstellationFrame::OperationRequest)
        }
        proto_frame::Body::OperationResponse(frame) => {
            decode_operation_response(frame).map(ConstellationFrame::OperationResponse)
        }
        proto_frame::Body::StreamOpen(frame) => {
            decode_stream_open(frame).map(ConstellationFrame::StreamOpen)
        }
        proto_frame::Body::StreamData(frame) => {
            decode_stream_data(frame).map(ConstellationFrame::StreamData)
        }
        proto_frame::Body::StreamFlow(frame) => {
            decode_stream_flow(frame).map(ConstellationFrame::StreamFlow)
        }
        proto_frame::Body::StreamClose(frame) => {
            decode_stream_close(frame).map(ConstellationFrame::StreamClose)
        }
        proto_frame::Body::StreamResume(frame) => {
            decode_stream_resume(frame).map(ConstellationFrame::StreamResume)
        }
        proto_frame::Body::TypedError(frame) => {
            decode_error(frame).map(ConstellationFrame::TypedError)
        }
        proto_frame::Body::AdmissionAudit(frame) => {
            decode_admission_audit(frame).map(ConstellationFrame::AdmissionAudit)
        }
    }
}

fn encode_handshake(frame: &Handshake) -> Result<ProtoHandshake, ConstellationError> {
    let mut capabilities = frame
        .capabilities
        .capabilities
        .iter()
        .map(encode_capability)
        .collect::<Result<Vec<_>, _>>()?;
    for token in frame.capabilities.capabilities.unknown_iter() {
        if let Some(raw) = token
            .as_str()
            .strip_prefix("unknown-protobuf-capability-")
            .and_then(|raw| raw.parse::<i32>().ok())
        {
            capabilities.push(raw);
        }
    }
    capabilities.sort_unstable();
    capabilities.dedup();
    Ok(ProtoHandshake {
        protocol_version: Some(frame.protocol_version),
        codec_id: frame.codec_id.as_str().to_owned(),
        schema_fingerprint: frame.schema_fingerprint.as_str().to_owned(),
        capabilities,
        capability_schema_version: frame.capabilities.schema_version,
        capability_fingerprint: frame.capabilities.fingerprint.clone(),
        peer: frame.peer.as_ref().map(encode_peer_context),
    })
}

fn decode_handshake(frame: ProtoHandshake) -> Result<Handshake, ConstellationError> {
    Ok(Handshake {
        protocol_version: frame
            .protocol_version
            .ok_or_else(|| malformed("handshake protocol_version is missing"))?,
        codec_id: parse_protocol_token(frame.codec_id, "handshake codec_id")?,
        schema_fingerprint: parse_protocol_token(
            frame.schema_fingerprint,
            "handshake schema_fingerprint",
        )?,
        capabilities: decode_capability_negotiation(
            frame.capabilities,
            frame.capability_schema_version,
            frame.capability_fingerprint,
        )?,
        peer: frame.peer.map(decode_peer_context).transpose()?,
    })
}

fn decode_capability_negotiation(
    capabilities: Vec<i32>,
    schema_version: u32,
    fingerprint: String,
) -> Result<CapabilityNegotiation, ConstellationError> {
    let wire_fingerprint = parse_protocol_token(fingerprint, "handshake capability_fingerprint")?;
    let caps =
        capabilities
            .into_iter()
            .map(|raw| match decode_capability(raw) {
                Some(capability) => ProtocolToken::parse(capability.code()).map_err(|_| {
                    malformed("failed to encode known protobuf capability as protocol token")
                }),
                None => ProtocolToken::parse(format!("unknown-protobuf-capability-{raw}")).map_err(
                    |err| malformed(format!("unknown protobuf capability token invalid: {err}")),
                ),
            })
            .collect::<Result<Vec<_>, _>>()?;
    let capabilities = CapabilitySet::from_tokens(caps);
    let expected = capabilities.stable_fingerprint();
    let has_unknown = capabilities.unknown_iter().next().is_some();
    if schema_version != d2b_realm_core::capability::CAPABILITY_NEGOTIATION_SCHEMA_VERSION {
        return Err(malformed(
            "handshake capability negotiation schema version mismatch",
        ));
    }
    if !has_unknown && wire_fingerprint.as_str() != expected {
        return Err(malformed(
            "handshake capability negotiation fingerprint mismatch",
        ));
    }
    Ok(CapabilityNegotiation {
        schema_version,
        capabilities,
        fingerprint: expected,
    })
}

fn encode_handshake_accepted(
    frame: &HandshakeAccepted,
) -> Result<ProtoHandshakeAccepted, ConstellationError> {
    Ok(ProtoHandshakeAccepted {
        selected: Some(encode_handshake(&frame.selected)?),
    })
}

fn decode_handshake_accepted(
    frame: ProtoHandshakeAccepted,
) -> Result<HandshakeAccepted, ConstellationError> {
    Ok(HandshakeAccepted {
        selected: decode_handshake(
            frame
                .selected
                .ok_or_else(|| malformed("handshake_accepted selected is missing"))?,
        )?,
    })
}

fn encode_handshake_rejected(
    frame: &HandshakeRejected,
) -> Result<ProtoHandshakeRejected, ConstellationError> {
    Ok(ProtoHandshakeRejected {
        reason: encode_handshake_rejection(frame.reason),
    })
}

fn decode_handshake_rejected(
    frame: ProtoHandshakeRejected,
) -> Result<HandshakeRejected, ConstellationError> {
    Ok(HandshakeRejected {
        reason: decode_handshake_rejection(frame.reason)?,
    })
}

fn encode_peer_context(peer: &PeerContext) -> ProtoPeerContext {
    ProtoPeerContext {
        auth_mechanism: peer.auth_mechanism.as_str().to_owned(),
        peer_principal: peer
            .peer_principal
            .as_ref()
            .map(|principal| principal.as_str().to_owned()),
        peer_node: peer.peer_node.as_ref().map(|node| node.as_str().to_owned()),
    }
}

fn decode_peer_context(peer: ProtoPeerContext) -> Result<PeerContext, ConstellationError> {
    Ok(PeerContext {
        auth_mechanism: parse_protocol_token(peer.auth_mechanism, "peer auth_mechanism")?,
        peer_principal: peer
            .peer_principal
            .map(|principal| parse_principal_id(principal, "peer principal"))
            .transpose()?,
        peer_node: peer
            .peer_node
            .map(|node| parse_node_id(node, "peer node"))
            .transpose()?,
    })
}

fn encode_operation_request(
    frame: &OperationRequest,
) -> Result<ProtoOperationRequest, ConstellationError> {
    Ok(ProtoOperationRequest {
        operation_id: frame.operation_id.as_str().to_owned(),
        correlation_id: frame.correlation_id.as_str().to_owned(),
        idempotency_key: frame
            .idempotency_key
            .as_ref()
            .map(|key| key.as_str().to_owned()),
        realm: encode_realm(&frame.realm),
        node: frame.node.as_str().to_owned(),
        workload: frame
            .workload
            .as_ref()
            .map(|workload| workload.as_str().to_owned()),
        principal: frame.principal.as_str().to_owned(),
        kind: encode_operation_kind(frame.kind)?,
        trace: frame.trace.as_ref().map(encode_trace_context),
        body: Some(encode_payload(&frame.body)),
    })
}

fn decode_operation_request(
    frame: ProtoOperationRequest,
) -> Result<OperationRequest, ConstellationError> {
    let kind = decode_operation_kind(frame.kind)?;
    let request = OperationRequest {
        operation_id: parse_operation_id(frame.operation_id, "operation_request operation_id")?,
        correlation_id: parse_correlation_id(
            frame.correlation_id,
            "operation_request correlation_id",
        )?,
        idempotency_key: frame
            .idempotency_key
            .map(|key| parse_idempotency_key(key, "operation_request idempotency_key"))
            .transpose()?,
        realm: decode_realm(frame.realm, "operation_request realm")?,
        node: parse_node_id(frame.node, "operation_request node")?,
        workload: frame
            .workload
            .map(|workload| parse_workload_id(workload, "operation_request workload"))
            .transpose()?,
        principal: parse_principal_id(frame.principal, "operation_request principal")?,
        kind,
        trace: frame.trace.map(decode_trace_context).transpose()?,
        body: decode_payload(frame.body, "operation_request body")?,
    };
    if request.requires_idempotency_key() && request.idempotency_key.is_none() {
        return Err(malformed(
            "mutating operation_request requires an idempotency_key",
        ));
    }
    Ok(request)
}

fn encode_operation_response(frame: &OperationResponse) -> ProtoOperationResponse {
    ProtoOperationResponse {
        operation_id: frame.operation_id.as_str().to_owned(),
        body: Some(encode_payload(&frame.body)),
        correlation_id: frame.correlation_id.as_str().to_owned(),
    }
}

fn decode_operation_response(
    frame: ProtoOperationResponse,
) -> Result<OperationResponse, ConstellationError> {
    Ok(OperationResponse {
        operation_id: parse_operation_id(frame.operation_id, "operation_response operation_id")?,
        correlation_id: parse_correlation_id(
            frame.correlation_id,
            "operation_response correlation_id",
        )?,
        body: decode_payload(frame.body, "operation_response body")?,
    })
}

fn encode_stream_open(frame: &StreamOpen) -> Result<ProtoStreamOpen, ConstellationError> {
    Ok(ProtoStreamOpen {
        descriptor: Some(encode_stream_descriptor(&frame.descriptor)?),
        operation_id: frame.operation_id.as_str().to_owned(),
        authz: Some(encode_stream_authz(&frame.authz)?),
    })
}

fn decode_stream_open(frame: ProtoStreamOpen) -> Result<StreamOpen, ConstellationError> {
    let open = StreamOpen {
        descriptor: decode_stream_descriptor(
            frame
                .descriptor
                .ok_or_else(|| malformed("stream_open descriptor is missing"))?,
        )?,
        operation_id: parse_operation_id(frame.operation_id, "stream_open operation_id")?,
        authz: decode_stream_authz(
            frame
                .authz
                .ok_or_else(|| malformed("stream_open authz is missing"))?,
        )?,
    };
    if open.is_consistent() {
        Ok(open)
    } else {
        Err(malformed(
            "stream_open authz capability does not match descriptor kind",
        ))
    }
}

fn encode_stream_descriptor(
    descriptor: &StreamDescriptor,
) -> Result<ProtoStreamDescriptor, ConstellationError> {
    Ok(ProtoStreamDescriptor {
        id: descriptor.id.as_str().to_owned(),
        kind: encode_stream_kind(descriptor.kind)?,
    })
}

fn decode_stream_descriptor(
    descriptor: ProtoStreamDescriptor,
) -> Result<StreamDescriptor, ConstellationError> {
    Ok(StreamDescriptor {
        id: parse_stream_id(descriptor.id, "stream_descriptor id")?,
        kind: decode_stream_kind(descriptor.kind)?,
    })
}

fn encode_stream_authz(authz: &StreamAuthz) -> Result<ProtoStreamAuthz, ConstellationError> {
    Ok(ProtoStreamAuthz {
        principal: authz.principal.as_str().to_owned(),
        realm: encode_realm(&authz.realm),
        capability: encode_capability(authz.capability)?,
    })
}

fn decode_stream_authz(authz: ProtoStreamAuthz) -> Result<StreamAuthz, ConstellationError> {
    Ok(StreamAuthz {
        principal: parse_principal_id(authz.principal, "stream_authz principal")?,
        realm: decode_realm(authz.realm, "stream_authz realm")?,
        capability: decode_capability_strict(authz.capability)?,
    })
}

fn encode_stream_data(frame: &StreamData) -> ProtoStreamData {
    ProtoStreamData {
        stream: frame.stream.as_str().to_owned(),
        data: Some(encode_payload(&frame.data)),
        sequence: frame.sequence,
        channel: encode_stream_channel(frame.channel),
        cursor: frame.cursor.as_ref().map(|c| c.as_str().to_owned()),
    }
}

fn decode_stream_data(frame: ProtoStreamData) -> Result<StreamData, ConstellationError> {
    Ok(StreamData {
        stream: parse_stream_id(frame.stream, "stream_data stream")?,
        sequence: frame.sequence,
        channel: decode_stream_channel(frame.channel)?,
        cursor: frame
            .cursor
            .map(|c| parse_stream_cursor(c, "stream_data cursor"))
            .transpose()?,
        data: decode_payload(frame.data, "stream_data data")?,
    })
}

fn encode_stream_flow(frame: &StreamFlow) -> ProtoStreamFlow {
    ProtoStreamFlow {
        stream: frame.stream.as_str().to_owned(),
        credits: frame.credits,
    }
}

fn decode_stream_flow(frame: ProtoStreamFlow) -> Result<StreamFlow, ConstellationError> {
    if frame.credits == 0 {
        return Err(malformed("stream_flow credit grant must be non-zero"));
    }
    Ok(StreamFlow {
        stream: parse_stream_id(frame.stream, "stream_flow stream")?,
        credits: frame.credits,
    })
}

fn encode_stream_close(frame: &StreamClose) -> ProtoStreamClose {
    ProtoStreamClose {
        stream: frame.stream.as_str().to_owned(),
        reason: encode_close_reason(frame.reason),
    }
}

fn decode_stream_close(frame: ProtoStreamClose) -> Result<StreamClose, ConstellationError> {
    Ok(StreamClose {
        stream: parse_stream_id(frame.stream, "stream_close stream")?,
        reason: decode_close_reason(frame.reason)?,
    })
}

fn encode_stream_resume(frame: &StreamResume) -> ProtoStreamResume {
    ProtoStreamResume {
        stream: frame.stream.as_str().to_owned(),
        cursor: frame.cursor.as_str().to_owned(),
    }
}

fn decode_stream_resume(frame: ProtoStreamResume) -> Result<StreamResume, ConstellationError> {
    Ok(StreamResume {
        stream: parse_stream_id(frame.stream, "stream_resume stream")?,
        cursor: parse_stream_cursor(frame.cursor, "stream_resume cursor")?,
    })
}

fn encode_error(error: &ConstellationError) -> Result<ProtoError, ConstellationError> {
    Ok(ProtoError {
        kind: encode_error_kind(error.kind())?,
        capability: error
            .missing_capability()
            .map(encode_capability)
            .transpose()?,
        message: Some(error.message().to_owned()),
        negotiated_capability_fingerprint: error
            .negotiated_capability_fingerprint()
            .map(ToOwned::to_owned),
        correlation_id: error.correlation_id().map(|id| id.as_str().to_owned()),
    })
}

fn decode_error(error: ProtoError) -> Result<ConstellationError, ConstellationError> {
    let kind = decode_error_kind(error.kind)?;
    let capability = error.capability.map(decode_capability_strict).transpose()?;
    let message = error
        .message
        .ok_or_else(|| malformed("typed_error message is missing"))?;
    if kind == ErrorKind::CapabilityDenied {
        let capability = capability.ok_or_else(|| {
            malformed("capability-denied typed_error is missing the capability field")
        })?;
        let mut decoded = ConstellationError::capability_denied_with_fingerprint(
            capability,
            error.negotiated_capability_fingerprint,
        );
        if let Some(correlation_id) = error.correlation_id {
            decoded = decoded.with_correlation_id(parse_correlation_id(
                correlation_id,
                "typed_error correlation_id",
            )?);
        }
        if decoded.message() != message {
            return Err(malformed(
                "capability-denied typed_error message is not canonical",
            ));
        }
        Ok(decoded)
    } else if capability.is_some() || error.negotiated_capability_fingerprint.is_some() {
        Err(malformed(
            "non-capability-denied typed_error carries capability-denial context",
        ))
    } else {
        let mut decoded = ConstellationError::new(kind, message);
        if let Some(correlation_id) = error.correlation_id {
            decoded = decoded.with_correlation_id(parse_correlation_id(
                correlation_id,
                "typed_error correlation_id",
            )?);
        }
        Ok(decoded)
    }
}

fn encode_admission_audit(
    audit: &AdmissionAuditRecord,
) -> Result<ProtoAuditEnvelope, ConstellationError> {
    Ok(ProtoAuditEnvelope {
        operation_id: audit.operation_id.as_str().to_owned(),
        correlation_id: audit.correlation_id.as_str().to_owned(),
        realm: encode_realm(&audit.realm),
        principal: audit
            .principal
            .as_ref()
            .map(|principal| principal.as_str().to_owned()),
        node: audit.node.as_str().to_owned(),
        workload: None,
        stream: None,
        execution: None,
        scope: Some(encode_authorization_scope(audit.scope)?),
        decision: encode_authz_decision(audit.decision),
        trace: audit.trace.as_ref().map(encode_trace_context),
        reason: Some(encode_error_kind(audit.reason)?),
    })
}

fn decode_admission_audit(
    audit: ProtoAuditEnvelope,
) -> Result<AdmissionAuditRecord, ConstellationError> {
    if audit.workload.is_some() || audit.stream.is_some() || audit.execution.is_some() {
        return Err(malformed(
            "admission audit must not carry workload, stream, or execution fields",
        ));
    }
    let record = AdmissionAuditRecord {
        operation_id: parse_operation_id(audit.operation_id, "audit operation_id")?,
        correlation_id: parse_correlation_id(audit.correlation_id, "audit correlation_id")?,
        realm: decode_realm(audit.realm, "audit realm")?,
        principal: audit
            .principal
            .map(|principal| parse_principal_id(principal, "audit principal"))
            .transpose()?,
        node: parse_node_id(audit.node, "audit node")?,
        scope: decode_authorization_scope(
            audit
                .scope
                .ok_or_else(|| malformed("audit scope is missing"))?,
        )?,
        decision: decode_authz_decision(audit.decision)?,
        reason: audit
            .reason
            .ok_or_else(|| malformed("admission audit reason is missing"))
            .and_then(decode_error_kind)?,
        trace: audit.trace.map(decode_trace_context).transpose()?,
    };
    if record.is_admission_denial() {
        Ok(record)
    } else {
        Err(malformed("admission audit records must be denials"))
    }
}

fn encode_authorization_scope(
    scope: AuthorizationScope,
) -> Result<ProtoAuthorizationScope, ConstellationError> {
    let scope = match scope {
        AuthorizationScope::Capability { capability } => {
            proto_authorization_scope::Scope::Capability(encode_capability(capability)?)
        }
        AuthorizationScope::NodeControl => {
            proto_authorization_scope::Scope::NodeControl(ProtoUnit {})
        }
        AuthorizationScope::Enrollment => {
            proto_authorization_scope::Scope::Enrollment(ProtoUnit {})
        }
        AuthorizationScope::Health => proto_authorization_scope::Scope::Health(ProtoUnit {}),
    };
    Ok(ProtoAuthorizationScope { scope: Some(scope) })
}

fn decode_authorization_scope(
    scope: ProtoAuthorizationScope,
) -> Result<AuthorizationScope, ConstellationError> {
    match scope
        .scope
        .ok_or_else(|| malformed("authorization scope body is missing"))?
    {
        proto_authorization_scope::Scope::Capability(capability) => Ok(
            AuthorizationScope::capability(decode_capability_strict(capability)?),
        ),
        proto_authorization_scope::Scope::NodeControl(_) => Ok(AuthorizationScope::NodeControl),
        proto_authorization_scope::Scope::Enrollment(_) => Ok(AuthorizationScope::Enrollment),
        proto_authorization_scope::Scope::Health(_) => Ok(AuthorizationScope::Health),
    }
}

fn encode_trace_context(trace: &TraceContext) -> ProtoTraceContext {
    ProtoTraceContext {
        trace_id: trace.trace_id().to_owned(),
        span_id: trace.span_id().to_owned(),
    }
}

fn decode_trace_context(trace: ProtoTraceContext) -> Result<TraceContext, ConstellationError> {
    TraceContext::new(trace.trace_id, trace.span_id)
        .ok_or_else(|| malformed("trace context field is out of bounds"))
}

fn encode_payload(payload: &OpaquePayload) -> ProtoPayload {
    ProtoPayload {
        bytes: payload.as_bytes().to_vec(),
    }
}

fn decode_payload(
    payload: Option<ProtoPayload>,
    field: &'static str,
) -> Result<OpaquePayload, ConstellationError> {
    let payload = payload.ok_or_else(|| malformed(format!("{field} is missing")))?;
    OpaquePayload::new(payload.bytes).map_err(|err| malformed(format!("{field}: {err}")))
}

fn encode_realm(realm: &RealmPath) -> Vec<String> {
    realm
        .labels()
        .iter()
        .map(|label| label.as_str().to_owned())
        .collect()
}

fn decode_realm(labels: Vec<String>, field: &'static str) -> Result<RealmPath, ConstellationError> {
    let labels = labels
        .into_iter()
        .map(|label| RealmId::parse(label).map_err(|err| malformed(format!("{field}: {err}"))))
        .collect::<Result<Vec<_>, _>>()?;
    RealmPath::new(labels).ok_or_else(|| malformed(format!("{field} is empty or exceeds bounds")))
}

macro_rules! parse_id_fn {
    ($fn_name:ident, $ty:ty) => {
        fn $fn_name(raw: String, field: &'static str) -> Result<$ty, ConstellationError> {
            <$ty>::parse(raw).map_err(|err| malformed(format!("{field}: {err}")))
        }
    };
}

parse_id_fn!(parse_operation_id, OperationId);
parse_id_fn!(parse_correlation_id, CorrelationId);
parse_id_fn!(parse_idempotency_key, IdempotencyKey);
parse_id_fn!(parse_node_id, NodeId);
parse_id_fn!(parse_workload_id, WorkloadId);
parse_id_fn!(parse_principal_id, PrincipalId);
parse_id_fn!(parse_stream_id, StreamId);
parse_id_fn!(parse_stream_cursor, StreamCursor);
parse_id_fn!(parse_protocol_token, ProtocolToken);

fn encode_handshake_rejection(reason: HandshakeRejectedReason) -> i32 {
    match reason {
        HandshakeRejectedReason::VersionSkew => 1,
        HandshakeRejectedReason::CodecMismatch => 2,
        HandshakeRejectedReason::SchemaFingerprintMismatch => 3,
        HandshakeRejectedReason::ChannelBindingMismatch => 4,
        HandshakeRejectedReason::MalformedHandshake => 5,
        HandshakeRejectedReason::CapabilityMismatch => 6,
    }
}

fn decode_handshake_rejection(raw: i32) -> Result<HandshakeRejectedReason, ConstellationError> {
    match raw {
        1 => Ok(HandshakeRejectedReason::VersionSkew),
        2 => Ok(HandshakeRejectedReason::CodecMismatch),
        3 => Ok(HandshakeRejectedReason::SchemaFingerprintMismatch),
        4 => Ok(HandshakeRejectedReason::ChannelBindingMismatch),
        5 => Ok(HandshakeRejectedReason::MalformedHandshake),
        6 => Ok(HandshakeRejectedReason::CapabilityMismatch),
        _ => Err(malformed(format!(
            "unknown handshake rejection value {raw}"
        ))),
    }
}

fn encode_operation_kind(kind: OperationKind) -> Result<i32, ConstellationError> {
    Ok(match kind {
        OperationKind::NodeRegister => 1,
        OperationKind::NodeHeartbeat => 2,
        OperationKind::NodeCapabilities => 3,
        OperationKind::WorkloadList => 4,
        OperationKind::WorkloadStart => 5,
        OperationKind::WorkloadStop => 6,
        OperationKind::GuestHealth => 7,
        OperationKind::ExecStart => 8,
        OperationKind::ExecAttach => 9,
        OperationKind::ExecLogs => 10,
        OperationKind::ExecCancel => 11,
        OperationKind::FileCopyStart => 12,
        OperationKind::PortForwardOpen => 13,
        OperationKind::DisplaySessionOpen => 14,
        OperationKind::ShellList => 15,
        OperationKind::ShellAttach => 16,
        OperationKind::ShellDetach => 17,
        OperationKind::ShellKill => 18,
    })
}

fn decode_operation_kind(raw: i32) -> Result<OperationKind, ConstellationError> {
    match raw {
        1 => Ok(OperationKind::NodeRegister),
        2 => Ok(OperationKind::NodeHeartbeat),
        3 => Ok(OperationKind::NodeCapabilities),
        4 => Ok(OperationKind::WorkloadList),
        5 => Ok(OperationKind::WorkloadStart),
        6 => Ok(OperationKind::WorkloadStop),
        7 => Ok(OperationKind::GuestHealth),
        8 => Ok(OperationKind::ExecStart),
        9 => Ok(OperationKind::ExecAttach),
        10 => Ok(OperationKind::ExecLogs),
        11 => Ok(OperationKind::ExecCancel),
        12 => Ok(OperationKind::FileCopyStart),
        13 => Ok(OperationKind::PortForwardOpen),
        14 => Ok(OperationKind::DisplaySessionOpen),
        15 => Ok(OperationKind::ShellList),
        16 => Ok(OperationKind::ShellAttach),
        17 => Ok(OperationKind::ShellDetach),
        18 => Ok(OperationKind::ShellKill),
        _ => Err(malformed(format!("unknown operation kind value {raw}"))),
    }
}

fn encode_stream_kind(kind: StreamKind) -> Result<i32, ConstellationError> {
    Ok(match kind {
        StreamKind::Control => 1,
        StreamKind::Pty => 2,
        StreamKind::Stdio => 3,
        StreamKind::Logs => 4,
        StreamKind::FileCopy => 5,
        StreamKind::PortForward => 6,
        StreamKind::Display => 7,
        StreamKind::Clipboard => 8,
        StreamKind::AudioPlayback => 9,
        StreamKind::AudioCapture => 10,
        StreamKind::DeviceHid => 11,
        StreamKind::DeviceUsb => 12,
        StreamKind::ShellPty => 13,
        _ => return Err(malformed("unsupported stream kind")),
    })
}

fn decode_stream_kind(raw: i32) -> Result<StreamKind, ConstellationError> {
    match raw {
        1 => Ok(StreamKind::Control),
        2 => Ok(StreamKind::Pty),
        3 => Ok(StreamKind::Stdio),
        4 => Ok(StreamKind::Logs),
        5 => Ok(StreamKind::FileCopy),
        6 => Ok(StreamKind::PortForward),
        7 => Ok(StreamKind::Display),
        8 => Ok(StreamKind::Clipboard),
        9 => Ok(StreamKind::AudioPlayback),
        10 => Ok(StreamKind::AudioCapture),
        11 => Ok(StreamKind::DeviceHid),
        12 => Ok(StreamKind::DeviceUsb),
        13 => Ok(StreamKind::ShellPty),
        _ => Err(malformed(format!("unknown stream kind value {raw}"))),
    }
}

fn encode_stream_channel(channel: StreamChannel) -> i32 {
    match channel {
        StreamChannel::Primary => 0,
        StreamChannel::Stdout => 1,
        StreamChannel::Stderr => 2,
        _ => 0,
    }
}

fn decode_stream_channel(raw: i32) -> Result<StreamChannel, ConstellationError> {
    match raw {
        0 => Ok(StreamChannel::Primary),
        1 => Ok(StreamChannel::Stdout),
        2 => Ok(StreamChannel::Stderr),
        _ => Err(malformed(format!("unknown stream channel value {raw}"))),
    }
}

fn encode_close_reason(reason: StreamCloseReason) -> i32 {
    match reason {
        StreamCloseReason::Completed => 1,
        StreamCloseReason::Cancelled => 2,
        StreamCloseReason::TimedOut => 3,
        StreamCloseReason::Errored => 4,
        StreamCloseReason::PeerGone => 5,
        _ => 4,
    }
}

fn decode_close_reason(raw: i32) -> Result<StreamCloseReason, ConstellationError> {
    match raw {
        1 => Ok(StreamCloseReason::Completed),
        2 => Ok(StreamCloseReason::Cancelled),
        3 => Ok(StreamCloseReason::TimedOut),
        4 => Ok(StreamCloseReason::Errored),
        5 => Ok(StreamCloseReason::PeerGone),
        _ => Err(malformed(format!(
            "unknown stream close reason value {raw}"
        ))),
    }
}

fn encode_capability(capability: Capability) -> Result<i32, ConstellationError> {
    Ok(match capability {
        Capability::Lifecycle => 1,
        Capability::Exec => 2,
        Capability::Pty => 3,
        Capability::Logs => 4,
        Capability::FileCopy => 5,
        Capability::PortForward => 6,
        Capability::Vsock => 7,
        Capability::Virtiofs => 8,
        Capability::WindowForwarding => 9,
        Capability::DisplayStreaming => 10,
        Capability::Clipboard => 11,
        Capability::AudioPlayback => 12,
        Capability::AudioCapture => 13,
        Capability::Hid => 14,
        Capability::Usb => 15,
        Capability::GpuAccel => 16,
        Capability::Snapshots => 17,
        Capability::Hotplug => 18,
        Capability::EphemeralSessions => 19,
        Capability::ProviderManagedIsolation => 20,
        Capability::PersistentShell => 21,
    })
}

fn decode_capability(raw: i32) -> Option<Capability> {
    match raw {
        1 => Some(Capability::Lifecycle),
        2 => Some(Capability::Exec),
        3 => Some(Capability::Pty),
        4 => Some(Capability::Logs),
        5 => Some(Capability::FileCopy),
        6 => Some(Capability::PortForward),
        7 => Some(Capability::Vsock),
        8 => Some(Capability::Virtiofs),
        9 => Some(Capability::WindowForwarding),
        10 => Some(Capability::DisplayStreaming),
        11 => Some(Capability::Clipboard),
        12 => Some(Capability::AudioPlayback),
        13 => Some(Capability::AudioCapture),
        14 => Some(Capability::Hid),
        15 => Some(Capability::Usb),
        16 => Some(Capability::GpuAccel),
        17 => Some(Capability::Snapshots),
        18 => Some(Capability::Hotplug),
        19 => Some(Capability::EphemeralSessions),
        20 => Some(Capability::ProviderManagedIsolation),
        21 => Some(Capability::PersistentShell),
        _ => None,
    }
}

fn decode_capability_strict(raw: i32) -> Result<Capability, ConstellationError> {
    decode_capability(raw).ok_or_else(|| malformed(format!("unknown capability value {raw}")))
}

fn encode_error_kind(kind: ErrorKind) -> Result<i32, ConstellationError> {
    Ok(match kind {
        ErrorKind::CapabilityDenied => 1,
        ErrorKind::Unauthorized => 2,
        ErrorKind::NoRealmEntrypoint => 3,
        ErrorKind::GatewayUnavailable => 4,
        ErrorKind::ProviderAllocationFailed => 5,
        ErrorKind::RelayUnavailable => 6,
        ErrorKind::AuthenticationFailed => 7,
        ErrorKind::VersionSkew => 8,
        ErrorKind::OperationInProgress => 9,
        ErrorKind::IdempotencyKeyConflict => 10,
        ErrorKind::IdempotencyKeyExpired => 11,
        ErrorKind::Backpressure => 12,
        ErrorKind::Cancelled => 13,
        ErrorKind::Timeout => 14,
        ErrorKind::FrameTooLarge => 15,
        ErrorKind::MalformedFrame => 16,
        ErrorKind::InvalidTarget => 17,
        ErrorKind::AuditUnavailable => 18,
        ErrorKind::UnsupportedFeature => 19,
    })
}

fn decode_error_kind(raw: i32) -> Result<ErrorKind, ConstellationError> {
    match raw {
        1 => Ok(ErrorKind::CapabilityDenied),
        2 => Ok(ErrorKind::Unauthorized),
        3 => Ok(ErrorKind::NoRealmEntrypoint),
        4 => Ok(ErrorKind::GatewayUnavailable),
        5 => Ok(ErrorKind::ProviderAllocationFailed),
        6 => Ok(ErrorKind::RelayUnavailable),
        7 => Ok(ErrorKind::AuthenticationFailed),
        8 => Ok(ErrorKind::VersionSkew),
        9 => Ok(ErrorKind::OperationInProgress),
        10 => Ok(ErrorKind::IdempotencyKeyConflict),
        11 => Ok(ErrorKind::IdempotencyKeyExpired),
        12 => Ok(ErrorKind::Backpressure),
        13 => Ok(ErrorKind::Cancelled),
        14 => Ok(ErrorKind::Timeout),
        15 => Ok(ErrorKind::FrameTooLarge),
        16 => Ok(ErrorKind::MalformedFrame),
        17 => Ok(ErrorKind::InvalidTarget),
        18 => Ok(ErrorKind::AuditUnavailable),
        19 => Ok(ErrorKind::UnsupportedFeature),
        _ => Err(malformed(format!("unknown error kind value {raw}"))),
    }
}

fn encode_authz_decision(decision: AuthzDecision) -> i32 {
    match decision {
        AuthzDecision::Allow => 1,
        AuthzDecision::Deny => 2,
    }
}

fn decode_authz_decision(raw: i32) -> Result<AuthzDecision, ConstellationError> {
    match raw {
        1 => Ok(AuthzDecision::Allow),
        2 => Ok(AuthzDecision::Deny),
        _ => Err(malformed(format!("unknown authz decision value {raw}"))),
    }
}

fn malformed(message: impl Into<String>) -> ConstellationError {
    ConstellationError::new(ErrorKind::MalformedFrame, message)
}

fn frame_too_large(message: impl Into<String>) -> ConstellationError {
    ConstellationError::new(ErrorKind::FrameTooLarge, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct JsonCodec;

    impl ProtocolCodec for JsonCodec {
        fn codec_id(&self) -> &str {
            "json.test"
        }

        fn encode_frame(&self, frame: &ConstellationFrame) -> Result<Vec<u8>, ConstellationError> {
            serde_json::to_vec(frame)
                .map_err(|err| ConstellationError::new(ErrorKind::MalformedFrame, err.to_string()))
        }

        fn decode_frame(&self, bytes: &[u8]) -> Result<ConstellationFrame, ConstellationError> {
            serde_json::from_slice(bytes)
                .map_err(|err| ConstellationError::new(ErrorKind::MalformedFrame, err.to_string()))
        }

        fn schema_fingerprint(&self) -> String {
            "json.test/semantic-serde".to_owned()
        }
    }

    #[test]
    fn protobuf_round_trips_every_frame_variant() {
        let codec = ProtobufCodec::new();
        for frame in sample_frames() {
            let bytes = codec.encode_frame(&frame).unwrap();
            let decoded = codec.decode_frame(&bytes).unwrap();
            assert_eq!(decoded, frame);
        }
    }

    #[test]
    fn protobuf_decode_fails_closed_for_malformed_inputs() {
        let codec = ProtobufCodec::new();
        let garbage = codec.decode_frame(&[0xff, 0xff, 0xff]);
        assert_malformed(garbage);

        let invalid = ProtoFrame {
            body: Some(proto_frame::Body::OperationRequest(ProtoOperationRequest {
                operation_id: "op has spaces".to_owned(),
                idempotency_key: None,
                realm: vec!["work".to_owned()],
                node: "node-a".to_owned(),
                workload: None,
                principal: "principal-1".to_owned(),
                kind: encode_operation_kind(OperationKind::WorkloadList).unwrap(),
                trace: None,
                body: Some(ProtoPayload { bytes: Vec::new() }),
                correlation_id: "corr-1".to_owned(),
            })),
        }
        .encode_to_vec();
        assert_malformed(codec.decode_frame(&invalid));
    }

    #[test]
    fn protobuf_operation_response_requires_bounded_correlation_id() {
        let codec = ProtobufCodec::new();
        let missing = ProtoFrame {
            body: Some(proto_frame::Body::OperationResponse(
                ProtoOperationResponse {
                    operation_id: "op-1".to_owned(),
                    body: Some(ProtoPayload { bytes: Vec::new() }),
                    correlation_id: String::new(),
                },
            )),
        }
        .encode_to_vec();
        assert_malformed(codec.decode_frame(&missing));

        let malformed = ProtoFrame {
            body: Some(proto_frame::Body::OperationResponse(
                ProtoOperationResponse {
                    operation_id: "op-1".to_owned(),
                    body: Some(ProtoPayload { bytes: Vec::new() }),
                    correlation_id: "secret-token".to_owned(),
                },
            )),
        }
        .encode_to_vec();
        assert_malformed(codec.decode_frame(&malformed));
    }

    #[test]
    fn protobuf_operation_response_round_trips_correlation_id() {
        let codec = ProtobufCodec::new();
        let frame = ConstellationFrame::OperationResponse(OperationResponse {
            operation_id: operation_id("op-1"),
            correlation_id: CorrelationId::parse("corr-response-1").unwrap(),
            body: payload(b"response-body"),
        });

        let decoded = codec
            .decode_frame(&codec.encode_frame(&frame).unwrap())
            .unwrap();
        let ConstellationFrame::OperationResponse(response) = decoded else {
            panic!("expected operation response frame");
        };
        assert_eq!(response.correlation_id.as_str(), "corr-response-1");
    }

    #[test]
    fn protobuf_decode_rejects_oversized_input_before_prost_decode() {
        let codec = ProtobufCodec::new();
        let oversized = vec![0xff; MAX_FRAME_SIZE + 1];

        assert_frame_too_large(codec.decode_frame(&oversized));
    }

    #[test]
    fn protobuf_decode_rejects_unknown_operation_kind_enum_value() {
        let codec = ProtobufCodec::new();
        let invalid = ProtoFrame {
            body: Some(proto_frame::Body::OperationRequest(ProtoOperationRequest {
                operation_id: "op-1".to_owned(),
                idempotency_key: None,
                realm: vec!["work".to_owned()],
                node: "node-a".to_owned(),
                workload: None,
                principal: "principal-1".to_owned(),
                kind: 99,
                trace: None,
                body: Some(ProtoPayload { bytes: Vec::new() }),
                correlation_id: "corr-1".to_owned(),
            })),
        }
        .encode_to_vec();

        assert_malformed(codec.decode_frame(&invalid));
    }

    #[test]
    fn shell_enum_integer_mappings_are_stable() {
        assert_eq!(encode_operation_kind(OperationKind::ShellList).unwrap(), 15);
        assert_eq!(
            encode_operation_kind(OperationKind::ShellAttach).unwrap(),
            16
        );
        assert_eq!(
            encode_operation_kind(OperationKind::ShellDetach).unwrap(),
            17
        );
        assert_eq!(encode_operation_kind(OperationKind::ShellKill).unwrap(), 18);
        assert_eq!(decode_operation_kind(15).unwrap(), OperationKind::ShellList);
        assert_eq!(
            decode_operation_kind(16).unwrap(),
            OperationKind::ShellAttach
        );
        assert_eq!(
            decode_operation_kind(17).unwrap(),
            OperationKind::ShellDetach
        );
        assert_eq!(decode_operation_kind(18).unwrap(), OperationKind::ShellKill);

        assert_eq!(encode_stream_kind(StreamKind::ShellPty).unwrap(), 13);
        assert_eq!(decode_stream_kind(13).unwrap(), StreamKind::ShellPty);

        assert_eq!(encode_capability(Capability::PersistentShell).unwrap(), 21);
        assert_eq!(
            decode_capability_strict(21).unwrap(),
            Capability::PersistentShell
        );
    }

    #[test]
    fn protobuf_decode_ignores_unknown_advertised_capabilities() {
        let codec = ProtobufCodec::new();
        let expected_caps: CapabilitySet =
            serde_json::from_str("[\"exec\",\"unknown-protobuf-capability-999\"]").unwrap();
        let encoded = ProtoFrame {
            body: Some(proto_frame::Body::Handshake(ProtoHandshake {
                protocol_version: Some(1),
                codec_id: CODEC_ID.to_owned(),
                schema_fingerprint: SCHEMA_FINGERPRINT.to_owned(),
                capabilities: vec![encode_capability(Capability::Exec).unwrap(), 999],
                capability_schema_version:
                    d2b_realm_core::capability::CAPABILITY_NEGOTIATION_SCHEMA_VERSION,
                capability_fingerprint: expected_caps.stable_fingerprint(),
                peer: None,
            })),
        }
        .encode_to_vec();

        let decoded = codec.decode_frame(&encoded).unwrap();
        let ConstellationFrame::Handshake(handshake) = decoded else {
            panic!("expected handshake frame");
        };
        assert!(handshake.capabilities.capabilities.has(Capability::Exec));
        assert!(!handshake.capabilities.capabilities.has(Capability::Logs));
        assert_eq!(
            handshake
                .capabilities
                .capabilities
                .unknown_iter()
                .map(ProtocolToken::as_str)
                .collect::<Vec<_>>(),
            ["unknown-protobuf-capability-999"]
        );
        assert!(
            serde_json::to_string(&handshake.capabilities.capabilities)
                .unwrap()
                .contains("unknown-protobuf-capability-999")
        );
        let reencoded = encode_handshake(&handshake).unwrap();
        assert!(reencoded.capabilities.contains(&999));
    }

    #[test]
    fn protobuf_decode_rejects_unknown_handshake_rejection_reason() {
        let codec = ProtobufCodec::new();
        let invalid = ProtoFrame {
            body: Some(proto_frame::Body::HandshakeRejected(
                ProtoHandshakeRejected { reason: 99 },
            )),
        }
        .encode_to_vec();

        assert_malformed(codec.decode_frame(&invalid));
    }

    #[test]
    fn protobuf_decode_rejects_missing_required_stream_id() {
        let codec = ProtobufCodec::new();
        let invalid = ProtoFrame {
            body: Some(proto_frame::Body::StreamClose(ProtoStreamClose {
                stream: String::new(),
                reason: encode_close_reason(StreamCloseReason::Completed),
            })),
        }
        .encode_to_vec();

        assert_malformed(codec.decode_frame(&invalid));
    }

    #[test]
    fn protobuf_decode_rejects_missing_frame_body() {
        let codec = ProtobufCodec::new();
        let invalid = ProtoFrame { body: None }.encode_to_vec();

        assert_malformed(codec.decode_frame(&invalid));
    }

    #[test]
    fn protobuf_decode_rejects_inconsistent_stream_open_authz() {
        let codec = ProtobufCodec::new();
        let invalid = ProtoFrame {
            body: Some(proto_frame::Body::StreamOpen(ProtoStreamOpen {
                descriptor: Some(ProtoStreamDescriptor {
                    id: "stream-1".to_owned(),
                    kind: encode_stream_kind(StreamKind::Display).unwrap(),
                }),
                operation_id: "op-1".to_owned(),
                authz: Some(ProtoStreamAuthz {
                    principal: "principal-1".to_owned(),
                    realm: vec!["work".to_owned()],
                    capability: encode_capability(Capability::Exec).unwrap(),
                }),
            })),
        }
        .encode_to_vec();

        assert_malformed(codec.decode_frame(&invalid));
    }

    #[test]
    fn protobuf_decode_rejects_admission_audit_with_workload_stream_or_execution() {
        let codec = ProtobufCodec::new();
        for invalid_audit in [
            ProtoAuditEnvelope {
                workload: Some("workload-a".to_owned()),
                ..valid_admission_audit_proto()
            },
            ProtoAuditEnvelope {
                stream: Some("stream-1".to_owned()),
                ..valid_admission_audit_proto()
            },
            ProtoAuditEnvelope {
                execution: Some("exec-1".to_owned()),
                ..valid_admission_audit_proto()
            },
        ] {
            let invalid = ProtoFrame {
                body: Some(proto_frame::Body::AdmissionAudit(invalid_audit)),
            }
            .encode_to_vec();
            assert_malformed(codec.decode_frame(&invalid));
        }
    }

    #[test]
    fn alternate_codec_round_trips_the_same_semantic_frames() {
        let protobuf = ProtobufCodec::new();
        let json = JsonCodec;
        for frame in sample_frames() {
            let protobuf_decoded = protobuf
                .decode_frame(&protobuf.encode_frame(&frame).unwrap())
                .unwrap();
            let json_decoded = json
                .decode_frame(&json.encode_frame(&frame).unwrap())
                .unwrap();
            assert_eq!(protobuf_decoded, frame);
            assert_eq!(json_decoded, frame);
            assert_eq!(protobuf_decoded, json_decoded);
        }
    }

    fn assert_malformed(result: Result<ConstellationFrame, ConstellationError>) {
        let err = result.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedFrame);
    }

    fn assert_frame_too_large(result: Result<ConstellationFrame, ConstellationError>) {
        let err = result.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::FrameTooLarge);
    }

    fn sample_frames() -> Vec<ConstellationFrame> {
        let realm = realm();
        let operation_id = operation_id("op-1");
        let principal = principal("principal-1");
        let node = node("node-a");
        let workload = workload("workload-a");
        let stream = stream("stream-1");
        let trace = trace();

        vec![
            ConstellationFrame::Handshake(Handshake {
                protocol_version: 1,
                codec_id: token(CODEC_ID),
                schema_fingerprint: token(SCHEMA_FINGERPRINT),
                capabilities: CapabilitySet::empty()
                    .with(Capability::Exec)
                    .with(Capability::WindowForwarding)
                    .negotiation(),
                peer: Some(PeerContext {
                    auth_mechanism: token("none"),
                    peer_principal: Some(principal.clone()),
                    peer_node: Some(node.clone()),
                }),
            }),
            ConstellationFrame::HandshakeAccepted(HandshakeAccepted {
                selected: Handshake {
                    protocol_version: 1,
                    codec_id: token(CODEC_ID),
                    schema_fingerprint: token(SCHEMA_FINGERPRINT),
                    capabilities: CapabilitySet::empty().negotiation(),
                    peer: None,
                },
            }),
            ConstellationFrame::HandshakeRejected(HandshakeRejected {
                reason: HandshakeRejectedReason::SchemaFingerprintMismatch,
            }),
            ConstellationFrame::OperationRequest(OperationRequest {
                operation_id: operation_id.clone(),
                correlation_id: correlation_id(),
                idempotency_key: Some(idempotency_key("idem-1")),
                realm: realm.clone(),
                node: node.clone(),
                workload: Some(workload.clone()),
                principal: principal.clone(),
                kind: OperationKind::WorkloadStart,
                trace: Some(trace.clone()),
                body: payload(b"request-body"),
            }),
            ConstellationFrame::OperationResponse(OperationResponse {
                operation_id: operation_id.clone(),
                correlation_id: correlation_id(),
                body: payload(b"response-body"),
            }),
            ConstellationFrame::StreamOpen(StreamOpen {
                descriptor: StreamDescriptor {
                    id: stream.clone(),
                    kind: StreamKind::Display,
                },
                operation_id: operation_id.clone(),
                authz: StreamAuthz::for_kind(principal.clone(), realm.clone(), StreamKind::Display),
            }),
            ConstellationFrame::StreamData(StreamData {
                stream: stream.clone(),
                sequence: 0,
                channel: StreamChannel::Primary,
                cursor: None,
                data: payload(b"stream-data"),
            }),
            ConstellationFrame::StreamData(StreamData {
                stream: stream.clone(),
                sequence: 1,
                channel: StreamChannel::Stderr,
                cursor: Some(StreamCursor::parse("cur-1").unwrap()),
                data: payload(b"logs-chunk"),
            }),
            ConstellationFrame::StreamFlow(StreamFlow {
                stream: stream.clone(),
                credits: 8,
            }),
            ConstellationFrame::StreamClose(StreamClose {
                stream: stream.clone(),
                reason: StreamCloseReason::Completed,
            }),
            ConstellationFrame::StreamResume(StreamResume {
                stream: stream.clone(),
                cursor: StreamCursor::parse("cur-resume").unwrap(),
            }),
            ConstellationFrame::TypedError(
                ConstellationError::capability_denied(Capability::WindowForwarding)
                    .with_correlation_id(correlation_id()),
            ),
            ConstellationFrame::AdmissionAudit(
                AdmissionAuditRecord::denied(
                    operation_id,
                    correlation_id(),
                    realm,
                    node,
                    AuthorizationScope::capability(Capability::Exec),
                    ErrorKind::AuthenticationFailed,
                )
                .with_principal(principal)
                .with_trace(trace),
            ),
        ]
    }

    fn token(raw: &str) -> ProtocolToken {
        ProtocolToken::parse(raw).unwrap()
    }

    fn operation_id(raw: &str) -> OperationId {
        OperationId::parse(raw).unwrap()
    }

    fn correlation_id() -> CorrelationId {
        CorrelationId::parse("corr-1").unwrap()
    }

    fn idempotency_key(raw: &str) -> IdempotencyKey {
        IdempotencyKey::parse(raw).unwrap()
    }

    fn node(raw: &str) -> NodeId {
        NodeId::parse(raw).unwrap()
    }

    fn workload(raw: &str) -> WorkloadId {
        WorkloadId::parse(raw).unwrap()
    }

    fn principal(raw: &str) -> PrincipalId {
        PrincipalId::parse(raw).unwrap()
    }

    fn stream(raw: &str) -> StreamId {
        StreamId::parse(raw).unwrap()
    }

    fn valid_admission_audit_proto() -> ProtoAuditEnvelope {
        ProtoAuditEnvelope {
            operation_id: "op-1".to_owned(),
            correlation_id: "corr-1".to_owned(),
            realm: vec!["work".to_owned()],
            principal: None,
            node: "node-a".to_owned(),
            workload: None,
            stream: None,
            execution: None,
            scope: Some(ProtoAuthorizationScope {
                scope: Some(proto_authorization_scope::Scope::Enrollment(ProtoUnit {})),
            }),
            decision: encode_authz_decision(AuthzDecision::Deny),
            trace: None,
            reason: Some(encode_error_kind(ErrorKind::AuthenticationFailed).unwrap()),
        }
    }

    fn realm() -> RealmPath {
        RealmPath::new(vec![
            RealmId::parse("payments").unwrap(),
            RealmId::parse("work").unwrap(),
        ])
        .unwrap()
    }

    fn trace() -> TraceContext {
        TraceContext::new("trace-1", "span-1").unwrap()
    }

    fn payload(bytes: &[u8]) -> OpaquePayload {
        OpaquePayload::new(bytes.to_vec()).unwrap()
    }
}
