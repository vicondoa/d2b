//! Unregistered resource dispatch prepared for a future authenticated bus router.
//!
//! This workspace has no ComponentSession or d2b-bus implementation, so these
//! generated services have no production registration path.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use d2b_contracts::resource_proto as wire;

use crate::{
    ResourceStore,
    authz::authenticated_relay_hop,
    client::UnregisteredResourceClient,
    generated::d2b_resource_v3_ttrpc,
    identity::AuthenticatedSubjectContext,
    service::{ResourceService, TrustedRequest, UpgradeDispatcher},
};

/// Failure to bind an authenticated ComponentSession route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterBindingError;

impl core::fmt::Display for AdapterBindingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("authenticated bus route is not valid for the resource API")
    }
}

impl std::error::Error for AdapterBindingError {}

/// Current production reachability of the resource service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceApiReachability {
    AwaitingAuthenticatedComponentSessionRouter,
}

pub const RESOURCE_API_REACHABILITY: ResourceApiReachability =
    ResourceApiReachability::AwaitingAuthenticatedComponentSessionRouter;

/// Session-scoped dispatcher that is intentionally not registered on a server.
pub struct UnregisteredBusAdapter<S, U> {
    service: Arc<ResourceService<S, U>>,
    session: AuthenticatedSubjectContext,
}

impl<S, U> core::fmt::Debug for UnregisteredBusAdapter<S, U> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("UnregisteredBusAdapter(<redacted>)")
    }
}

impl<S, U> UnregisteredBusAdapter<S, U>
where
    S: ResourceStore,
    U: UpgradeDispatcher,
{
    /// Seal authenticated identity and policy state to one ComponentSession.
    pub fn bind_unregistered_session(
        service: Arc<ResourceService<S, U>>,
        session: AuthenticatedSubjectContext,
    ) -> Result<Self, AdapterBindingError> {
        authenticated_relay_hop(session.claims()).map_err(|_| AdapterBindingError)?;
        Ok(Self { service, session })
    }

    /// Return an explicitly unregistered in-process contract client.
    pub fn unregistered_client(&self) -> UnregisteredResourceClient<S, U> {
        UnregisteredResourceClient::from_session_capability(
            Arc::clone(&self.service),
            Arc::clone(self.session.claims()),
            self.session.authorization_state().clone(),
        )
    }

    pub(crate) fn service(&self) -> &ResourceService<S, U> {
        &self.service
    }

    pub(crate) fn trusted<T>(&self, request: T) -> TrustedRequest<T> {
        TrustedRequest::from_session_capability(
            Arc::clone(self.session.claims()),
            self.session.authorization_state().clone(),
            request,
        )
    }
}

impl<S, U> UnregisteredBusAdapter<S, U>
where
    S: ResourceStore + 'static,
    U: UpgradeDispatcher + 'static,
{
    /// Build the generated service map without registering it on a bus server.
    pub fn unregistered_ttrpc_services(
        self: Arc<Self>,
    ) -> HashMap<String, ttrpc::r#async::Service> {
        d2b_resource_v3_ttrpc::create_resource_service(self)
    }
}

