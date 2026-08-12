//! Durable Host-global authority operation ownership.
//!
//! The Zone redb store owns the bytes and commit boundary. Core owns the
//! typed row and recovery validation. Only this adapter can turn storage rows
//! into the private receipt consumed by `HostGlobalAuthorityIndex`.

use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use crate::authority::{
    AuthorityOperationState, AuthorityRecoveryReceipt, AuthorityStorageClaim,
    AuthorityStorageOperation, ExternalNicRecoveryInventory, HostGlobalAuthorityIndex,
};
use d2b_resource_store::{StoreOperationContext, StoreResolveRequest};
use d2b_resource_store_redb::{
    AuthorityOperation, AuthorityOperationState as StoreAuthorityOperationState, RedbResourceStore,
};

/// Stable persistence error without host paths or storage details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityPersistenceError {
    StoreUnavailable,
    /// The adapter could not prove whether the prepare transaction committed.
    CommitUnknown,
    RowInvalid,
    StateInvalid,
}

impl core::fmt::Display for AuthorityPersistenceError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::StoreUnavailable => "authority-persistence-unavailable",
            Self::CommitUnknown => "authority-persistence-commit-unknown",
            Self::RowInvalid => "authority-persistence-row-invalid",
            Self::StateInvalid => "authority-persistence-state-invalid",
        })
    }
}

impl std::error::Error for AuthorityPersistenceError {}

pub type AuthorityFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, AuthorityPersistenceError>> + Send + 'a>>;

mod sealed {
    pub trait RecoveryProvenance {}
}

/// Opaque Core-owned handle for one prepared operation.
///
/// The operation id is intentionally not exposed as a public accessor. A
/// persistence implementation may bind this handle to its store instance and
/// reject handles minted by another store.
pub struct AuthorityOperationCapability {
    pub(crate) operation_id: String,
    pub(crate) store_binding_digest: String,
    pub(crate) nonce: u64,
}

impl core::fmt::Debug for AuthorityOperationCapability {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuthorityOperationCapability(<store-bound>)")
    }
}

impl AuthorityOperationCapability {
    pub(crate) fn new(
        operation_id: String,
        store_binding_digest: String,
        nonce: u64,
    ) -> Result<Self, AuthorityPersistenceError> {
        if operation_id.is_empty() || operation_id.len() > 512 || store_binding_digest.is_empty() {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        Ok(Self {
            operation_id,
            store_binding_digest,
            nonce,
        })
    }
}

/// Typed persistence port for an authority operation lifecycle.
pub trait AuthorityPersistence: Send + Sync {
    /// Write the pending claim before any broker dispatch.
    fn prepare<'a>(
        &'a self,
        operation_id: &'a str,
        claim: &'a AuthorityStorageClaim,
    ) -> AuthorityFuture<'a, AuthorityOperationCapability>;

    /// Record the effect outcome while retaining ownership.
    fn record_effect<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
        state: AuthorityOperationState,
    ) -> AuthorityFuture<'a, ()>;

    /// Record confirmed closure before releasing the in-memory lease.
    fn record_close<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
    ) -> AuthorityFuture<'a, ()>;

    /// Mark a closed operation released.
    fn release<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
    ) -> AuthorityFuture<'a, ()>;

    /// Load every non-terminal row and terminal operation identity before
    /// new admission.
    fn recover<'a>(&'a self) -> AuthorityFuture<'a, AuthorityRecoveryReceipt>;
}

/// Trusted provenance port for recovery rows. Implementations must validate
/// authoritative owner generations and external inventory identities; digest
/// equality alone is not sufficient.
pub trait AuthorityRecoveryProvenance: sealed::RecoveryProvenance + Send + Sync {
    fn validate<'a>(&'a self, operation: &'a AuthorityStorageOperation) -> AuthorityFuture<'a, ()>;
}

/// Production authority persistence owner for one Zone redb store.
pub struct RedbAuthorityPersistence {
    store: Arc<RedbResourceStore>,
    operation_capabilities:
        Mutex<BTreeMap<String, Arc<d2b_resource_store_redb::AuthorityOperationCapability>>>,
    external_inventory: Option<Arc<dyn ExternalNicRecoveryInventory>>,
}

impl core::fmt::Debug for RedbAuthorityPersistence {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("RedbAuthorityPersistence(<store-bound>)")
    }
}

impl RedbAuthorityPersistence {
    /// Bind the port to the already opened, broker-owned Zone store.
    pub fn new(store: Arc<RedbResourceStore>) -> Self {
        Self {
            store,
            operation_capabilities: Mutex::new(BTreeMap::new()),
            external_inventory: None,
        }
    }

