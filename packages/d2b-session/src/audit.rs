//! ComponentSession audit projections.

use d2b_audit::{
    AuditHash, AuditRecord, AuditRecordError, AuditRecordFields, AuditSink, AuditSinkError,
    AuditWriteClass, AuditWriteOutcome, ProcessEffectFields, SessionConnectFields,
};
use d2b_telemetry::TraceContext;

/// Construct a `SessionConnect` record after handshake completion.
#[allow(clippy::too_many_arguments)]
pub fn session_connect_record(
    ts_ms: u64,
    zone: impl Into<String>,
    operation_id: impl Into<String>,
    correlation_id: impl Into<String>,
    source: impl Into<String>,
    previous_hash: AuditHash,
    event: impl Into<String>,
    profile: impl Into<String>,
    purpose_class: impl Into<String>,
    transport_class: impl Into<String>,
    subject_digest: impl Into<String>,
    authz_decision: impl Into<String>,
    authz_revision: u64,
    session_gen_digest: impl Into<String>,
    outcome: impl Into<String>,
    error_code: Option<String>,
) -> Result<AuditRecord, AuditRecordError> {
    session_connect_record_with_trace(
        ts_ms,
        zone,
        operation_id,
        correlation_id,
        source,
        previous_hash,
        event,
        profile,
        purpose_class,
        transport_class,
        subject_digest,
        authz_decision,
        authz_revision,
        session_gen_digest,
        outcome,
        error_code,
        None,
    )
}

/// Construct a `SessionConnect` record and carry the validated trace id.
#[allow(clippy::too_many_arguments)]
pub fn session_connect_record_with_trace(
    ts_ms: u64,
    zone: impl Into<String>,
    operation_id: impl Into<String>,
    correlation_id: impl Into<String>,
    source: impl Into<String>,
    previous_hash: AuditHash,
    event: impl Into<String>,
    profile: impl Into<String>,
    purpose_class: impl Into<String>,
    transport_class: impl Into<String>,
    subject_digest: impl Into<String>,
    authz_decision: impl Into<String>,
    authz_revision: u64,
    session_gen_digest: impl Into<String>,
    outcome: impl Into<String>,
    error_code: Option<String>,
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
        AuditRecordFields::SessionConnect(SessionConnectFields {
            event: event.into(),
            profile: profile.into(),
            purpose_class: purpose_class.into(),
            transport_class: transport_class.into(),
            subject_digest: subject_digest.into(),
            authz_decision: authz_decision.into(),
            authz_revision,
            session_gen_digest: session_gen_digest.into(),
            outcome: outcome.into(),
            error_code,
        }),
    )
}

/// A typed bridge from session lifecycle decisions to the durable audit sink.
pub struct SessionAuditWriter<'a> {
    sink: &'a AuditSink,
}

impl<'a> SessionAuditWriter<'a> {
    /// Borrow a session audit sink.
    pub const fn new(sink: &'a AuditSink) -> Self {
        Self { sink }
    }

    /// Append a completed session connection using its required durability.
    pub fn append_connect(
        &self,
        record: &AuditRecord,
    ) -> Result<AuditWriteOutcome, AuditSinkError> {
        self.sink.append(connect_write_class(record), record)
    }

    /// Append an informational process effect for a running session.
    pub fn append_process_effect(
        &self,
        record: &AuditRecord,
    ) -> Result<AuditWriteOutcome, AuditSinkError> {
        self.sink.append(AuditWriteClass::Standard, record)
    }
}

fn connect_write_class(record: &AuditRecord) -> AuditWriteClass {
    match record.fields() {
        AuditRecordFields::SessionConnect(fields)
            if fields.authz_decision == "denied"
                || matches!(fields.outcome.as_str(), "auth" | "policy") =>
        {
            AuditWriteClass::Privileged
        }
        _ => AuditWriteClass::Standard,
    }
}

/// Construct the informational ProcessEffect emitted for a running session.
pub fn session_process_effect(
    ts_ms: u64,
    zone: impl Into<String>,
    previous_hash: AuditHash,
    source: impl Into<String>,
) -> Result<AuditRecord, AuditRecordError> {
    session_process_effect_with_trace(ts_ms, zone, previous_hash, source, None)
}

/// Construct the informational ProcessEffect with a propagated trace id.
pub fn session_process_effect_with_trace(
    ts_ms: u64,
    zone: impl Into<String>,
    previous_hash: AuditHash,
    source: impl Into<String>,
    trace: Option<&TraceContext>,
) -> Result<AuditRecord, AuditRecordError> {
    AuditRecord::new(
        ts_ms,
        zone,
        "operation-digest",
        "correlation-digest",
        trace.map(|trace| trace.trace_id().to_owned()),
        source,
        previous_hash,
        AuditRecordFields::ProcessEffect(ProcessEffectFields {
            event: "adopt".to_owned(),
            provider: "systemd".to_owned(),
            domain: "system".to_owned(),
            no_isolation: false,
            execution_ref_digest: "sha256:session".to_owned(),
            process_uid: "uid-session".to_owned(),
            outcome: "ok".to_owned(),
            exit_class: None,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_audit::genesis_hash;

    #[test]
    fn handshake_record_uses_zone_link_without_legacy_fields() {
        let record = session_connect_record(
            1,
            "work",
            "operation",
            "correlation",
            "session",
            genesis_hash(),
            "connect",
            "NN",
            "local",
            "zone_link",
            "sha256:subject",
            "allowed",
            1,
            "sha256:generation",
            "ok",
            None,
        )
        .unwrap();
        let value = serde_json::to_value(record).unwrap();
        assert!(value.get("zone").is_some());
        assert!(value.get("realm").is_none());
        assert!(value.get("node").is_none());
    }

    #[test]
    fn traced_handshake_carries_only_the_opaque_trace_id() {
        let trace = TraceContext::new("trace-id", "span-id").unwrap();
        let record = session_connect_record_with_trace(
            1,
            "work",
            "operation",
            "correlation",
            "session",
            genesis_hash(),
            "connect",
            "NN",
            "local",
            "zone_link",
            "sha256:subject",
            "allowed",
            1,
            "sha256:generation",
            "ok",
            None,
            Some(&trace),
        )
        .unwrap();
        assert_eq!(record.trace_id(), Some("trace-id"));
        assert!(!serde_json::to_string(&record).unwrap().contains("span-id"));
    }
}
