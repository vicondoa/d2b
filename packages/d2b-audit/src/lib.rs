//! Durable, hash-chained, redacted Zone audit records.

#![forbid(unsafe_code)]

pub mod export;
pub mod hash_chain;
pub mod operation;
pub mod rate_limit;
pub mod reconcile;
pub mod record_types;
pub mod segment;
pub mod sink;

pub use d2b_telemetry::TraceContext;
pub use export::{ExportLine, export_segments, export_segments_range, is_segment_name};
pub use hash_chain::{
    AuditChainLink, AuditHash, AuditHashError, ChainVerificationError, genesis_hash,
    is_canonical_digest, payload_hash, record_hash,
};
pub use operation::{
    OperationIdentity, OperationIdentityError, ZoneId, ZoneOperationKey, opaque_identity,
};
pub use rate_limit::{
    AuditRateLimiter, AuditWriteClass, DEFAULT_AUDIT_WRITES_PER_SECOND, RateDecision,
};
pub use reconcile::{
    DurabilityEvidence, DurabilityOutcome, EvidenceError, Reconciliation,
    evidence_from_decision_result, reconcile as reconcile_durability,
};
pub use record_types::{
    AUDIT_SCHEMA_VERSION, AuditRecord, AuditRecordClass, AuditRecordError, AuditRecordFields,
    BrokerEffectFields, ProcessEffectFields, RbacChangeFields, ResourceMutationFields,
    ResourceShareFields, ResourceUpgradeFields, RouteAdmissionFields, SessionConnectFields,
    StateResetFields,
};
pub use segment::{
    DEFAULT_MAX_SEGMENT_BYTES, DEFAULT_RETENTION_DAYS, FailureInjector, FailurePoint, SegmentWriter,
};
pub use sink::{AuditSink, AuditSinkError, AuditWriteOutcome};
