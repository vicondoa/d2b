//! Restart reconciliation for the two independent audit durability domains.

use crate::operation::{OperationIdentity, ZoneId, ZoneOperationKey};

/// Terminal outcome recorded by one durability domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurabilityOutcome {
    /// The operation and its effect are durably successful.
    Success,
    /// The operation was durably refused or failed.
    Failure,
    /// The domain has a prepared record but no terminal outcome.
    Pending,
}

/// Minimal evidence from one audit durability domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurabilityEvidence {
    /// Shared Zone-scoped operation key.
    pub key: ZoneOperationKey,
    /// Domain-local terminal outcome.
    pub outcome: DurabilityOutcome,
    /// Whether the effect itself is known durable.
    pub effect_durable: bool,
}

/// Closed broker decision/result parsing failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceError {
    /// The decision/result pair is missing or not one of the closed pairs.
    DecisionResultInvalid,
    /// The supplied join key does not match the canonical Zone and operation.
    KeyMismatch,
}

impl core::fmt::Display for EvidenceError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::DecisionResultInvalid => "audit-evidence-decision-result-invalid",
            Self::KeyMismatch => "audit-evidence-zone-operation-key-mismatch",
        })
    }
}

impl std::error::Error for EvidenceError {}

/// Parse one authoritative broker decision into typed durability evidence.
///
/// Only the closed terminal pairs are valid. Legacy `denied/denied` and
/// `error/error` remain readable, while current broker decisions distinguish
/// the refused, policy, unknown, and errored classes. A caller-supplied key
/// is accepted only when it exactly equals the canonical key built from the
/// explicit Zone and operation identity.
pub fn evidence_from_decision_result(
    zone: ZoneId,
    operation: OperationIdentity,
    supplied_key: Option<&ZoneOperationKey>,
    decision: Option<&str>,
    result: Option<&str>,
) -> Result<DurabilityEvidence, EvidenceError> {
    let key = ZoneOperationKey::new(zone, operation);
    if supplied_key.is_none_or(|supplied| supplied != &key) {
        return Err(EvidenceError::KeyMismatch);
    }
    let outcome = match (decision, result) {
        (Some("allowed"), Some("success")) => DurabilityOutcome::Success,
        (
            Some("denied" | "denied-refused" | "denied-policy" | "denied-unknown"),
            Some("denied"),
        ) => DurabilityOutcome::Failure,
        (Some("error" | "errored"), Some("error" | "errored")) => DurabilityOutcome::Failure,
        _ => return Err(EvidenceError::DecisionResultInvalid),
    };
    Ok(DurabilityEvidence {
        key,
        outcome,
        effect_durable: outcome == DurabilityOutcome::Success,
    })
}

/// Restart reconciliation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconciliation {
    /// Both domains agree on a durable success.
    Success,
    /// Both domains agree on a durable failure.
    Failure,
    /// One or both domains still need replay.
    ReplayRequired,
    /// The domains disagree and the operation must fail closed.
    IntegrityFailure,
}

/// Reconcile broker and resource-store evidence by operation identity.
pub fn reconcile(
    broker: Option<&DurabilityEvidence>,
    resource: Option<&DurabilityEvidence>,
) -> Reconciliation {
    match (broker, resource) {
        (None, None) => Reconciliation::ReplayRequired,
        (Some(evidence), None) | (None, Some(evidence)) => {
            if evidence.outcome == DurabilityOutcome::Pending {
                Reconciliation::ReplayRequired
            } else {
                Reconciliation::IntegrityFailure
            }
        }
        (Some(broker), Some(resource)) => {
            let invalid = broker.key != resource.key
                || broker.outcome != resource.outcome
                || (broker.outcome == DurabilityOutcome::Success
                    && (!broker.effect_durable || !resource.effect_durable))
                || (broker.outcome == DurabilityOutcome::Failure
                    && (broker.effect_durable || resource.effect_durable));
            if invalid {
                Reconciliation::IntegrityFailure
            } else {
                match (broker.outcome, broker.effect_durable) {
                    (DurabilityOutcome::Success, true) => Reconciliation::Success,
                    (DurabilityOutcome::Failure, false) => Reconciliation::Failure,
                    _ => Reconciliation::ReplayRequired,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(
        token: &str,
        outcome: DurabilityOutcome,
        effect_durable: bool,
    ) -> DurabilityEvidence {
        DurabilityEvidence {
            key: ZoneOperationKey::derive("work", token).unwrap(),
            outcome,
            effect_durable,
        }
    }

    #[test]
    fn matching_success_requires_both_durable_domains() {
        let broker = evidence("operation", DurabilityOutcome::Success, true);
        let resource = evidence("operation", DurabilityOutcome::Success, true);
        assert_eq!(
            reconcile(Some(&broker), Some(&resource)),
            Reconciliation::Success
        );
        let resource = evidence("operation", DurabilityOutcome::Success, false);
        assert_eq!(
            reconcile(Some(&broker), Some(&resource)),
            Reconciliation::IntegrityFailure
        );
    }

    #[test]
    fn missing_or_mismatched_domains_fail_closed() {
        let broker = evidence("operation", DurabilityOutcome::Success, true);
        assert_eq!(
            reconcile(Some(&broker), None),
            Reconciliation::IntegrityFailure
        );
        let resource = DurabilityEvidence {
            key: ZoneOperationKey::derive("personal", "operation").unwrap(),
            outcome: DurabilityOutcome::Success,
            effect_durable: true,
        };
        assert_eq!(
            reconcile(Some(&broker), Some(&resource)),
            Reconciliation::IntegrityFailure
        );
    }

    #[test]
    fn impossible_success_is_integrity_failure_and_one_sided_terminal_is_not_replayed() {
        let impossible = evidence("operation", DurabilityOutcome::Success, false);
        assert_eq!(
            reconcile(Some(&impossible), Some(&impossible)),
            Reconciliation::IntegrityFailure
        );
        let failed = evidence("operation", DurabilityOutcome::Failure, false);
        assert_eq!(
            reconcile(Some(&failed), None),
            Reconciliation::IntegrityFailure
        );
    }

    #[test]
    fn evidence_requires_a_closed_pair_and_matching_supplied_key() {
        let zone = ZoneId::derive("work").unwrap();
        let operation = OperationIdentity::derive("operation").unwrap();
        let key = ZoneOperationKey::new(zone.clone(), operation.clone());
        assert!(
            evidence_from_decision_result(
                zone.clone(),
                operation.clone(),
                Some(&key),
                Some("allowed"),
                Some("denied"),
            )
            .is_err()
        );
        assert!(
            evidence_from_decision_result(
                zone.clone(),
                operation.clone(),
                None,
                Some("allowed"),
                Some("success"),
            )
            .is_err()
        );
        let wrong = ZoneOperationKey::derive("personal", "operation").unwrap();
        assert!(
            evidence_from_decision_result(
                zone,
                operation,
                Some(&wrong),
                Some("allowed"),
                Some("success"),
            )
            .is_err()
        );
    }
}
