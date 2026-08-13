//! Store-side durable mutation audit handoff.

use std::sync::Arc;

use d2b_audit::{
    AuditHash, AuditRecord, AuditRecordError, AuditRecordFields, AuditSink, AuditSinkError,
    AuditWriteClass, AuditWriteOutcome, DurabilityEvidence, DurabilityOutcome, OperationIdentity,
    Reconciliation, ResourceMutationFields, genesis_hash, reconcile_durability,
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
    resource_mutation_record_with_identity(
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
        expected_revision,
        resulting_revision,
        subject_digest,
        policy_revision,
        outcome,
        error_code,
        None,
        None,
    )
}

/// Build a ResourceMutation record with deterministic replay identity.
#[allow(clippy::too_many_arguments)]
pub fn resource_mutation_record_with_identity(
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
    mutation_id: Option<String>,
    mutation_ordinal: Option<u32>,
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
            mutation_id,
            mutation_ordinal,
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

/// Reconcile a resource outbox operation with broker evidence.
///
/// The two physical durability domains remain independent. This helper only
/// joins their typed operation identities and refuses mismatched outcomes.
pub fn reconcile_broker_evidence(
    resource_operation_id: &str,
    resource_outcome: DurabilityOutcome,
    resource_effect_durable: bool,
    broker: &DurabilityEvidence,
) -> Result<Reconciliation, AuditReconciliationError> {
    let resource_operation = OperationIdentity::derive(resource_operation_id)
        .map_err(|_| AuditReconciliationError::InvalidOperationIdentity)?;
    let resource = DurabilityEvidence {
        key: d2b_audit::ZoneOperationKey::new(broker.key.zone().clone(), resource_operation),
        outcome: resource_outcome,
        effect_durable: resource_effect_durable,
    };
    let result = reconcile_durability(Some(broker), Some(&resource));
    if result == Reconciliation::IntegrityFailure {
        return Err(AuditReconciliationError::IntegrityFailure);
    }
    Ok(result)
}

/// Fail-closed resource/broker reconciliation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditReconciliationError {
    /// The operation token could not be converted to a bounded identity.
    InvalidOperationIdentity,
    /// The domains disagree on identity or outcome.
    IntegrityFailure,
}

impl core::fmt::Display for AuditReconciliationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidOperationIdentity => "audit-operation-identity-invalid",
            Self::IntegrityFailure => "audit-domain-integrity-failure",
        })
    }
}

impl std::error::Error for AuditReconciliationError {}

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

    /// Look up a mutation that may already have reached the sink before a
    /// crash interrupted outbox acknowledgement.
    fn existing_mutation_hash(
        &self,
        _key: &d2b_audit::ZoneOperationKey,
        _mutation_id: &str,
    ) -> Result<Option<AuditHash>, AuditRecordError>;

    /// Return the predecessor hash for an already durable mutation.
    fn existing_mutation_predecessor(
        &self,
        _key: &d2b_audit::ZoneOperationKey,
        _mutation_id: &str,
    ) -> Result<Option<AuditHash>, AuditRecordError>;
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

    fn existing_mutation_hash(
        &self,
        _key: &d2b_audit::ZoneOperationKey,
        _mutation_id: &str,
    ) -> Result<Option<AuditHash>, AuditRecordError> {
        Ok(None)
    }

    fn existing_mutation_predecessor(
        &self,
        _key: &d2b_audit::ZoneOperationKey,
        _mutation_id: &str,
    ) -> Result<Option<AuditHash>, AuditRecordError> {
        Ok(None)
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

    fn existing_mutation_hash(
        &self,
        key: &d2b_audit::ZoneOperationKey,
        mutation_id: &str,
    ) -> Result<Option<AuditHash>, AuditRecordError> {
        self.sink
            .mutation_record_hash(key, mutation_id)
            .map_err(Self::map_sink_error)
    }

    fn existing_mutation_predecessor(
        &self,
        key: &d2b_audit::ZoneOperationKey,
        mutation_id: &str,
    ) -> Result<Option<AuditHash>, AuditRecordError> {
        self.sink
            .mutation_record_predecessor(key, mutation_id)
            .map_err(Self::map_sink_error)
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
            "sha256:0000000000000000000000000000000000000000000000000000000000000001",
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

    #[test]
    fn broker_and_outbox_must_share_identity_and_outcome() {
        let broker = DurabilityEvidence {
            key: d2b_audit::ZoneOperationKey::derive("work", "operation").unwrap(),
            outcome: DurabilityOutcome::Success,
            effect_durable: true,
        };
        assert_eq!(
            reconcile_broker_evidence("operation", DurabilityOutcome::Success, true, &broker)
                .unwrap(),
            Reconciliation::Success
        );
        assert_eq!(
            reconcile_broker_evidence("operation", DurabilityOutcome::Failure, false, &broker),
            Err(AuditReconciliationError::IntegrityFailure)
        );
        assert_eq!(
            reconcile_broker_evidence(
                "different-operation",
                DurabilityOutcome::Success,
                true,
                &broker
            ),
            Err(AuditReconciliationError::IntegrityFailure)
        );
    }

    #[test]
    fn production_restart_join_key_keeps_same_token_in_separate_zones() {
        let broker_work = DurabilityEvidence {
            key: d2b_audit::ZoneOperationKey::derive("work", "shared-token").unwrap(),
            outcome: DurabilityOutcome::Success,
            effect_durable: true,
        };
        let personal = DurabilityEvidence {
            key: d2b_audit::ZoneOperationKey::derive("personal", "shared-token").unwrap(),
            outcome: DurabilityOutcome::Success,
            effect_durable: true,
        };
        assert_eq!(
            reconcile_durability(Some(&broker_work), Some(&personal)),
            Reconciliation::IntegrityFailure
        );
        assert_eq!(
            reconcile_durability(Some(&broker_work), None),
            Reconciliation::IntegrityFailure
        );
    }
}
