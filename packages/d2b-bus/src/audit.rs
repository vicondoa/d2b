//! Route-admission audit records.

use d2b_audit::{
    AuditHash, AuditRecord, AuditRecordError, AuditRecordFields, RouteAdmissionFields,
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
    AuditRecord::new(
        ts_ms,
        zone,
        operation_id,
        correlation_id,
        None,
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
}
