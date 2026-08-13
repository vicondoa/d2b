//! Durable Host-global authority operation ownership.
//!
//! The Zone redb store owns the bytes and commit boundary. Core owns the
//! typed row and recovery validation. Only this adapter can turn storage rows
//! into the private receipt consumed by `HostGlobalAuthorityIndex`.

use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

use crate::authority::{
    AuthorityOperationState, AuthorityRecoveryReceipt, AuthorityStorageClaim,
    AuthorityStorageOperation, HostGlobalAuthorityIndex,
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

/// Non-authorizing evidence returned by a storage adapter after it has
/// durably prepared one operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAuthorityOperation {
    operation_id: String,
    store_binding_digest: String,
    nonce: u64,
}

impl PreparedAuthorityOperation {
    pub fn new(
        operation_id: String,
        store_binding_digest: String,
        nonce: u64,
    ) -> Result<Self, AuthorityPersistenceError> {
        if operation_id.is_empty()
            || operation_id.len() > 512
            || store_binding_digest.is_empty()
            || nonce == 0
        {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        Ok(Self {
            operation_id,
            store_binding_digest,
            nonce,
        })
    }

    pub(crate) fn matches_operation(&self, operation: &AuthorityStorageOperation) -> bool {
        self.operation_id == operation.operation_id
            && self.store_binding_digest == operation.store_binding_digest
            && self.nonce != 0
    }
}

/// Non-authorizing recovery data returned by a storage adapter. Core validates
/// the operations and provenance before minting any authority capabilities.
pub struct AuthorityRecoveryData {
    operations: Vec<AuthorityStorageOperation>,
    prepared_operations: BTreeMap<String, PreparedAuthorityOperation>,
}

impl AuthorityRecoveryData {
    pub fn new(
        operations: Vec<AuthorityStorageOperation>,
        prepared_operations: BTreeMap<String, PreparedAuthorityOperation>,
    ) -> Self {
        Self {
            operations,
            prepared_operations,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<AuthorityStorageOperation>,
        BTreeMap<String, PreparedAuthorityOperation>,
    ) {
        (self.operations, self.prepared_operations)
    }

    pub(crate) fn operations(&self) -> &[AuthorityStorageOperation] {
        &self.operations
    }
}

/// Opaque Core-owned handle for one prepared operation.
///
/// The operation id is intentionally not exposed as a public accessor. A
/// persistence implementation may bind this handle to its store instance and
/// reject handles minted by another store.
///
/// ```compile_fail
/// use d2b_core_controller::authority_persistence::AuthorityOperationCapability;
/// let _ = AuthorityOperationCapability::new(
///     "operation".to_owned(),
///     "sha256:".to_owned() + &"0".repeat(64),
///     1,
/// );
/// ```
///
/// ```compile_fail
/// use d2b_core_controller::authority_persistence::AuthorityOperationCapability;
/// let _ = AuthorityOperationCapability::default();
/// ```
///
/// ```compile_fail
/// use d2b_core_controller::authority_persistence::AuthorityOperationCapability;
/// let _: AuthorityOperationCapability = serde_json::from_str("{}").unwrap();
/// ```
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
    pub(crate) fn from_prepared(
        operation_id: &str,
        prepared: PreparedAuthorityOperation,
    ) -> Result<Self, AuthorityPersistenceError> {
        if prepared.operation_id != operation_id {
            return Err(AuthorityPersistenceError::RowInvalid);
        }
        Ok(Self {
            operation_id: prepared.operation_id,
            store_binding_digest: prepared.store_binding_digest,
            nonce: prepared.nonce,
        })
    }

    #[doc(hidden)]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    #[doc(hidden)]
    pub fn store_binding_digest(&self) -> &str {
        &self.store_binding_digest
    }

    #[doc(hidden)]
    pub const fn nonce(&self) -> u64 {
        self.nonce
    }
}

/// Typed persistence port for an authority operation lifecycle.
pub trait AuthorityPersistence: Send + Sync {
    /// Write the pending claim before any broker dispatch.
    fn prepare<'a>(
        &'a self,
        operation_id: &'a str,
        claim: &'a AuthorityStorageClaim,
    ) -> AuthorityFuture<'a, PreparedAuthorityOperation>;

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
    fn recover<'a>(&'a self) -> AuthorityFuture<'a, AuthorityRecoveryData>;
}

/// Trusted provenance port for recovery rows. Implementations must validate
/// authoritative owner generations and external inventory identities; digest
/// equality alone is not sufficient.
pub trait AuthorityRecoveryProvenance: Send + Sync {
    fn validate<'a>(&'a self, operation: &'a AuthorityStorageOperation) -> AuthorityFuture<'a, ()>;
}

/// Rehydrate the central index from the production Zone persistence owner.
pub async fn rehydrate_from_persistence(
    persistence: &dyn AuthorityPersistence,
    provenance: &dyn AuthorityRecoveryProvenance,
) -> Result<HostGlobalAuthorityIndex, AuthorityPersistenceError> {
    let receipt = validated_recovery_receipt(persistence.recover().await?, provenance).await?;
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
        let receipt = validated_recovery_receipt(persistence.recover().await?, provenance).await?;
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

async fn validated_recovery_receipt(
    data: AuthorityRecoveryData,
    provenance: &dyn AuthorityRecoveryProvenance,
) -> Result<AuthorityRecoveryReceipt, AuthorityPersistenceError> {
    for operation in data.operations() {
        provenance.validate(operation).await?;
    }
    let (operations, prepared_operations) = data.into_parts();
    HostGlobalAuthorityIndex::recovery_receipt_from_operations_with_prepared_capabilities(
        operations,
        None,
        prepared_operations,
    )
    .map_err(|_| AuthorityPersistenceError::RowInvalid)
}