#[async_trait]
impl<S, U> d2b_resource_v3_ttrpc::ResourceService for UnregisteredBusAdapter<S, U>
where
    S: ResourceStore + 'static,
    U: UpgradeDispatcher + 'static,
{
    async fn get(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::GetRequest,
    ) -> ttrpc::Result<wire::GetResponse> {
        Ok(self.service().get(self.trusted(request)).await)
    }

    async fn list(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::ListRequest,
    ) -> ttrpc::Result<wire::ListResponse> {
        Ok(self.service().list(self.trusted(request)).await)
    }

    async fn watch(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::WatchRequest,
    ) -> ttrpc::Result<wire::WatchResponse> {
        Ok(self.service().watch(self.trusted(request)).await)
    }

    async fn create(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::CreateRequest,
    ) -> ttrpc::Result<wire::CreateResponse> {
        Ok(self.service().create(self.trusted(request)).await)
    }

    async fn update_spec(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::UpdateSpecRequest,
    ) -> ttrpc::Result<wire::UpdateSpecResponse> {
        Ok(self.service().update_spec(self.trusted(request)).await)
    }

    async fn update_status(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::UpdateStatusRequest,
    ) -> ttrpc::Result<wire::UpdateStatusResponse> {
        Ok(self.service().update_status(self.trusted(request)).await)
    }

    async fn update_metadata(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::UpdateMetadataRequest,
    ) -> ttrpc::Result<wire::UpdateMetadataResponse> {
        Ok(self.service().update_metadata(self.trusted(request)).await)
    }

    async fn update_finalizers(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::UpdateFinalizersRequest,
    ) -> ttrpc::Result<wire::UpdateFinalizersResponse> {
        Ok(self
            .service()
            .update_finalizers(self.trusted(request))
            .await)
    }

    async fn delete(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::DeleteRequest,
    ) -> ttrpc::Result<wire::DeleteResponse> {
        Ok(self.service().delete(self.trusted(request)).await)
    }

    async fn commit_batch(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::CommitBatchRequest,
    ) -> ttrpc::Result<wire::CommitBatchResponse> {
        Ok(self.service().commit_batch(self.trusted(request)).await)
    }

    async fn resolve_ref(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::ResolveRefRequest,
    ) -> ttrpc::Result<wire::ResolveRefResponse> {
        Ok(self.service().resolve_ref(self.trusted(request)).await)
    }

    async fn inspect_schema(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::InspectSchemaRequest,
    ) -> ttrpc::Result<wire::InspectSchemaResponse> {
        Ok(self.service().inspect_schema(self.trusted(request)).await)
    }

    async fn upgrade(
        &self,
        _ctx: &ttrpc::r#async::TtrpcContext,
        request: wire::UpgradeRequest,
    ) -> ttrpc::Result<wire::UpgradeResponse> {
        Ok(self.service().upgrade(self.trusted(request)).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use d2b_contracts::v3::{
        AuthenticatedSubjectContext as SessionClaims, BindingDigest, ConfigurationGeneration,
        EvidenceClass, Locality, ReconnectGeneration, ResourceName, ResourceRef, ResourceTypeName,
        ResourceUid, SchemaFingerprint, ServiceName, SessionBinding, SessionPurpose,
        TranscriptHash, TransportBinding, ZoneId, ZoneRevision,
    };
    use d2b_resource_store::{
        PolicySnapshot, StoreCommitResult, StoreError, StoreGetRequest, StoreInspectSchemaRequest,
        StoreListRequest, StoreListResult, StoreResolveRequest, StoreResolvedIdentity,
        StoreWatchReceipt, StoreWatchRequest, StoredResource, StoredSchema,
    };
    use protobuf::{EnumOrUnknown, MessageField};

    use crate::authz::{
        ApiCatalog, AuthorizationState, BindingScope, BootstrapPhase, BoundSubject, CompiledRole,
        CompiledRoleBinding, NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority,
        ResourceVerb,
    };
    use crate::identity::issue_test_subject;
    use crate::{AdmissionVerifier, ResourceStoreBackend, VerifiedMutation};

    #[derive(Debug)]
    struct UnreachableStore {
        admission_verifier: AdmissionVerifier,
    }

    impl UnreachableStore {
        fn paired(
            catalog: ApiCatalog,
            policy: Option<PolicySet>,
        ) -> (Arc<Self>, Arc<NativeAuthorizer>) {
            let (authorizer, admission_verifier) = NativeAuthorizer::new(catalog, policy).unwrap();
            (Arc::new(Self { admission_verifier }), Arc::new(authorizer))
        }
    }

    impl ResourceStoreBackend for UnreachableStore {
        fn admission_verifier(&self) -> &AdmissionVerifier {
            &self.admission_verifier
        }

        async fn get(&self, _: StoreGetRequest) -> Result<StoredResource, StoreError> {
            unreachable!("authorization must run before the store")
        }

        async fn list(&self, _: StoreListRequest) -> Result<StoreListResult, StoreError> {
            unreachable!("authorization must run before the store")
        }

        async fn watch(&self, _: StoreWatchRequest) -> Result<StoreWatchReceipt, StoreError> {
            unreachable!("authorization must run before the store")
        }

        async fn resolve_ref(
            &self,
            _: StoreResolveRequest,
        ) -> Result<StoreResolvedIdentity, StoreError> {
            unreachable!("authorization must run before the store")
        }

        async fn inspect_schema(
            &self,
            _: StoreInspectSchemaRequest,
        ) -> Result<StoredSchema, StoreError> {
            unreachable!("authorization must run before the store")
        }

        async fn commit_verified(
            &self,
            _: VerifiedMutation,
        ) -> Result<StoreCommitResult, StoreError> {
            unreachable!("authorization must run before the store")
        }
    }

    fn subject(locality: Locality, evidence: EvidenceClass) -> Arc<SessionClaims> {
        subject_named(locality, evidence, "alice")
    }

    fn subject_named(
        locality: Locality,
        evidence: EvidenceClass,
        name: &str,
    ) -> Arc<SessionClaims> {
        Arc::new(SessionClaims::new(
            ResourceRef::parse(&format!("User/{name}")).unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceRef::parse("Zone/dev").unwrap(),
            evidence,
            SessionPurpose::parse("resource-api").unwrap(),
            ServiceName::parse("d2b.resource.v3").unwrap(),
            SessionBinding::new(
                SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
                TransportBinding::new(
                    locality,
                    BindingDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
                ),
                ReconnectGeneration::new(1).unwrap(),
                TranscriptHash::from_bytes([3; 32]),
            ),
        ))
    }

    fn state() -> AuthorizationState {
        AuthorizationState {
            snapshot: PolicySnapshot {
                policy_revision: 4,
                api_catalog_revision: 5,
                active_configuration_revision: ConfigurationGeneration::new(6).unwrap(),
                controller_generation: None,
            },
            zone_policy_revision: ZoneRevision::new(7),
            bootstrap_phase: BootstrapPhase::Disabled,
            now_tick: 1,
        }
    }

    fn denied_adapter()
    -> Arc<UnregisteredBusAdapter<UnreachableStore, crate::service::UnavailableUpgradeDispatcher>>
    {
        let (store, authorizer) = UnreachableStore::paired(ApiCatalog::standard(), None);
        let service = Arc::new(ResourceService::new(store, authorizer));
        Arc::new(
            UnregisteredBusAdapter::bind_unregistered_session(
                service,
                issue_test_subject(subject(Locality::Local, EvidenceClass::UnixPeer), state()),
            )
            .unwrap(),
        )
    }

    fn adapter_for(
        verb: ResourceVerb,
        named: bool,
        subresource: Option<&str>,
    ) -> Arc<UnregisteredBusAdapter<UnreachableStore, crate::service::UnavailableUpgradeDispatcher>>
    {
        let context = subject(Locality::Local, EvidenceClass::UnixPeer);
        let catalog = ApiCatalog::standard();
        let role = CompiledRole::new(
            ResourceRef::parse("Role/dispatch-test").unwrap(),
            vec![
                PolicyRule::new(
                    &catalog,
                    [ResourceTypeName::parse("Host").unwrap()],
                    [verb],
                    [],
                    subresource.into_iter().map(str::to_owned),
                    named
                        .then(|| ResourceName::parse("host-system").unwrap())
                        .into_iter(),
                    [ZoneId::parse("dev").unwrap()],
                    [],
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let binding = CompiledRoleBinding::new(
            role.role_ref.clone(),
            [BoundSubject {
                subject_ref: context.subject_ref().clone(),
                subject_uid: context.subject_uid().clone(),
            }],
            BindingScope::default(),
            RelayGrantAuthority::None,
        )
        .unwrap();
        let (store, authorizer) = UnreachableStore::paired(
            catalog.clone(),
            Some(PolicySet::new(&catalog, 4, vec![role], vec![binding]).unwrap()),
        );
        let service = Arc::new(ResourceService::new(store, authorizer));
        Arc::new(
            UnregisteredBusAdapter::bind_unregistered_session(
                service,
                issue_test_subject(context, state()),
            )
            .unwrap(),
        )
    }

    fn context() -> ttrpc::r#async::TtrpcContext {
        ttrpc::r#async::TtrpcContext {
            mh: Default::default(),
            metadata: HashMap::new(),
            timeout_nano: 0,
        }
    }

    fn identity() -> MessageField<wire::ResourceIdentity> {
        let mut identity = wire::ResourceIdentity::new();
        identity.zone = "dev".to_owned();
        identity.resource_type = "Host".to_owned();
        identity.name = "host-system".to_owned();
        MessageField::some(identity)
    }

    fn mutation(kind: wire::MutationKind) -> wire::Mutation {
        let mut mutation = wire::Mutation::new();
        mutation.kind = EnumOrUnknown::new(kind);
        mutation.target = identity();
        mutation
    }

    #[test]
    fn unregistered_service_map_contains_the_exact_thirteen_method_surface() {
        assert_eq!(
            RESOURCE_API_REACHABILITY,
            ResourceApiReachability::AwaitingAuthenticatedComponentSessionRouter
        );
        let services = denied_adapter().unregistered_ttrpc_services();
        assert_eq!(services.len(), 1);
        let methods = &services["d2b.resource.v3.ResourceService"].methods;
        let actual = methods.keys().cloned().collect::<BTreeSet<_>>();
        let expected = [
            "CommitBatch",
            "Create",
            "Delete",
            "Get",
            "InspectSchema",
            "List",
            "ResolveRef",
            "UpdateFinalizers",
            "UpdateMetadata",
            "UpdateSpec",
            "UpdateStatus",
            "Upgrade",
            "Watch",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn adapter_and_client_debug_redact_session_identity() {
        const MARKER: &str = "sentinel-observability-marker";

        let (store, authorizer) = UnreachableStore::paired(ApiCatalog::standard(), None);
        let service = Arc::new(ResourceService::new(store, authorizer));
        let adapter = UnregisteredBusAdapter::bind_unregistered_session(
            service,
            issue_test_subject(
                subject_named(Locality::Local, EvidenceClass::UnixPeer, MARKER),
                state(),
            ),
        )
        .unwrap();
        let adapter_debug = format!("{adapter:?}");
        let client_debug = format!("{:?}", adapter.unregistered_client());

        assert!(!adapter_debug.contains(MARKER), "{adapter_debug}");
        assert!(!client_debug.contains(MARKER), "{client_debug}");
    }

    #[tokio::test]
    async fn unregistered_thirteen_method_adapter_pins_targets_and_errors() {
        let ctx = context();
        let mut dispatched = 0;
        macro_rules! schema_rejected {
            ($call:expr) => {{
                let response = $call.await.unwrap();
                assert_eq!(
                    response.error.as_ref().unwrap().kind.enum_value().unwrap(),
                    wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_SCHEMA_INVALID
                );
                dispatched += 1;
            }};
        }

        let adapter = adapter_for(ResourceVerb::Get, true, None);
        let mut get = wire::GetRequest::new();
        get.target = identity();
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::get(
            &*adapter, &ctx, get
        ));

        let adapter = adapter_for(ResourceVerb::List, false, None);
        let mut list = wire::ListRequest::new();
        list.resource_types.push("Host".to_owned());
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::list(
            &*adapter, &ctx, list
        ));

        let adapter = adapter_for(ResourceVerb::Watch, false, None);
        let mut watch = wire::WatchRequest::new();
        watch.resource_types.push("Host".to_owned());
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::watch(
            &*adapter, &ctx, watch
        ));

        let adapter = adapter_for(ResourceVerb::Create, true, None);
        let mut create = wire::CreateRequest::new();
        create.mutation = MessageField::some(mutation(wire::MutationKind::MUTATION_KIND_CREATE));
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::create(
            &*adapter, &ctx, create
        ));

        let adapter = adapter_for(ResourceVerb::UpdateSpec, true, None);
        let mut update_spec = wire::UpdateSpecRequest::new();
        update_spec.mutation =
            MessageField::some(mutation(wire::MutationKind::MUTATION_KIND_UPDATE_SPEC));
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::update_spec(
            &*adapter,
            &ctx,
            update_spec
        ));

        let adapter = adapter_for(ResourceVerb::UpdateStatus, true, Some("status"));
        let mut update_status = wire::UpdateStatusRequest::new();
        update_status.mutation =
            MessageField::some(mutation(wire::MutationKind::MUTATION_KIND_UPDATE_STATUS));
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::update_status(
            &*adapter,
            &ctx,
            update_status
        ));

        let adapter = adapter_for(ResourceVerb::UpdateMetadata, true, None);
        let mut update_metadata = wire::UpdateMetadataRequest::new();
        update_metadata.mutation =
            MessageField::some(mutation(wire::MutationKind::MUTATION_KIND_UPDATE_METADATA));
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::update_metadata(
            &*adapter,
            &ctx,
            update_metadata
        ));

        let adapter = adapter_for(ResourceVerb::UpdateFinalizers, true, Some("finalizers"));
        let mut update_finalizers = wire::UpdateFinalizersRequest::new();
        update_finalizers.mutation = MessageField::some(mutation(
            wire::MutationKind::MUTATION_KIND_UPDATE_FINALIZERS,
        ));
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::update_finalizers(
            &*adapter,
            &ctx,
            update_finalizers
        ));

        let adapter = adapter_for(ResourceVerb::Delete, true, None);
        let mut delete = wire::DeleteRequest::new();
        delete.mutation = MessageField::some(mutation(wire::MutationKind::MUTATION_KIND_DELETE));
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::delete(
            &*adapter, &ctx, delete
        ));

        let adapter = adapter_for(ResourceVerb::Delete, true, None);
        let mut batch = wire::CommitBatchRequest::new();
        batch
            .mutations
            .push(mutation(wire::MutationKind::MUTATION_KIND_DELETE));
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::commit_batch(
            &*adapter, &ctx, batch
        ));

        let adapter = adapter_for(ResourceVerb::Get, true, None);
        let mut resolve = wire::ResolveRefRequest::new();
        resolve.target = identity();
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::resolve_ref(
            &*adapter, &ctx, resolve
        ));

        let adapter = adapter_for(ResourceVerb::Get, false, Some("schema"));
        let mut inspect = wire::InspectSchemaRequest::new();
        inspect.resource_type = "Host".to_owned();
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::inspect_schema(
            &*adapter, &ctx, inspect
        ));

        let adapter = adapter_for(ResourceVerb::UpdateSpec, true, None);
        let mut upgrade = wire::UpgradeRequest::new();
        upgrade.target = identity();
        schema_rejected!(d2b_resource_v3_ttrpc::ResourceService::upgrade(
            &*adapter, &ctx, upgrade
        ));

        assert_eq!(dispatched, 13);
    }

    #[test]
    fn adapter_rejects_locality_evidence_mismatches() {
        let (store, authorizer) = UnreachableStore::paired(ApiCatalog::standard(), None);
        let service = Arc::new(ResourceService::new(store, authorizer));
        for (locality, evidence) in [
            (Locality::AdjacentZone, EvidenceClass::BootstrapIkpsk2),
            (Locality::Remote, EvidenceClass::EnrolledKk),
        ] {
            assert!(
                UnregisteredBusAdapter::bind_unregistered_session(
                    Arc::clone(&service),
                    issue_test_subject(subject(locality, evidence), state()),
                )
                .is_err()
            );
        }
    }
}
