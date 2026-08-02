//! Host process-effect audit construction.

use d2b_audit::{AuditHash, AuditRecord, AuditRecordError, AuditRecordFields, ProcessEffectFields};

/// Build a ProcessEffect for a Host child process.
pub fn process_effect_record(
    ts_ms: u64,
    zone: impl Into<String>,
    previous_hash: AuditHash,
    event: impl Into<String>,
    domain: impl Into<String>,
    user_only_host: bool,
    process_uid: impl Into<String>,
    outcome: impl Into<String>,
) -> Result<AuditRecord, AuditRecordError> {
    let domain = domain.into();
    let provider = if user_only_host && domain == "user" {
        "system-core-user"
    } else {
        "systemd"
    };
    AuditRecord::new(
        ts_ms,
        zone,
        "operation-digest",
        "correlation-digest",
        None,
        "system-core",
        previous_hash,
        AuditRecordFields::ProcessEffect(ProcessEffectFields {
            event: event.into(),
            provider: provider.to_owned(),
            no_isolation: user_only_host && domain == "user",
            execution_ref_digest: "sha256:execution".to_owned(),
            process_uid: process_uid.into(),
            outcome: outcome.into(),
            exit_class: None,
            domain,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_audit::genesis_hash;

    #[test]
    fn only_user_only_hosts_get_the_no_isolation_flag() {
        let user = process_effect_record(
            1,
            "work",
            genesis_hash(),
            "launch",
            "user",
            true,
            "uid",
            "ok",
        )
        .unwrap();
        let system = process_effect_record(
            1,
            "work",
            user.record_hash().clone(),
            "launch",
            "system",
            false,
            "uid",
            "ok",
        )
        .unwrap();
        assert!(
            serde_json::to_string(&user)
                .unwrap()
                .contains("\"no_isolation\":true")
        );
        assert!(
            serde_json::to_string(&system)
                .unwrap()
                .contains("\"no_isolation\":false")
        );
    }
}
