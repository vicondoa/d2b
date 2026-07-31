//! Store lifecycle admission and operation coordination policy.
//!
//! This module does not implement a store. It produces typed requests for the
//! independently owned store actor and tracks only bounded process-local
//! coordination state.

use std::collections::BTreeMap;

/// Store lifecycle operation class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StoreOperationKind {
    Backup,
    Restore,
    SchemaUpgrade,
    Compaction,
    CorruptionQuarantine,
    ResetInventory,
}

impl StoreOperationKind {
    /// Return the fixed metric label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Backup => "backup",
            Self::Restore => "restore",
            Self::SchemaUpgrade => "schema-upgrade",
            Self::Compaction => "compaction",
            Self::CorruptionQuarantine => "corruption-quarantine",
            Self::ResetInventory => "reset-inventory",
        }
    }

    const fn destructive(self) -> bool {
        matches!(
            self,
            Self::Restore | Self::SchemaUpgrade | Self::ResetInventory
        )
    }
}

/// Cardinality-safe label keys for store lifecycle metrics.
pub const STORE_METRIC_LABEL_KEYS: &[&str] = &["operation", "outcome"];

/// Trusted store observations required for lifecycle admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreObservation {
    pub healthy: bool,
    pub quarantined: bool,
    pub active_writers: u32,
    pub active_watchers: u32,
    pub staged_restore_valid: bool,
    pub backup_admitted: bool,
}

/// Typed request for the independently owned store actor.
#[derive(Clone, PartialEq, Eq)]
pub struct StoreOperationRequest {
    kind: StoreOperationKind,
    idempotency_digest: [u8; 32],
}

impl StoreOperationRequest {
    /// Return the operation class.
    pub const fn kind(&self) -> StoreOperationKind {
        self.kind
    }
}

impl core::fmt::Debug for StoreOperationRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("StoreOperationRequest")
            .field("kind", &self.kind)
            .field("has_idempotency_digest", &true)
            .finish()
    }
}

/// Store operation completion class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOperationOutcome {
    Succeeded,
    Degraded,
    Failed,
    Unknown,
}

/// Closed store lifecycle refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreHandlerError {
    InvalidIdempotency,
    DuplicateInFlight,
    StoreUnhealthy,
    StoreQuarantined,
    ActiveUseBlocksOperation,
    RestoreNotStaged,
    BackupNotAdmitted,
    UnknownOperation,
}

impl StoreHandlerError {
    /// Return a stable, identity-free reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidIdempotency => "store-operation-idempotency-invalid",
            Self::DuplicateInFlight => "store-operation-in-flight",
            Self::StoreUnhealthy => "store-unhealthy",
            Self::StoreQuarantined => "store-quarantined",
            Self::ActiveUseBlocksOperation => "store-operation-active-use",
            Self::RestoreNotStaged => "store-restore-not-staged",
            Self::BackupNotAdmitted => "store-backup-not-admitted",
            Self::UnknownOperation => "store-operation-unknown",
        }
    }
}

impl core::fmt::Display for StoreHandlerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for StoreHandlerError {}

/// Pure store lifecycle coordinator.
#[derive(Default)]
pub struct StoreHandler {
    in_flight: BTreeMap<[u8; 32], StoreOperationKind>,
    last_outcome: Option<(StoreOperationKind, StoreOperationOutcome)>,
}

impl core::fmt::Debug for StoreHandler {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("StoreHandler")
            .field("in_flight_count", &self.in_flight.len())
            .field("last_outcome", &self.last_outcome)
            .finish()
    }
}

impl StoreHandler {
    /// Admit one lifecycle request against trusted actor observations.
    pub fn admit(
        &mut self,
        kind: StoreOperationKind,
        idempotency_digest: [u8; 32],
        observation: StoreObservation,
    ) -> Result<StoreOperationRequest, StoreHandlerError> {
        if idempotency_digest == [0; 32] {
            return Err(StoreHandlerError::InvalidIdempotency);
        }
        if self.in_flight.contains_key(&idempotency_digest) || !self.in_flight.is_empty() {
            return Err(StoreHandlerError::DuplicateInFlight);
        }
        if observation.quarantined && kind != StoreOperationKind::ResetInventory {
            return Err(StoreHandlerError::StoreQuarantined);
        }
        if !observation.healthy
            && !matches!(
                kind,
                StoreOperationKind::CorruptionQuarantine | StoreOperationKind::ResetInventory
            )
        {
            return Err(StoreHandlerError::StoreUnhealthy);
        }
        if kind.destructive() && (observation.active_writers > 0 || observation.active_watchers > 0)
        {
            return Err(StoreHandlerError::ActiveUseBlocksOperation);
        }
        if kind == StoreOperationKind::Restore && !observation.staged_restore_valid {
            return Err(StoreHandlerError::RestoreNotStaged);
        }
        if kind == StoreOperationKind::Backup && !observation.backup_admitted {
            return Err(StoreHandlerError::BackupNotAdmitted);
        }
        self.in_flight.insert(idempotency_digest, kind);
        Ok(StoreOperationRequest {
            kind,
            idempotency_digest,
        })
    }

