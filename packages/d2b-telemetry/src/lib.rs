//! Bounded telemetry primitives for d2b core processes.
//!
//! This crate intentionally has no OpenTelemetry SDK dependency. The
//! observability Provider owns decoding, aggregation, and export.

#![forbid(unsafe_code)]

pub mod audit_hash;
pub mod emitter;
pub mod meter_registry;
pub mod metric_label_policy;
pub mod redaction_guard;
pub mod session_metrics_sink;
pub mod trace_context;

pub use audit_hash::{AuditChainLink, AuditHash, ChainVerificationError};
pub use emitter::{
    BoundedEmitter, DEFAULT_RING_CAPACITY_BYTES, DropSnapshot, EmitOutcome, EmitterError, Signal,
};
pub use metric_label_policy::{
    FORBIDDEN_LABEL_KEYS, FORBIDDEN_LABEL_SUFFIXES, IdentityCanaries, LabelDescriptor,
    MetricDescriptor, MetricPolicyError, OTEL_RESOURCE_ATTRIBUTES, allowed_values,
    validate_data_point, validate_descriptor, validate_label_key,
};
pub use redaction_guard::{RedactionError, RedactionGuard};
pub use trace_context::{MAX_TRACE_FIELD_LEN, TraceContext};
