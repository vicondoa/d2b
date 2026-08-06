//! Store-side durable mutation audit handoff.

use std::sync::Arc;

use d2b_audit::{
    AuditHash, AuditRecord, AuditRecordError, AuditRecordFields, AuditSink, AuditSinkError,
    AuditWriteClass, AuditWriteOutcome, ResourceMutationFields, genesis_hash,
};
use sha2::{Digest, Sha256};

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

/// Hash a subject, resource reference, or operation token before it reaches
/// the audit envelope.
pub fn opaque_digest(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::from("sha256:");
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

/// A durable-audit callback used by the store transaction boundary.
pub trait DurableMutationAudit: Send + Sync {
    /// Whether this port owns a durable sink rather than an intentional no-op.
    fn enabled(&self) -> bool {
        true
    }

    /// Return the predecessor hash for the next record.
    fn previous_hash(&self) -> Result<AuditHash, AuditRecordError> {
        Ok(genesis_hash())
    }

    /// Append and synchronize a privileged mutation record.
    fn append_before_commit(&self, record: &AuditRecord) -> Result<(), AuditRecordError>;
}

/// A no-op implementation for isolated store tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopMutationAudit;

impl DurableMutationAudit for NoopMutationAudit {
    fn enabled(&self) -> bool {
        false
    }

    fn append_before_commit(&self, _record: &AuditRecord) -> Result<(), AuditRecordError> {
        Ok(())
    }
}

/// Adapter from the store's pre-commit port to the synchronized audit sink.
#[derive(Clone)]
pub struct AuditSinkMutationAudit {
    sink: Arc<AuditSink>,
}

impl core::fmt::Debug for AuditSinkMutationAudit {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuditSinkMutationAudit(<redacted>)")
    }
}

impl AuditSinkMutationAudit {
    /// Construct a store audit adapter around a shared sink.
    pub fn new(sink: Arc<AuditSink>) -> Self {
        Self { sink }
    }

    /// Borrow the shared sink.
    pub fn sink(&self) -> &Arc<AuditSink> {
        &self.sink
    }

    fn map_sink_error(_error: AuditSinkError) -> AuditRecordError {
        AuditRecordError::Serialization
    }
}

impl DurableMutationAudit for AuditSinkMutationAudit {
    fn previous_hash(&self) -> Result<AuditHash, AuditRecordError> {
        self.sink.chain_head().map_err(Self::map_sink_error)
    }

    fn append_before_commit(&self, record: &AuditRecord) -> Result<(), AuditRecordError> {
        match self
            .sink
            .append(AuditWriteClass::Privileged, record)
            .map_err(Self::map_sink_error)?
        {
            AuditWriteOutcome::Written => Ok(()),
            AuditWriteOutcome::RateLimited | AuditWriteOutcome::DroppedUnavailable => {
                Err(AuditRecordError::Serialization)
            }
        }
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

    #[test]
    fn opaque_digest_keeps_resource_identity_out_of_the_record() {
        let digest = opaque_digest("Host/secret-name");
        assert!(digest.starts_with("sha256:"));
        assert!(!digest.contains("secret-name"));
    }
}
