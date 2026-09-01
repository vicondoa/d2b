//! Durable Host-global authority operation ownership.
//!
//! The Zone redb store owns the bytes and commit boundary. Core owns the
//! typed row and recovery validation. Only this adapter can turn storage rows
//! into the private receipt consumed by `HostGlobalAuthorityIndex`.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use crate::zone_authority::ZONE_GENERATION_PUBLICATION_OPERATION_PREFIX;
use d2b_core_controller::authority::{
    AuthorityOperationState, AuthorityStorageClaim, AuthorityStorageOperation,
    ExternalNicRecoveryInventory, claim_digest,
};
use d2b_core_controller::authority_persistence::{
    AuthorityFuture, AuthorityOperationCapability, AuthorityPersistence, AuthorityPersistenceError,
    AuthorityRecoveryData, AuthorityRecoveryProvenance, PreparedAuthorityOperation,
};
use d2b_resource_store::{StoreOperationContext, StoreResolveRequest};
use d2b_resource_store_redb::{
    AuthorityOperation, AuthorityOperationState as StoreAuthorityOperationState, RedbResourceStore,
};

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
            PreparedAuthorityOperation::new(operation_id.to_owned(), binding_digest, nonce)
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
                .remove(capability.operation_id());
            Ok(())
        })
    }

    fn recover<'a>(&'a self) -> AuthorityFuture<'a, AuthorityRecoveryData> {
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

impl AuthorityRecoveryProvenance for RedbAuthorityPersistence {
    fn validate<'a>(&'a self, operation: &'a AuthorityStorageOperation) -> AuthorityFuture<'a, ()> {
        Box::pin(async move {
            let claim_digest = claim_digest(&operation.claim)
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
            .get(capability.operation_id())
            .cloned()
            .ok_or(AuthorityPersistenceError::StateInvalid)?;
        if capability.store_binding_digest().is_empty()
            || capability.nonce() != store_capability.nonce()
            || !store_capability.matches_binding_digest(capability.store_binding_digest())
        {
            return Err(AuthorityPersistenceError::StateInvalid);
        }
        Ok(store_capability)
    }
}

async fn recovery_receipt(
    rows: Vec<AuthorityOperation>,
    store: &Arc<RedbResourceStore>,
    operation_capabilities: &Mutex<
        BTreeMap<String, Arc<d2b_resource_store_redb::AuthorityOperationCapability>>,
    >,
) -> Result<AuthorityRecoveryData, AuthorityPersistenceError> {
    let mut operations = Vec::new();
    let mut prepared_operations = BTreeMap::new();
    let mut operation_ids = std::collections::BTreeSet::new();

    for row in rows {
        // The all-Zone publication marker shares the store's durable
        // operation transaction but is not a Host-global authority claim.
        if is_zone_generation_publication(&row) {
            continue;
        }
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
            let prepared = PreparedAuthorityOperation::new(
                stored.operation_id.clone(),
                stored.store_binding_digest.clone(),
                nonce,
            )?;
            prepared_operations.insert(stored.operation_id.clone(), prepared);
        }
        operations.push(stored);
    }

    Ok(AuthorityRecoveryData::new(operations, prepared_operations))
}

fn is_zone_generation_publication(row: &AuthorityOperation) -> bool {
    row.operation_id
        .starts_with(ZONE_GENERATION_PUBLICATION_OPERATION_PREFIX)
        && serde_json::from_slice::<serde_json::Value>(&row.payload)
            .ok()
            .is_some_and(|value| {
                value.get("publication").and_then(serde_json::Value::as_str)
                    == Some("zone-resource-plane")
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_publication_rows_are_not_rehydrated_as_host_authority() {
        let row = AuthorityOperation {
            operation_id: format!(
                "{ZONE_GENERATION_PUBLICATION_OPERATION_PREFIX}sha256:{}",
                "a".repeat(64)
            ),
            payload: serde_json::to_vec(&serde_json::json!({
                "publication": "zone-resource-plane"
            }))
            .expect("publication payload"),
            state: StoreAuthorityOperationState::Pending,
        };
        assert!(is_zone_generation_publication(&row));
    }

    #[test]
    fn malformed_or_other_operation_rows_still_fail_closed() {
        for row in [
            AuthorityOperation {
                operation_id: format!(
                    "{ZONE_GENERATION_PUBLICATION_OPERATION_PREFIX}sha256:{}",
                    "b".repeat(64)
                ),
                payload: b"not-json".to_vec(),
                state: StoreAuthorityOperationState::Pending,
            },
            AuthorityOperation {
                operation_id: "authority-operation".to_owned(),
                payload: serde_json::to_vec(&serde_json::json!({
                    "publication": "zone-resource-plane"
                }))
                .expect("payload"),
                state: StoreAuthorityOperationState::Pending,
            },
        ] {
            assert!(!is_zone_generation_publication(&row));
        }
    }
}
