//! Authorization audit projections.

use d2b_audit::{
    AuditHash, AuditRecord, AuditRecordError, AuditRecordFields, AuditSink, AuditSinkError,
    AuditWriteClass, AuditWriteOutcome, RbacChangeFields,
};
use sha2::{Digest, Sha256};

/// Derive a subject digest from a normalized canonical subject.
pub fn subject_digest(subject: &str) -> Result<String, AuditRecordError> {
    if subject.is_empty()
        || subject.bytes().any(|byte| byte.is_ascii_control())
        || subject.len() > 256
    {
        return Err(AuditRecordError::TextOutOfBounds);
    }
    let digest = Sha256::digest(subject.as_bytes());
    let mut output = String::from("sha256:");
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    Ok(output)
}

/// Build an RBACChange record without retaining the subject text.
#[allow(clippy::too_many_arguments)]
pub fn rbac_change_record(
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
    subject: &str,
    policy_revision: u64,
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
        AuditRecordFields::RbacChange(RbacChangeFields {
            verb: verb.into(),
            resource_type: resource_type.into(),
            resource_uid: resource_uid.into(),
            generation,
            subject_digest: subject_digest(subject)?,
            policy_revision,
            outcome: outcome.into(),
        }),
    )
}

/// Typed bridge for the privileged RBAC audit boundary.
pub struct AuthzAuditWriter<'a> {
    sink: &'a AuditSink,
}

impl<'a> AuthzAuditWriter<'a> {
    /// Borrow the shared authoritative audit sink.
    pub const fn new(sink: &'a AuditSink) -> Self {
        Self { sink }
    }

    /// Append one RBAC change with privileged durability.
    pub fn append_rbac_change(
        &self,
        record: &AuditRecord,
    ) -> Result<AuditWriteOutcome, AuditSinkError> {
        self.sink.append(AuditWriteClass::Privileged, record)
    }

    /// Build and append one RBAC change without retaining subject text.
    #[allow(clippy::too_many_arguments)]
    pub fn record_rbac_change(
        &self,
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
        subject: &str,
        policy_revision: u64,
        outcome: impl Into<String>,
    ) -> Result<AuditWriteOutcome, AuthzAuditError> {
        let record = rbac_change_record(
            ts_ms,
            zone,
            operation_id,
            correlation_id,
            source,
            previous_hash,
            verb,
            resource_type,
            resource_uid,
            generation,
            subject,
            policy_revision,
            outcome,
        )
        .map_err(AuthzAuditError::Record)?;
        self.append_rbac_change(&record)
            .map_err(AuthzAuditError::Sink)
    }
}

/// Failure from the RBAC record construction or durable append.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthzAuditError {
    /// The typed record was invalid.
    Record(AuditRecordError),
    /// The sink could not append or synchronize the record.
    Sink(AuditSinkError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_audit::genesis_hash;

    #[test]
    fn subject_digest_is_stable_and_not_plaintext() {
        let digest = subject_digest("Subject/alice").unwrap();
        assert!(digest.starts_with("sha256:"));
        assert!(!digest.contains("alice"));
        let record = rbac_change_record(
            1,
            "work",
            "op",
            "corr",
            "authz",
            genesis_hash(),
            "update-spec",
            "Role",
            "uid",
            1,
            "Subject/alice",
            2,
            "ok",
        )
        .unwrap();
        assert!(!serde_json::to_string(&record).unwrap().contains("alice"));
    }
}
