//! Zone-global authority operation ledger.
//!
//! The pre-v3 Zone store owned these rows durably. With the store deleted
//! (spec §26, R29) the ledger is process-local: it keeps the admission
//! barrier's operation lifecycle inside one daemon lifetime, and recovery
//! across a restart is the drivers' probe/adopt path rather than a replayed
//! controller checkpoint (R11, R16).
//!
//! Core owns the typed rows and recovery validation; this adapter owns the
//! bytes and the capability binding, exactly as the deleted redb owner did,
//! minus the durable substrate.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use sha2::{Digest, Sha256};

use d2b_contracts_resource::v3::ZoneId;
use d2b_core_controller::authority::{
    AuthorityOperationState, AuthorityStorageClaim, AuthorityStorageOperation, claim_digest,
};
use d2b_core_controller::authority_persistence::{
    AuthorityFuture, AuthorityOperationCapability, AuthorityPersistence, AuthorityPersistenceError,
    AuthorityRecoveryData, AuthorityRecoveryProvenance, PreparedAuthorityOperation,
};

/// One ledger row: the typed operation plus the payload and state the
/// admission barrier reads back by operation id.
pub struct LedgerRow {
    pub operation_id: String,
    pub state: AuthorityOperationState,
    pub claim_digest: String,
    pub store_binding_digest: String,
    pub payload: Vec<u8>,
}

impl core::fmt::Debug for LedgerRow {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("LedgerRow")
            .field("operation_id", &self.operation_id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

type Rows = Arc<Mutex<BTreeMap<String, LedgerRow>>>;

/// Process-local authority operation owner for one Zone.
pub struct ZoneAuthorityLedger {
    binding_digest: String,
    rows: Rows,
}

impl core::fmt::Debug for ZoneAuthorityLedger {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ZoneAuthorityLedger(<process-local>)")
    }
}

impl ZoneAuthorityLedger {
    /// Bind one ledger to its Zone's admission barrier.
    pub fn new(zone: &ZoneId) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"d2b-zone-authority-ledger/v1\x00");
        digest.update(zone.as_str().as_bytes());
        Self {
            binding_digest: format!("sha256:{:x}", digest.finalize()),
            rows: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Stable binding digest for one claim: identical inputs produce the same
    /// digest for this ledger's lifetime, so a resumed operation keeps its
    /// fence.
    pub fn authority_binding_digest(&self, claim_digest: &str) -> String {
        let mut digest = Sha256::new();
        digest.update(b"d2b-zone-authority-binding/v1\x00");
        digest.update(self.binding_digest.as_bytes());
        digest.update([0]);
        digest.update(claim_digest.as_bytes());
        format!("sha256:{:x}", digest.finalize())
    }

    /// Every operation the ledger currently holds, ordered by operation id.
    pub fn authority_operations(&self) -> Vec<LedgerRow> {
        let rows = self.rows.lock().expect("authority ledger lock");
        rows.values()
            .map(|row| LedgerRow {
                operation_id: row.operation_id.clone(),
                state: row.state,
                claim_digest: row.claim_digest.clone(),
                store_binding_digest: row.store_binding_digest.clone(),
                payload: row.payload.clone(),
            })
            .collect()
    }

    /// Prepare one operation, idempotently: an already prepared id with the
    /// same claim keeps its capability.
    pub fn prepare_authority_operation(
        &self,
        operation_id: String,
        payload: Vec<u8>,
        claim_digest: &str,
    ) -> Result<ZoneAuthorityCapability, AuthorityPersistenceError> {
        let store_binding_digest = self.authority_binding_digest(claim_digest);
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityPersistenceError::StateInvalid)?;
        match rows.get(&operation_id) {
            Some(row) if row.claim_digest != claim_digest => {
                return Err(AuthorityPersistenceError::RowInvalid);
            }
            Some(_) => {}
            None => {
                rows.insert(
                    operation_id.clone(),
                    LedgerRow {
                        operation_id: operation_id.clone(),
                        state: AuthorityOperationState::Pending,
                        claim_digest: claim_digest.to_owned(),
                        store_binding_digest: store_binding_digest.clone(),
                        payload,
                    },
                );
            }
        }
        Ok(ZoneAuthorityCapability {
            rows: Arc::clone(&self.rows),
            operation_id,
            store_binding_digest,
        })
    }

    /// Resume a prepared non-terminal operation.
    pub fn resume_authority_operation(
        &self,
        operation_id: String,
        binding_digest: &str,
    ) -> Result<ZoneAuthorityCapability, AuthorityPersistenceError> {
        let rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityPersistenceError::StateInvalid)?;
        let row = rows
            .get(&operation_id)
            .ok_or(AuthorityPersistenceError::RowInvalid)?;
        if row.store_binding_digest != binding_digest {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        Ok(ZoneAuthorityCapability {
            rows: Arc::clone(&self.rows),
            operation_id,
            store_binding_digest: binding_digest.to_owned(),
        })
    }

