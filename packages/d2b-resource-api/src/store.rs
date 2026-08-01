//! Checked resource-store backend boundary.

use std::{future::Future, sync::Arc};

use d2b_resource_store::{
    StoreCommitResult, StoreError, StoreGetRequest, StoreInspectSchemaRequest, StoreListRequest,
    StoreListResult, StoreResolveRequest, StoreResolvedIdentity, StoreWatchReceipt,
    StoreWatchRequest, StoredResource, StoredSchema,
};

use crate::admission::{AdmittedMutation, StoreAdmissionBinding, VerifiedMutation};

/// Trusted persistence seam reached only after instance-bound admission verification.
///
/// The checked store guarantees that a caller cannot construct the
/// [`VerifiedMutation`] passed to [`ResourceStoreBackend::commit_verified`]
/// without a successful native authorization evaluation, and that the
/// resulting evidence is verified against the identity of this store.
///
/// This seal does not constrain the backend implementation. A backend could
/// ignore a verified mutation, change storage through another path, or omit
/// required transaction checks. Implementations are therefore part of the
/// trusted computing base: they must mutate only from the supplied
/// [`VerifiedMutation`], recheck its captured revisions in the write
/// transaction, preserve the store's structural and atomicity invariants, and
/// expose no independent mutation path. A production backend requires security
/// review and conformance tests for these obligations before it is registered.
pub trait ResourceStoreBackend: Send + Sync {
    fn get(
        &self,
        request: StoreGetRequest,
    ) -> impl Future<Output = Result<StoredResource, StoreError>> + Send;

    fn list(
        &self,
        request: StoreListRequest,
    ) -> impl Future<Output = Result<StoreListResult, StoreError>> + Send;

    fn watch(
        &self,
        request: StoreWatchRequest,
    ) -> impl Future<Output = Result<StoreWatchReceipt, StoreError>> + Send;

    fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> impl Future<Output = Result<StoreResolvedIdentity, StoreError>> + Send;

    fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> impl Future<Output = Result<StoredSchema, StoreError>> + Send;

    fn commit_verified(
        &self,
        mutation: VerifiedMutation,
    ) -> impl Future<Output = Result<StoreCommitResult, StoreError>> + Send;
}

/// API bridge that owns the concrete verified-mutation store binding.
///
/// A caller cannot reach the store or fabricate its required mutation type.
///
/// ```compile_fail
/// use d2b_resource_api::{RedbBackend, VerifiedMutation};
///
/// fn bypass(backend: &RedbBackend, mutation: VerifiedMutation) {
///     let _ = backend.store.commit_verified(mutation);
/// }
/// ```
pub struct RedbBackend {
    store: d2b_resource_store_redb::RedbResourceStore<VerifiedMutation>,
}

impl core::fmt::Debug for RedbBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RedbBackend(<redacted>)")
    }
}

impl RedbBackend {
    pub const fn new(store: d2b_resource_store_redb::RedbResourceStore<VerifiedMutation>) -> Self {
        Self { store }
    }
}

impl d2b_resource_store_redb::VerifiedPreparedMutationView for crate::PreparedStoreMutation {
    fn mutation(&self) -> &d2b_resource_store::StoreMutation {
        self.mutation()
    }

    fn resource_uid(&self) -> Option<&d2b_contracts::v3::ResourceUid> {
        self.resource_uid()
    }
}

impl d2b_resource_store_redb::VerifiedMutationView for VerifiedMutation {
    type Prepared = crate::PreparedStoreMutation;

    fn authorization(&self) -> &d2b_resource_store::AdmittedAuthorization {
        self.authorization()
    }

    fn policy_snapshot(&self) -> d2b_resource_store::PolicySnapshot {
        self.policy_snapshot()
    }

    fn operation(&self) -> &d2b_resource_store::StoreOperationContext {
        self.operation()
    }

    fn mutations(&self) -> &[Self::Prepared] {
        self.mutations()
    }
}

impl ResourceStoreBackend for RedbBackend {
    async fn get(&self, request: StoreGetRequest) -> Result<StoredResource, StoreError> {
        self.store.get(request).await
    }

    async fn list(&self, request: StoreListRequest) -> Result<StoreListResult, StoreError> {
        self.store.list(request).await
    }

    async fn watch(&self, request: StoreWatchRequest) -> Result<StoreWatchReceipt, StoreError> {
        self.store.watch(request).await
    }

    async fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> Result<StoreResolvedIdentity, StoreError> {
        self.store.resolve_ref(request).await
    }

    async fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> Result<StoredSchema, StoreError> {
        self.store.inspect_schema(request).await
    }

    async fn commit_verified(
        &self,
        mutation: VerifiedMutation,
    ) -> Result<StoreCommitResult, StoreError> {
        self.store.commit_verified(mutation).await
    }
}

#[cfg(test)]
mod redb_tests {
    use super::*;

    #[test]
    fn concrete_redb_backend_implements_the_checked_api_seam() {
        fn assert_backend<T: ResourceStoreBackend>() {}
        assert_backend::<RedbBackend>();
    }
}

/// A native authorizer has already been bound to a store backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreBindingError;

impl core::fmt::Display for StoreBindingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("native authorizer is already bound to a store backend")
    }
}

impl std::error::Error for StoreBindingError {}

pub(super) struct CheckedResourceStore<S> {
    backend: Arc<S>,
    admission: StoreAdmissionBinding,
}

impl<S> CheckedResourceStore<S> {
    pub(super) const fn new(backend: Arc<S>, admission: StoreAdmissionBinding) -> Self {
        Self { backend, admission }
    }
}

impl<S> CheckedResourceStore<S>
where
    S: ResourceStoreBackend,
{
    pub(super) fn get(
        &self,
        request: StoreGetRequest,
    ) -> impl Future<Output = Result<StoredResource, StoreError>> + Send {
        self.backend.get(request)
    }

    pub(super) fn list(
        &self,
        request: StoreListRequest,
    ) -> impl Future<Output = Result<StoreListResult, StoreError>> + Send {
        self.backend.list(request)
    }

    pub(super) fn watch(
        &self,
        request: StoreWatchRequest,
    ) -> impl Future<Output = Result<StoreWatchReceipt, StoreError>> + Send {
        self.backend.watch(request)
    }

    pub(super) fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> impl Future<Output = Result<StoreResolvedIdentity, StoreError>> + Send {
        self.backend.resolve_ref(request)
    }

    pub(super) fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> impl Future<Output = Result<StoredSchema, StoreError>> + Send {
        self.backend.inspect_schema(request)
    }

    pub(super) fn commit(
        &self,
        mutation: AdmittedMutation,
    ) -> impl Future<Output = Result<StoreCommitResult, StoreError>> + Send {
        let verified = self.admission.verify(mutation);
        async move { self.backend.commit_verified(verified?).await }
    }
}
