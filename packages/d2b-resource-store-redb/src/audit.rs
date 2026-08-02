//! Store-side durable mutation audit handoff.

use d2b_audit::{
    AuditHash, AuditRecord, AuditRecordError, AuditRecordFields, ResourceMutationFields,
};

/// Build a ResourceMutation record from commit metadata only.
#[allow(clippy::too_many_arguments)]
pub fn resource_mutation_record(
    ts_ms: u64,
    zone: impl Into<String>,
    operation_id: impl Into<String>,
    correlation_id: impl Into<String>,
    source: impl Into<String>,
    previous_hash: AuditHash,
    verb: impl Into<String>,
    resource_type: impl Into<String>,
    resource_uid: impl Into<String>,
    generation: u64,
    expected_revision: u64,
    resulting_revision: u64,
    subject_digest: impl Into<String>,
    policy_revision: u64,
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
        AuditRecordFields::ResourceMutation(ResourceMutationFields {
            verb: verb.into(),
            resource_type: resource_type.into(),
            resource_uid: resource_uid.into(),
            generation,
            expected_revision,
            resulting_revision,
            subject_digest: subject_digest.into(),
            policy_revision,
            outcome: outcome.into(),
            error_code,
        }),
    )
}

/// A durable-audit callback used by the store transaction boundary.
pub trait DurableMutationAudit: Send + Sync {
    /// Append and synchronize a privileged mutation record.
    fn append_before_commit(&self, record: &AuditRecord) -> Result<(), AuditRecordError>;
}

/// A no-op implementation for isolated store tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopMutationAudit;

impl DurableMutationAudit for NoopMutationAudit {
    fn append_before_commit(&self, _record: &AuditRecord) -> Result<(), AuditRecordError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_audit::genesis_hash;

    #[test]
    fn mutation_record_has_no_payload_bytes() {
        let record = resource_mutation_record(
            1,
            "work",
            "op",
            "corr",
            "store",
            genesis_hash(),
            "update-spec",
            "Provider",
            "uid",
            1,
            2,
            3,
            "sha256:subject",
            4,
            "ok",
            None,
        )
        .unwrap();
        let json = serde_json::to_string(&record).unwrap();
        assert!(!json.contains("\"spec\""));
        assert!(!json.contains("\"realm\""));
    }
}
