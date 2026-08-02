//! ComponentSession audit projections.

use d2b_audit::{
    AuditHash, AuditRecord, AuditRecordError, AuditRecordFields, ProcessEffectFields,
    SessionConnectFields,
};

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
    AuditRecord::new(
        ts_ms,
        zone,
        operation_id,
        correlation_id,
        None,
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

/// Construct the informational ProcessEffect emitted for a running session.
pub fn session_process_effect(
    ts_ms: u64,
    zone: impl Into<String>,
    previous_hash: AuditHash,
    source: impl Into<String>,
) -> Result<AuditRecord, AuditRecordError> {
    AuditRecord::new(
        ts_ms,
        zone,
        "operation-digest",
        "correlation-digest",
        None,
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
}
