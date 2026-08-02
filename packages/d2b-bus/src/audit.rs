//! Route-admission audit records.

use d2b_audit::TraceContext;
use d2b_audit::{
    AuditHash, AuditRecord, AuditRecordError, AuditRecordFields, AuditSink, AuditSinkError,
    AuditWriteClass, AuditWriteOutcome, RouteAdmissionFields,
};

/// Construct one route-admission record with no resource identity fields.
#[allow(clippy::too_many_arguments)]
pub fn route_admission_record(
    ts_ms: u64,
    zone: impl Into<String>,
    operation_id: impl Into<String>,
    correlation_id: impl Into<String>,
    source: impl Into<String>,
    previous_hash: AuditHash,
    service: impl Into<String>,
    method: impl Into<String>,
    direction: impl Into<String>,
    subject_digest: impl Into<String>,
    authz_decision: impl Into<String>,
    authz_revision: u64,
    outcome: impl Into<String>,
) -> Result<AuditRecord, AuditRecordError> {
    route_admission_record_with_trace(
        ts_ms,
        zone,
        operation_id,
        correlation_id,
        source,
        previous_hash,
        service,
        method,
        direction,
        subject_digest,
        authz_decision,
        authz_revision,
        outcome,
        None,
    )
}

/// Construct a route-admission record with an optional propagated trace.
#[allow(clippy::too_many_arguments)]
pub fn route_admission_record_with_trace(
    ts_ms: u64,
    zone: impl Into<String>,
    operation_id: impl Into<String>,
    correlation_id: impl Into<String>,
    source: impl Into<String>,
    previous_hash: AuditHash,
    service: impl Into<String>,
    method: impl Into<String>,
    direction: impl Into<String>,
    subject_digest: impl Into<String>,
    authz_decision: impl Into<String>,
    authz_revision: u64,
    outcome: impl Into<String>,
    trace: Option<&TraceContext>,
) -> Result<AuditRecord, AuditRecordError> {
    AuditRecord::new(
        ts_ms,
        zone,
        operation_id,
        correlation_id,
        trace.map(|trace| trace.trace_id().to_owned()),
        source,
        previous_hash,
        AuditRecordFields::RouteAdmission(RouteAdmissionFields {
            service: service.into(),
            method: method.into(),
            direction: direction.into(),
            subject_digest: subject_digest.into(),
            authz_decision: authz_decision.into(),
            authz_revision,
            outcome: outcome.into(),
        }),
    )
}

/// Typed bridge from route resolution to the durable audit sink.
pub struct BusAuditWriter<'a> {
    sink: &'a AuditSink,
}

impl<'a> BusAuditWriter<'a> {
    /// Borrow a bus audit sink.
    pub const fn new(sink: &'a AuditSink) -> Self {
        Self { sink }
    }

    /// Append one route-admission decision.
    pub fn append_route(&self, record: &AuditRecord) -> Result<AuditWriteOutcome, AuditSinkError> {
        self.sink.append(AuditWriteClass::Standard, record)
    }
}

/// Closed bus error class used by metrics and audit callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusErrorKind {
    /// Route succeeded.
    Ok,
    /// Route was denied.
    Denied,
    /// Service or method was not found.
    NotFound,
    /// Internal route failure.
    Error,
}

impl BusErrorKind {
    /// Stable label.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::NotFound => "not_found",
            Self::Error => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_audit::genesis_hash;

    #[test]
    fn route_error_codes_are_closed() {
        assert_eq!(BusErrorKind::Denied.code(), "denied");
        let record = route_admission_record(
            1,
            "work",
            "op",
            "corr",
            "bus",
            genesis_hash(),
            "d2b.resource.v3",
            "Get",
            "local",
            "sha256:subject",
            "denied",
            1,
            "denied",
        )
        .unwrap();
        assert_eq!(record.class().as_str(), "route-admission");
    }

    #[test]
    fn route_trace_propagation_keeps_span_id_out_of_audit() {
        let trace = TraceContext::new("trace-id", "span-id").unwrap();
        let record = route_admission_record_with_trace(
            1,
            "work",
            "op",
            "corr",
            "bus",
            genesis_hash(),
            "d2b.resource.v3",
            "Get",
            "local",
            "sha256:subject",
            "allowed",
            1,
            "ok",
            Some(&trace),
        )
        .unwrap();
        assert_eq!(record.trace_id(), Some("trace-id"));
        assert!(!serde_json::to_string(&record).unwrap().contains("span-id"));
    }
}