    pub fn with_external_inventory(
        mut self,
        inventory: Arc<dyn ExternalNicRecoveryInventory>,
    ) -> Self {
        self.external_inventory = Some(inventory);
        self
    }
}

impl AuthorityPersistence for RedbAuthorityPersistence {
    fn prepare<'a>(
        &'a self,
        operation_id: &'a str,
        claim: &'a AuthorityStorageClaim,
    ) -> AuthorityFuture<'a, AuthorityOperationCapability> {
        Box::pin(async move {
            let claim_digest = crate::authority::claim_digest(claim)
                .map_err(|_| AuthorityPersistenceError::RowInvalid)?;
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
            let binding_digest = self.store.authority_binding_digest(&claim_digest);
            let row = AuthorityStorageOperation {
                operation_id: operation_id.to_owned(),
                claim: claim.clone(),
                state: AuthorityOperationState::Pending,
                claim_digest: claim_digest.clone(),
                store_binding_digest: binding_digest.clone(),
            };
            AuthorityRecoveryProvenance::validate(self, &row).await?;
            let payload =
                serde_json::to_vec(&row).map_err(|_| AuthorityPersistenceError::RowInvalid)?;
            let capability = self
                .store
                .prepare_authority_operation(operation_id.to_owned(), payload, &claim_digest)
                .await
                .map_err(|_| AuthorityPersistenceError::CommitUnknown)?;
            let nonce = capability.nonce();
            self.operation_capabilities
                .lock()
                .map_err(|_| AuthorityPersistenceError::StateInvalid)?
                .insert(operation_id.to_owned(), Arc::new(capability));
            AuthorityOperationCapability::new(operation_id.to_owned(), binding_digest, nonce)
        })
    }

    fn record_effect<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
        state: AuthorityOperationState,
    ) -> AuthorityFuture<'a, ()> {
        Box::pin(async move {
            let state = match state {
                AuthorityOperationState::Pending => StoreAuthorityOperationState::Pending,
                AuthorityOperationState::EffectConfirmed => {
                    StoreAuthorityOperationState::EffectConfirmed
                }
                AuthorityOperationState::EffectRetryable => {
                    StoreAuthorityOperationState::EffectRetryable
                }
                AuthorityOperationState::EffectTerminal => {
                    StoreAuthorityOperationState::EffectTerminal
                }
                AuthorityOperationState::Closing => StoreAuthorityOperationState::Closing,
                AuthorityOperationState::Closed | AuthorityOperationState::Released => {
                    return Err(AuthorityPersistenceError::StateInvalid);
                }
            };
            let store_capability = self.validate_capability(capability)?;
            store_capability
                .record_effect(state)
                .await
                .map_err(|_| AuthorityPersistenceError::StoreUnavailable)
        })
    }

    fn record_close<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
    ) -> AuthorityFuture<'a, ()> {
        Box::pin(async move {
            let store_capability = self.validate_capability(capability)?;
            store_capability
                .record_close()
                .await
                .map_err(|_| AuthorityPersistenceError::StoreUnavailable)
        })
    }

    fn release<'a>(
        &'a self,
        capability: &'a AuthorityOperationCapability,
    ) -> AuthorityFuture<'a, ()> {
        Box::pin(async move {
            let store_capability = self.validate_capability(capability)?;
            store_capability
                .release()
                .await
                .map_err(|_| AuthorityPersistenceError::StoreUnavailable)?;
            self.operation_capabilities
                .lock()
                .map_err(|_| AuthorityPersistenceError::StateInvalid)?
                .remove(&capability.operation_id);
            Ok(())
        })
    }

    fn recover<'a>(&'a self) -> AuthorityFuture<'a, AuthorityRecoveryReceipt> {
        Box::pin(async move {
            let rows = self
                .store
                .authority_operations()
                .await
                .map_err(|_| AuthorityPersistenceError::StoreUnavailable)?;
            recovery_receipt(rows, &self.store, &self.operation_capabilities).await
        })
    }
}

impl sealed::RecoveryProvenance for RedbAuthorityPersistence {}