    /// Record an actor result and release the process-local operation slot.
    pub fn complete(
        &mut self,
        request: StoreOperationRequest,
        outcome: StoreOperationOutcome,
    ) -> Result<(), StoreHandlerError> {
        if self.in_flight.remove(&request.idempotency_digest) != Some(request.kind) {
            return Err(StoreHandlerError::UnknownOperation);
        }
        self.last_outcome = Some((request.kind, outcome));
        Ok(())
    }

    /// On restart, preserve ambiguity instead of assuming an operation failed
    /// or succeeded. Durable recovery belongs to the store actor's operation
    /// ledger.
    pub fn restart(&mut self) {
        if let Some((_, kind)) = self.in_flight.first_key_value() {
            self.last_outcome = Some((*kind, StoreOperationOutcome::Unknown));
        }
        self.in_flight.clear();
    }

    /// Return the latest bounded outcome.
    pub const fn last_outcome(&self) -> Option<(StoreOperationKind, StoreOperationOutcome)> {
        self.last_outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> StoreObservation {
        StoreObservation {
            healthy: true,
            quarantined: false,
            active_writers: 0,
            active_watchers: 0,
            staged_restore_valid: true,
            backup_admitted: true,
        }
    }

    #[test]
    fn admitted_request_is_typed_and_completes_once() {
        let mut handler = StoreHandler::default();
        let request = handler
            .admit(StoreOperationKind::Backup, [1; 32], observation())
            .unwrap();
        assert_eq!(request.kind(), StoreOperationKind::Backup);
        handler
            .complete(request, StoreOperationOutcome::Succeeded)
            .unwrap();
        assert_eq!(
            handler.last_outcome(),
            Some((StoreOperationKind::Backup, StoreOperationOutcome::Succeeded))
        );
    }

    #[test]
    fn destructive_operation_is_refused_during_active_use() {
        let mut handler = StoreHandler::default();
        let mut observed = observation();
        observed.active_watchers = 1;
        assert_eq!(
            handler
                .admit(StoreOperationKind::Restore, [1; 32], observed)
                .unwrap_err(),
            StoreHandlerError::ActiveUseBlocksOperation
        );
    }

    #[test]
    fn quarantine_refuses_normal_operations_but_allows_reset_inventory() {
        let mut handler = StoreHandler::default();
        let mut observed = observation();
        observed.quarantined = true;
        assert_eq!(
            handler
                .admit(StoreOperationKind::Compaction, [1; 32], observed)
                .unwrap_err(),
            StoreHandlerError::StoreQuarantined
        );
        assert!(
            handler
                .admit(StoreOperationKind::ResetInventory, [2; 32], observed)
                .is_ok()
        );
    }

    #[test]
    fn restart_preserves_unknown_for_an_in_flight_operation() {
        let mut handler = StoreHandler::default();
        handler
            .admit(StoreOperationKind::SchemaUpgrade, [1; 32], observation())
            .unwrap();
        handler.restart();
        assert_eq!(
            handler.last_outcome(),
            Some((
                StoreOperationKind::SchemaUpgrade,
                StoreOperationOutcome::Unknown
            ))
        );
    }

    #[test]
    fn request_debug_redacts_idempotency_material() {
        let mut handler = StoreHandler::default();
        let request = handler
            .admit(StoreOperationKind::Backup, [197; 32], observation())
            .unwrap();
        let debug = format!("{request:?}");
        assert!(!debug.contains("197"));
        assert_eq!(STORE_METRIC_LABEL_KEYS, &["operation", "outcome"]);
    }
}
