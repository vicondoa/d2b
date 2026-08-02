//! Route-admission audit records.

use crate::routing::{RouteDirection, RouteTableError};
use d2b_audit::TraceContext;
use d2b_audit::{
    AuditHash, AuditRecord, AuditRecordError, AuditRecordFields, AuditSink, AuditSinkError,
    AuditWriteClass, AuditWriteOutcome, RouteAdmissionFields,
};

/// Immutable, redacted context carried from one route decision to its audit
/// append.
#[derive(Debug, Clone)]
pub struct RouteAuditContext {
    ts_ms: u64,
    zone: String,
    operation_id: String,
    correlation_id: String,
    source: String,
    previous_hash: AuditHash,
    subject_digest: String,
    authz_decision: String,
    authz_revision: u64,
    trace: Option<TraceContext>,
}

impl RouteAuditContext {
    /// Construct route context from opaque operation and subject values.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ts_ms: u64,
        zone: impl Into<String>,
        operation_id: impl Into<String>,
        correlation_id: impl Into<String>,
        source: impl Into<String>,
        previous_hash: AuditHash,
        subject_digest: impl Into<String>,
        authz_decision: impl Into<String>,
        authz_revision: u64,
        trace: Option<TraceContext>,
    ) -> Self {
        Self {
            ts_ms,
            zone: zone.into(),
            operation_id: operation_id.into(),
            correlation_id: correlation_id.into(),
            source: source.into(),
            previous_hash,
            subject_digest: subject_digest.into(),
            authz_decision: authz_decision.into(),
            authz_revision,
            trace,
        }
    }
}

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

    /// Build and append the route decision at the router callsite.
    pub fn record_resolution(
        &self,
        context: &RouteAuditContext,
        service: &str,
        method: &str,
        direction: RouteDirection,
        resolution: Result<(), RouteTableError>,
    ) -> Result<AuditWriteOutcome, RouteAuditError> {
        let (authz_decision, outcome) = match resolution {
            Ok(()) => (context.authz_decision.as_str(), "ok"),
            Err(RouteTableError::ServiceNotFound | RouteTableError::MethodNotFound) => {
                ("denied", "denied")
            }
            Err(_) => ("denied", "error"),
        };
        let record = route_admission_record_with_trace(
            context.ts_ms,
            context.zone.clone(),
            context.operation_id.clone(),
            context.correlation_id.clone(),
            context.source.clone(),
            context.previous_hash.clone(),
            service.to_owned(),
            method.to_owned(),
            direction.as_str(),
            context.subject_digest.clone(),
            authz_decision,
            context.authz_revision,
            outcome,
            context.trace.as_ref(),
        )
        .map_err(RouteAuditError::Record)?;
        self.append_route(&record).map_err(RouteAuditError::Sink)
    }
}

/// Failure while constructing or appending a route-admission record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteAuditError {
    /// The route record did not satisfy its closed field contract.
    Record(AuditRecordError),
    /// The standard audit sink failed.
    Sink(AuditSinkError),
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

    #[test]
    fn route_context_keeps_lifecycle_records_bounded() {
        let context = RouteAuditContext::new(
            1,
            "work",
            "operation",
            "correlation",
            "bus",
            genesis_hash(),
            "sha256:subject",
            "allowed",
            1,
            None,
        );
        let record = route_admission_record(
            context.ts_ms,
            context.zone,
            context.operation_id,
            context.correlation_id,
            context.source,
            context.previous_hash,
            "d2b.resource.v3",
            "Get",
            RouteDirection::Local.as_str(),
            context.subject_digest,
            context.authz_decision,
            context.authz_revision,
            "ok",
        )
        .unwrap();
        assert_eq!(record.class().as_str(), "route-admission");
    }
}