impl AuthorityRecoveryProvenance for RedbAuthorityPersistence {
    fn validate<'a>(&'a self, operation: &'a AuthorityStorageOperation) -> AuthorityFuture<'a, ()> {
        Box::pin(async move {
            let claim_digest = crate::authority::claim_digest(&operation.claim)
                .map_err(|_| AuthorityPersistenceError::RowInvalid)?;
            if claim_digest != operation.claim_digest
                || self.store.authority_binding_digest(&claim_digest)
                    != operation.store_binding_digest
            {
                return Err(AuthorityPersistenceError::RowInvalid);
            }
            if !matches!(
                operation.state,
                AuthorityOperationState::Closed | AuthorityOperationState::Released
            ) {
                let owner_proof = match &operation.claim {
                    AuthorityStorageClaim::Generic(claim) => claim.owner_proof(),
                    AuthorityStorageClaim::ExternalNic(claim) => claim.owner_proof(),
                };
                let Some(owner_ref) = owner_proof.resource_ref() else {
                    return Err(AuthorityPersistenceError::RowInvalid);
                };
                let resolved = self
                    .store
                    .resolve_ref(StoreResolveRequest {
                        operation: StoreOperationContext {
                            operation_id: format!("authority-recovery:{}", operation.operation_id),
                            idempotency_key: None,
                            correlation_id: "authority-recovery".to_owned(),
                            trace_id: None,
                            deadline_ms: 1,
                        },
                        zone: self.store.identity().zone().clone(),
                        target: owner_ref.clone(),
                        expected_uid: Some(owner_proof.resource_uid().clone()),
                    })
                    .await
                    .map_err(|_| AuthorityPersistenceError::RowInvalid)?;
                if resolved.uid != *owner_proof.resource_uid()
                    || resolved.generation != owner_proof.generation()
                {
                    return Err(AuthorityPersistenceError::RowInvalid);
                }
            }
            if !matches!(
                operation.state,
                AuthorityOperationState::Closed | AuthorityOperationState::Released
            ) && matches!(operation.claim, AuthorityStorageClaim::ExternalNic(_))
            {
                let AuthorityStorageClaim::ExternalNic(claim) = &operation.claim else {
                    unreachable!();
                };
                let Some(inventory) = &self.external_inventory else {
                    return Err(AuthorityPersistenceError::RowInvalid);
                };
                if !inventory.contains_identity(claim.host_uid(), claim.identity_digest()) {
                    return Err(AuthorityPersistenceError::RowInvalid);
                }
            }
            Ok(())
        })
    }
}

impl RedbAuthorityPersistence {
    fn validate_capability(
        &self,
        capability: &AuthorityOperationCapability,
    ) -> Result<Arc<d2b_resource_store_redb::AuthorityOperationCapability>, AuthorityPersistenceError>
    {
        let store_capability = self
            .operation_capabilities
            .lock()
            .map_err(|_| AuthorityPersistenceError::StateInvalid)?
            .get(&capability.operation_id)
            .cloned()
            .ok_or(AuthorityPersistenceError::StateInvalid)?;
        if capability.store_binding_digest.is_empty()
            || capability.nonce != store_capability.nonce()
            || !store_capability.matches_binding_digest(&capability.store_binding_digest)
        {
            return Err(AuthorityPersistenceError::StateInvalid);
        }
        Ok(store_capability)
    }
}

/// Rehydrate the central index from the production Zone persistence owner.
pub async fn rehydrate_from_persistence(
    persistence: &dyn AuthorityPersistence,
) -> Result<HostGlobalAuthorityIndex, AuthorityPersistenceError> {
    let receipt = persistence.recover().await?;
    HostGlobalAuthorityIndex::rehydrate(receipt).map_err(|_| AuthorityPersistenceError::RowInvalid)
}

/// Coordinates restart recovery so no active authority row can silently
/// become readiness. Each operation must be observed, adopted, closed, or
/// quarantined through this owner.
pub struct AuthorityRecoveryCoordinator {
    index: Arc<tokio::sync::Mutex<HostGlobalAuthorityIndex>>,
    persistence: Arc<dyn AuthorityPersistence>,
}

impl core::fmt::Debug for AuthorityRecoveryCoordinator {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AuthorityRecoveryCoordinator(<store-bound>)")
    }
}

impl AuthorityRecoveryCoordinator {
    pub async fn recover_with_provenance(
        persistence: Arc<dyn AuthorityPersistence>,
        provenance: &dyn AuthorityRecoveryProvenance,
    ) -> Result<Self, AuthorityPersistenceError> {
        let receipt = persistence.recover().await?;
        for operation in receipt.operations() {
            provenance.validate(operation).await?;
        }
        let index = HostGlobalAuthorityIndex::rehydrate(receipt)
            .map_err(|_| AuthorityPersistenceError::RowInvalid)?;
        Ok(Self {
            index: Arc::new(tokio::sync::Mutex::new(index)),
            persistence,
        })
    }

    pub fn index(&self) -> Arc<tokio::sync::Mutex<HostGlobalAuthorityIndex>> {
        Arc::clone(&self.index)
    }

