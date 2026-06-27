//! `d2b-constellation-core` is the pure, codec-neutral v2
//! constellation model (ADR 0032). It defines identifiers, the realm
//! model, the capability model, node/workload/execution/stream DTOs, the
//! persistent-shell contract, the audit envelope, the semantic `ConstellationFrame`, a bounded
//! `TraceContext`, and the typed error surface.
//!
//! Invariants:
//!
//! - `#![forbid(unsafe_code)]` (inherited via workspace lints).
//! - **No** dependency on `prost`, generated protobuf, any
//!   `d2b-constellation-codec-*`, any transport crate, or any
//!   host-only broker/daemon internals. Codecs map bytes to/from the
//!   semantic [`frame::ConstellationFrame`]; the operation/routing layer
//!   never depends on a wire encoding.
//! - DTOs are `serde` + `schemars` and security-sensitive structures use
//!   `deny_unknown_fields` (ADR 0010 strict wire discipline).

pub mod audit;
pub mod capability;
pub mod error;
pub mod execution;
pub mod frame;
pub mod ids;
pub mod mux;
pub mod node;
pub mod payload;
pub mod realm;
pub mod shell;
pub mod stream;
pub mod target;
pub mod token;
pub mod trace_context;
pub mod workload;

pub use audit::{AdmissionAuditRecord, AuditEnvelope, AuthorizationScope, AuthzDecision};
pub use capability::{Capability, CapabilityNegotiation, CapabilitySet};
pub use error::{ConstellationError, ErrorKind};
pub use execution::{
    ExecAttachMode, ExecAttachRequest, ExecCancelRequest, ExecLogsRequest, ExecStartRequest,
    ExecState, ExecutionGeneration, ExecutionSummary,
};
pub use frame::{
    ConstellationFrame, Handshake, HandshakeAccepted, HandshakeRejected, HandshakeRejectedReason,
    OperationKind, OperationRequest, OperationResponse, PeerContext, StreamClose, StreamData,
    StreamFlow, StreamOpen, StreamResume,
};
pub use ids::{
    ExecutionId, GatewayId, IdempotencyKey, NodeId, OperationId, PrincipalId, ProviderId, RealmId,
    StreamCursor, StreamId, WorkloadId,
};
pub use mux::{DEFAULT_MAX_OPEN_STREAMS, StreamMux};
pub use node::{NodeKind, NodeSummary};
pub use payload::OpaquePayload;
pub use realm::{EntrypointMode, RealmPath};
pub use shell::{
    ShellAttachId, ShellAttachRequest, ShellAttachSummary, ShellCause, ShellDetachRequest,
    ShellEventBatch, ShellEventSummary, ShellGeneration, ShellKillRequest, ShellListRequest,
    ShellListResponse, ShellName, ShellNameError, ShellOpaqueIdError, ShellSessionInstanceId,
    ShellState, ShellSummary,
};
pub use stream::{StreamAuthz, StreamChannel, StreamCloseReason, StreamDescriptor, StreamKind};
pub use target::{TARGET_SUFFIX, THIS_NODE_ALIAS, TargetName, TargetParseError};
pub use token::ProtocolToken;
pub use trace_context::TraceContext;
pub use workload::{WorkloadSelector, WorkloadState, WorkloadSummary};
