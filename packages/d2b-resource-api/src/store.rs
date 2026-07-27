//! Checked resource-store backend boundary.

use std::{future::Future, sync::Arc};

use d2b_resource_store::{
    StoreCommitResult, StoreError, StoreGetRequest, StoreInspectSchemaRequest, StoreListRequest,
    StoreListResult, StoreResolveRequest, StoreResolvedIdentity, StoreWatchReceipt,
    StoreWatchRequest, StoredResource, StoredSchema,
};

use crate::admission::{AdmittedMutation, StoreAdmissionBinding, VerifiedMutation};

/// Backend seam reached only after instance-bound admission verification.
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