    fn record(
        &self,
        capability: &AuthorityOperationCapability,
        state: AuthorityOperationState,
    ) -> Result<(), AuthorityPersistenceError> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityPersistenceError::StateInvalid)?;
        let row = rows
            .get_mut(capability.operation_id())
            .ok_or(AuthorityPersistenceError::RowInvalid)?;
        if row.store_binding_digest != capability.store_binding_digest() {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        row.state = state;
        Ok(())
    }

    fn insert_operation(
        &self,
        operation_id: String,
        payload: Vec<u8>,
        claim_digest: &str,
        store_binding_digest: String,
    ) -> Result<(), AuthorityPersistenceError> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityPersistenceError::StateInvalid)?;
        rows.insert(
            operation_id.clone(),
            LedgerRow {
                operation_id,
                state: AuthorityOperationState::Pending,
                claim_digest: claim_digest.to_owned(),
                store_binding_digest,
                payload,
            },
        );
        Ok(())
    }
}

/// Capability bound to one ledger row: the process-local fence that permits a
/// state transition on exactly this operation.
pub struct ZoneAuthorityCapability {
    rows: Rows,
    operation_id: String,
    store_binding_digest: String,
}

impl core::fmt::Debug for ZoneAuthorityCapability {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ZoneAuthorityCapability(<ledger-bound>)")
    }
}

impl ZoneAuthorityCapability {
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub async fn record_effect(
        &self,
        state: AuthorityOperationState,
    ) -> Result<(), AuthorityPersistenceError> {
        self.record(state)
    }

    pub async fn record_close(&self) -> Result<(), AuthorityPersistenceError> {
        self.record(AuthorityOperationState::Closed)
    }

    pub async fn release(&self) -> Result<(), AuthorityPersistenceError> {
        self.record(AuthorityOperationState::Released)
    }

    fn record(&self, state: AuthorityOperationState) -> Result<(), AuthorityPersistenceError> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityPersistenceError::StateInvalid)?;
        let row = rows
            .get_mut(&self.operation_id)
            .ok_or(AuthorityPersistenceError::RowInvalid)?;
        if row.store_binding_digest != self.store_binding_digest {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        row.state = state;
        Ok(())
    }
}

impl AuthorityPersistence for ZoneAuthorityLedger {
    fn prepare<'a>(
        &'a self,
        operation_id: &'a str,
        claim: &'a AuthorityStorageClaim,
    ) -> AuthorityFuture<'a, PreparedAuthorityOperation> {
        Box::pin(async move {
            let claim_digest =
                claim_digest(claim).map_err(|_| AuthorityPersistenceError::RowInvalid)?;
            let owner_ref_present = match claim {
                AuthorityStorageClaim::Generic(claim) => {
                    claim.owner_proof().resource_ref().is_some()
                }
                AuthorityStorageClaim::ExternalNic(claim) => {
                    claim.owner_proof().resource_ref().is_some()
                }
            };
            if !owner_ref_present {
                return Err(AuthorityPersistenceError::RowInvalid);
            }
            let store_binding_digest = self.authority_binding_digest(&claim_digest);
            let payload = serde_json::to_vec(&AuthorityStorageOperation {
                operation_id: operation_id.to_owned(),
                claim: claim.clone(),
                state: AuthorityOperationState::Pending,
                claim_digest: claim_digest.clone(),
                store_binding_digest: store_binding_digest.clone(),
            })
            .map_err(|_| AuthorityPersistenceError::RowInvalid)?;
            if !self
                .authority_operations()
                .iter()
                .any(|row| row.operation_id == operation_id)
            {
                self.insert_operation(
                    operation_id.to_owned(),
                    payload,
                    &claim_digest,
                    store_binding_digest.clone(),
                )?;
            }
            PreparedAuthorityOperation::new(operation_id.to_owned(), store_binding_digest, 1)
        })
    }

    fn record_effect<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
        state: AuthorityOperationState,
    ) -> AuthorityFuture<'a, ()> {
        Box::pin(async move { self.record(capability, state) })
    }

    fn record_close<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
    ) -> AuthorityFuture<'a, ()> {
        Box::pin(async move { self.record(capability, AuthorityOperationState::Closed) })
    }

    fn release<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
    ) -> AuthorityFuture<'a, ()> {
        Box::pin(async move { self.record(capability, AuthorityOperationState::Released) })
    }

    fn recover<'a>(&'a self) -> AuthorityFuture<'a, AuthorityRecoveryData> {
        // Nothing survives the process: a restart re-derives authority state
        // through driver recovery rather than replaying checkpoints.
        Box::pin(async move { Ok(AuthorityRecoveryData::new(Vec::new(), BTreeMap::new())) })
    }
}

impl AuthorityRecoveryProvenance for ZoneAuthorityLedger {
    fn validate<'a>(
        &'a self,
        _operation: &'a AuthorityStorageOperation,
    ) -> AuthorityFuture<'a, ()> {
        // The ledger holds no recovered rows, so there is nothing to
        // re-validate against external inventory.
        Box::pin(async move { Ok(()) })
    }
}
