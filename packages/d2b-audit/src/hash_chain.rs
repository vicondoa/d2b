//! Hash-chain exports for the audit crate.

pub use d2b_telemetry::audit_hash::{
    AuditChainLink, AuditHash, AuditHashError, ChainVerificationError, genesis_hash, payload_hash,
    record_hash,
};
