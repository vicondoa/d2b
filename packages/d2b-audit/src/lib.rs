//! Durable, hash-chained, redacted Zone audit records.

#![forbid(unsafe_code)]

pub mod export;
pub mod hash_chain;
pub mod rate_limit;
pub mod record_types;
pub mod segment;
pub mod sink;

pub use export::{ExportLine, export_segments, export_segments_range, is_segment_name};
pub use hash_chain::{AuditChainLink, AuditHash, AuditHashError, genesis_hash};
pub use rate_limit::{
    AuditRateLimiter, AuditWriteClass, DEFAULT_AUDIT_WRITES_PER_SECOND, RateDecision,
};
pub use record_types::{
    AUDIT_SCHEMA_VERSION, AuditRecord, AuditRecordClass, AuditRecordError, AuditRecordFields,
    BrokerEffectFields, ProcessEffectFields, RbacChangeFields, ResourceMutationFields,
    ResourceShareFields, ResourceUpgradeFields, RouteAdmissionFields, SessionConnectFields,
    StateResetFields,
};
pub use segment::{DEFAULT_MAX_SEGMENT_BYTES, DEFAULT_RETENTION_DAYS, SegmentWriter};
pub use sink::{AuditSink, AuditSinkError, AuditWriteOutcome};
