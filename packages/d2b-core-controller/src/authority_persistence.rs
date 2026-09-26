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

    /// The nonce this prepared operation was minted with. Hidden like the
    /// capability's accessor: it exists so an adapter can pin that every
    /// prepare derives its own non-zero value instead of a constant.
    #[doc(hidden)]
    pub const fn nonce(&self) -> u64 {
        self.nonce
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::{AuthorityOwnerProof, AuthorityRequest, claim_digest};
    use d2b_contracts_resource::v3::{ResourceGeneration, ResourceUid};

    const OP_ID: &str = "recovered-helper-operation";
    const STORE_BINDING_DIGEST: &str =
        "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).unwrap()
    }

    fn authority_proof(value: &str, generation: u64) -> AuthorityOwnerProof {
        AuthorityOwnerProof::new(uid(value), ResourceGeneration::new(generation).unwrap())
    }

    fn recovery_row(
        operation_id: &str,
    ) -> (AuthorityStorageOperation, PreparedAuthorityOperation, AuthorityRequest) {
        let request = AuthorityRequest::vsock_cid(
            uid("d93e4567-e89b-42d3-a456-426614174089"),
            87,
            authority_proof("e93e4567-e89b-42d3-a456-426614174090", 1),
        )
        .unwrap();
        let claim = AuthorityStorageClaim::Generic(request.durable_claim());
        let claim_digest = claim_digest(&claim).unwrap();
        let store_binding_digest = "sha256:".to_owned() + &"1".repeat(64);
        let operation = AuthorityStorageOperation {
            operation_id: operation_id.to_owned(),
            claim,
            state: AuthorityOperationState::Pending,
            claim_digest,
            store_binding_digest: store_binding_digest.clone(),
        };
        let prepared =
            PreparedAuthorityOperation::new(operation_id.to_owned(), store_binding_digest, 7)
                .unwrap();
        (operation, prepared, request)
    }

    fn recovery_data() -> AuthorityRecoveryData {
        let host_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let guest_uid = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
        let request = AuthorityRequest::guest_store_view_writer(
            host_uid,
            guest_uid,
            AuthorityOwnerProof::new(
                ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap(),
                ResourceGeneration::new(1).unwrap(),
            ),
        )
        .unwrap();
        let claim = AuthorityStorageClaim::Generic(request.durable_claim());
        let operation = AuthorityStorageOperation {
            operation_id: OP_ID.to_owned(),
            claim_digest: claim_digest(&claim).expect("claim digest"),
            state: AuthorityOperationState::Pending,
            claim,
            store_binding_digest: STORE_BINDING_DIGEST.to_owned(),
        };
        let prepared = PreparedAuthorityOperation::new(
            OP_ID.to_owned(),
            STORE_BINDING_DIGEST.to_owned(),
            7,
        )
        .expect("prepared operation");
        AuthorityRecoveryData::new(
            vec![operation],
            BTreeMap::from([(OP_ID.to_owned(), prepared)]),
        )
    }

    /// Provenance that accepts every recovered row; the coordinator tests
    /// exercise the receipt and rollback machinery, not provenance policy.
    struct AcceptingProvenance;

    impl AuthorityRecoveryProvenance for AcceptingProvenance {
        fn validate<'a>(
            &'a self,
            _operation: &'a AuthorityStorageOperation,
        ) -> AuthorityFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    /// Recovery double whose close/release legs can fail on demand.
    struct FailingRecoveryPersistence {
        fail_close: bool,
        fail_release: bool,
        operations: Vec<AuthorityStorageOperation>,
        prepared: BTreeMap<String, PreparedAuthorityOperation>,
    }

    impl AuthorityPersistence for FailingRecoveryPersistence {
        fn prepare<'a>(
            &'a self,
            _operation_id: &'a str,
            _claim: &'a AuthorityStorageClaim,
        ) -> AuthorityFuture<'a, PreparedAuthorityOperation> {
            Box::pin(async { Err(AuthorityPersistenceError::StoreUnavailable) })
        }

        fn record_effect<'a>(
            &'a self,
            _capability: &'a AuthorityOperationCapability,
            _state: AuthorityOperationState,
        ) -> AuthorityFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn record_close<'a>(
            &'a self,
            _capability: &'a AuthorityOperationCapability,
        ) -> AuthorityFuture<'a, ()> {
            if self.fail_close {
                Box::pin(async { Err(AuthorityPersistenceError::StoreUnavailable) })
            } else {
                Box::pin(async { Ok(()) })
            }
        }

        fn release<'a>(
            &'a self,
            _capability: &'a AuthorityOperationCapability,
        ) -> AuthorityFuture<'a, ()> {
            if self.fail_release {
                Box::pin(async { Err(AuthorityPersistenceError::StoreUnavailable) })
            } else {
                Box::pin(async { Ok(()) })
            }
        }

        fn recover<'a>(&'a self) -> AuthorityFuture<'a, AuthorityRecoveryData> {
            Box::pin(async {
                Ok(AuthorityRecoveryData::new(
                    self.operations.clone(),
                    self.prepared.clone(),
                ))
            })
        }
    }

    async fn failing_coordinator(
        operation_id: &str,
        fail_close: bool,
        fail_release: bool,
    ) -> AuthorityRecoveryCoordinator {
        let (operation, prepared, _) = recovery_row(operation_id);
        let persistence = Arc::new(FailingRecoveryPersistence {
            fail_close,
            fail_release,
            operations: vec![operation],
            prepared: BTreeMap::from([(operation_id.to_owned(), prepared)]),
        });
        AuthorityRecoveryCoordinator::recover_with_provenance(persistence, &AcceptingProvenance)
            .await
            .unwrap()
    }

    // R11 inventory note: tokio Mutex::lock().await resolves to the banned
    // Runtime::block_on bridge in clippy 1.97; sanctioned "cfg(test) helper".
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn coordinator_restores_capability_and_quarantines_after_record_close_failure() {
        let coordinator = failing_coordinator("recovery-close-failure", true, false).await;
        assert_eq!(
            coordinator
                .resolve_observed_closed("recovery-close-failure")
                .await
                .unwrap_err(),
            AuthorityPersistenceError::StoreUnavailable
        );
        assert!(
            coordinator
                .index()
                .lock()
                .await
                .take_recovery_capability("recovery-close-failure")
                .is_some(),
            "failed close must restore the recovery capability"
        );
        coordinator
            .resolve_observed_and_adopted("recovery-close-failure")
            .await
            .unwrap();
        assert!(
            !coordinator.is_ready_for_readiness().await,
            "failed close must quarantine the operation"
        );
    }

    // R11 inventory note: tokio Mutex::lock().await resolves to the banned
    // Runtime::block_on bridge in clippy 1.97; sanctioned "cfg(test) helper".
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn coordinator_restores_capability_and_quarantines_after_release_failure() {
        let coordinator = failing_coordinator("recovery-release-failure", false, true).await;
        assert_eq!(
            coordinator
                .resolve_observed_closed("recovery-release-failure")
                .await
                .unwrap_err(),
            AuthorityPersistenceError::StoreUnavailable
        );
        assert!(
            coordinator
                .index()
                .lock()
                .await
                .take_recovery_capability("recovery-release-failure")
                .is_some(),
            "failed release must restore the recovery capability"
        );
        coordinator
            .resolve_observed_and_adopted("recovery-release-failure")
            .await
            .unwrap();
        assert!(
            !coordinator.is_ready_for_readiness().await,
            "failed release must quarantine the operation"
        );
    }

    // R11 inventory note: tokio Mutex::lock().await resolves to the banned
    // Runtime::block_on bridge in clippy 1.97; sanctioned "cfg(test) helper".
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn coordinator_resolve_observed_and_adopted_clears_unresolved_set() {
        let coordinator = failing_coordinator("recovery-observed-adopted", false, false).await;
        assert!(!coordinator.is_ready_for_readiness().await);
        coordinator
            .resolve_observed_and_adopted("recovery-observed-adopted")
            .await
            .unwrap();
        assert!(coordinator.is_ready_for_readiness().await);
    }

    // R11 inventory note: tokio Mutex::lock().await resolves to the banned
    // Runtime::block_on bridge in clippy 1.97; sanctioned "cfg(test) helper".
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn coordinator_consumes_capability_and_reaches_readiness_on_successful_close() {
        let coordinator = failing_coordinator("recovery-close-success", false, false).await;
        coordinator
            .resolve_observed_closed("recovery-close-success")
            .await
            .unwrap();
        assert!(
            coordinator
                .index()
                .lock()
                .await
                .take_recovery_capability("recovery-close-success")
                .is_none(),
            "successful close must consume the recovery capability"
        );
        assert!(coordinator.is_ready_for_readiness().await);
    }

    #[tokio::test]
    async fn provenance_rejection_aborts_rehydration_with_the_adapter_error() {
        struct RefusingProvenance;
        impl AuthorityRecoveryProvenance for RefusingProvenance {
            fn validate<'a>(
                &'a self,
                _operation: &'a AuthorityStorageOperation,
            ) -> AuthorityFuture<'a, ()> {
                Box::pin(async { Err(AuthorityPersistenceError::StoreUnavailable) })
            }
        }

        assert!(matches!(
            validated_recovery_receipt(recovery_data(), &RefusingProvenance).await,
            Err(AuthorityPersistenceError::StoreUnavailable),
        ));
    }

    #[test]
    fn prepared_operation_rejects_empty_fields_and_zero_nonce() {
        assert!(
            PreparedAuthorityOperation::new(
                "op-1".to_owned(),
                STORE_BINDING_DIGEST.to_owned(),
                1,
            )
            .is_ok()
        );
        for (operation_id, store_digest, nonce) in [
            (String::new(), STORE_BINDING_DIGEST.to_owned(), 1),
            ("op-1".to_owned(), String::new(), 1),
            ("op-1".to_owned(), STORE_BINDING_DIGEST.to_owned(), 0),
        ] {
            assert_eq!(
                PreparedAuthorityOperation::new(operation_id, store_digest, nonce),
                Err(AuthorityPersistenceError::RowInvalid),
            );
        }
    }
}