    pub async fn is_ready_for_readiness(&self) -> bool {
        self.index.lock().await.is_ready_for_readiness()
    }

    pub async fn resolve_observed_and_adopted(
        &self,
        operation_id: &str,
    ) -> Result<(), AuthorityPersistenceError> {
        self.index
            .lock()
            .await
            .resolve_recovered_operation(
                operation_id,
                crate::authority::AuthorityRecoveryResolution::ObservedAndAdopted,
            )
            .map_err(|_| AuthorityPersistenceError::RowInvalid)
    }

    pub async fn resolve_observed_closed(
        &self,
        operation_id: &str,
    ) -> Result<(), AuthorityPersistenceError> {
        let capability = self
            .index
            .lock()
            .await
            .take_recovery_capability(operation_id)
            .ok_or(AuthorityPersistenceError::StateInvalid)?;
        if let Err(error) = self.persistence.record_close(&capability).await {
            let mut index = self.index.lock().await;
            index.restore_recovery_capability(operation_id.to_owned(), capability);
            index.quarantine_recovered_operation(operation_id);
            return Err(error);
        }
        if let Err(error) = self.persistence.release(&capability).await {
            let mut index = self.index.lock().await;
            index.restore_recovery_capability(operation_id.to_owned(), capability);
            index.quarantine_recovered_operation(operation_id);
            return Err(error);
        }
        self.index
            .lock()
            .await
            .resolve_recovered_operation(
                operation_id,
                crate::authority::AuthorityRecoveryResolution::ObservedClosed,
            )
            .map_err(|_| AuthorityPersistenceError::RowInvalid)
    }

    pub async fn quarantine(&self, operation_id: &str) -> Result<(), AuthorityPersistenceError> {
        let mut index = self.index.lock().await;
        index.quarantine_recovered_operation(operation_id);
        Ok(())
    }
}

async fn recovery_receipt(
    rows: Vec<AuthorityOperation>,
    store: &Arc<RedbResourceStore>,
    operation_capabilities: &Mutex<
        BTreeMap<String, Arc<d2b_resource_store_redb::AuthorityOperationCapability>>,
    >,
) -> Result<AuthorityRecoveryReceipt, AuthorityPersistenceError> {
    let mut operations = Vec::new();
    let mut capabilities = BTreeMap::new();
    let mut operation_ids = std::collections::BTreeSet::new();

    for row in rows {
        if !operation_ids.insert(row.operation_id.clone()) {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        let mut stored: AuthorityStorageOperation = serde_json::from_slice(&row.payload)
            .map_err(|_| AuthorityPersistenceError::RowInvalid)?;
        if stored.operation_id != row.operation_id {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        if stored.store_binding_digest != store.authority_binding_digest(&stored.claim_digest) {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        // The redb lifecycle column is the authoritative state. The payload
        // is an untrusted claim envelope and older physical rows may carry
        // the prepare-time state, so never let it override the committed
        // transition.
        stored.state = match row.state {
            StoreAuthorityOperationState::Pending => AuthorityOperationState::Pending,
            StoreAuthorityOperationState::EffectConfirmed => {
                AuthorityOperationState::EffectConfirmed
            }
            StoreAuthorityOperationState::EffectRetryable => {
                AuthorityOperationState::EffectRetryable
            }
            StoreAuthorityOperationState::EffectTerminal => AuthorityOperationState::EffectTerminal,
            StoreAuthorityOperationState::Closing => AuthorityOperationState::Closing,
            StoreAuthorityOperationState::Closed => AuthorityOperationState::Closed,
            StoreAuthorityOperationState::Released => AuthorityOperationState::Released,
        };
        if !matches!(
            stored.state,
            AuthorityOperationState::Closed | AuthorityOperationState::Released
        ) {
            let store_capability = store
                .resume_authority_operation(
                    stored.operation_id.clone(),
                    &stored.store_binding_digest,
                )
                .await
                .map_err(|_| AuthorityPersistenceError::RowInvalid)?;
            let nonce = store_capability.nonce();
            operation_capabilities
                .lock()
                .map_err(|_| AuthorityPersistenceError::StateInvalid)?
                .insert(stored.operation_id.clone(), Arc::new(store_capability));
            let capability = AuthorityOperationCapability::new(
                stored.operation_id.clone(),
                stored.store_binding_digest.clone(),
                nonce,
            )?;
            capabilities.insert(stored.operation_id.clone(), capability);
        }
        operations.push(stored);
    }

    HostGlobalAuthorityIndex::recovery_receipt_from_operations_with_capabilities(
        operations,
        None,
        capabilities,
    )
    .map_err(|_| AuthorityPersistenceError::RowInvalid)
}
