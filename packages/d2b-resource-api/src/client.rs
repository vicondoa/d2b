//! In-process async client used behind an authenticated d2b-bus binding.

use std::sync::Arc;

use d2b_contracts::{resource_proto as wire, v3::AuthenticatedSubjectContext};
use d2b_resource_store::ResourceStore;

use crate::{
    authz::AuthorizationState,
    service::{ResourceService, TrustedRequest, UpgradeDispatcher},
};

/// Resource client whose identity is fixed by its ComponentSession.
#[derive(Debug)]
pub struct ResourceClient<S, U> {
    service: Arc<ResourceService<S, U>>,
    subject: Arc<AuthenticatedSubjectContext>,
    state: AuthorizationState,
    relay_hop: bool,
}

impl<S, U> ResourceClient<S, U>
where
    S: ResourceStore,
    U: UpgradeDispatcher,
{
    pub fn from_component_session(
        service: Arc<ResourceService<S, U>>,
        subject: Arc<AuthenticatedSubjectContext>,
        state: AuthorizationState,
        relay_hop: bool,
    ) -> Self {
        Self {
            service,
            subject,
            state,
            relay_hop,
        }
    }

    pub async fn get(&self, request: wire::GetRequest) -> wire::GetResponse {
        self.service.get(self.trusted(request)).await
    }

    pub async fn list(&self, request: wire::ListRequest) -> wire::ListResponse {
        self.service.list(self.trusted(request)).await
    }

    pub async fn watch(&self, request: wire::WatchRequest) -> wire::WatchResponse {
        self.service.watch(self.trusted(request)).await
    }

    pub async fn create(&self, request: wire::CreateRequest) -> wire::CreateResponse {
        self.service.create(self.trusted(request)).await
    }

    pub async fn update_spec(&self, request: wire::UpdateSpecRequest) -> wire::UpdateSpecResponse {
        self.service.update_spec(self.trusted(request)).await
    }

    pub async fn update_status(
        &self,
        request: wire::UpdateStatusRequest,
    ) -> wire::UpdateStatusResponse {
        self.service.update_status(self.trusted(request)).await
    }

    pub async fn update_metadata(
        &self,
        request: wire::UpdateMetadataRequest,
    ) -> wire::UpdateMetadataResponse {
        self.service.update_metadata(self.trusted(request)).await
    }

    pub async fn update_finalizers(
        &self,
        request: wire::UpdateFinalizersRequest,
    ) -> wire::UpdateFinalizersResponse {
        self.service.update_finalizers(self.trusted(request)).await
    }

    pub async fn delete(&self, request: wire::DeleteRequest) -> wire::DeleteResponse {
        self.service.delete(self.trusted(request)).await
    }

    pub async fn commit_batch(
        &self,
        request: wire::CommitBatchRequest,
    ) -> wire::CommitBatchResponse {
        self.service.commit_batch(self.trusted(request)).await
    }

    pub async fn resolve_ref(&self, request: wire::ResolveRefRequest) -> wire::ResolveRefResponse {
        self.service.resolve_ref(self.trusted(request)).await
    }

    pub async fn inspect_schema(
        &self,
        request: wire::InspectSchemaRequest,
    ) -> wire::InspectSchemaResponse {
        self.service.inspect_schema(self.trusted(request)).await
    }

    pub async fn upgrade(&self, request: wire::UpgradeRequest) -> wire::UpgradeResponse {
        self.service.upgrade(self.trusted(request)).await
    }

    fn trusted<T>(&self, request: T) -> TrustedRequest<T> {
        TrustedRequest::from_component_session(
            Arc::clone(&self.subject),
            self.state.clone(),
            self.relay_hop,
            request,
        )
    }
}
