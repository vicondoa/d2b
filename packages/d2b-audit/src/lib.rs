//! Durable, hash-chained, redacted Zone audit records.

#![forbid(unsafe_code)]

pub mod evidence_chain;
pub mod export;
pub mod generated;
pub mod hash_chain;
pub mod operation;
pub mod rate_limit;
pub mod reconcile;
pub mod record_types;
pub mod segment;
pub mod sink;

pub use d2b_telemetry::TraceContext;
pub use evidence_chain::{
    ChainAuditSink, ChainLeg, ChainOutcome, ChainRecord, ChainRecordClass, EvidenceChain,
    MAX_NESTED_DEPTH, NESTED_DEPTH_EXCEEDED, root_record_count,
};
pub use hash_chain::{
    AuditChainLink, AuditHash, AuditHashError, ChainVerificationError, genesis_hash,
    is_canonical_digest, payload_hash, record_hash,
};
pub use operation::{
    OperationIdentity, OperationIdentityError, ZoneId, ZoneOperationKey,
    operation_identity_of_canonical_json, opaque_identity,
};
pub use reconcile::{
    DurabilityEvidence, EvidenceError, evidence_from_decision_result,
};
