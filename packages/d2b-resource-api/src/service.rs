//! Async resource methods and admission ordering.

use std::{future::Future, sync::Arc};

use d2b_contracts::{
    resource_proto as wire,
    v3::{
        AuthenticatedSubjectContext, DEFAULT_LIST_PAGE_SIZE, DEFAULT_REQUEST_DEADLINE_MS,
        DEFAULT_WATCH_CREDITS, FinalizerId, MAX_BATCH_MUTATIONS, MAX_EXPEDITED_DEADLINE_MS,
        MAX_FILTER_VALUES, MAX_LIST_FILTERS, MAX_LIST_PAGE_SIZE, MAX_LIST_RESOURCE_TYPES,
        MAX_PAGE_CURSOR_BYTES, MAX_REQUEST_CANONICAL_BYTES, MAX_REQUEST_DEADLINE_MS,
        MAX_RESPONSE_CANONICAL_BYTES, MAX_WATCH_CREDITS, MAX_WATCH_FILTERS,
        MAX_WATCH_RESOURCE_TYPES, ResourceEnvelope, ResourceError, ResourceErrorKind, ResourceName,
        ResourceRef, ResourceTypeName, ResourceUid, ZoneId, ZoneRevision,
    },
};
use d2b_resource_store::{
    AdmittedMutation, ExpectedRevision, ResourceMutationKind, ResourceStore, StoreCommitResult,
    StoreFilter, StoreGetRequest, StoreInspectSchemaRequest, StoreListRequest, StoreMutation,
    StoreOperationContext, StoreProjection, StoreResolveRequest, StoreWatchRequest, StoredResource,
};
use protobuf::{Message, MessageField};

use crate::{
    authz::{
        ApiMethod, AuthorizationRequest, AuthorizationState, AuthorizationTarget, NativeAuthorizer,
        ResourceVerb,
    },
    error::{map_store_error, map_store_error_with_revision_visibility, to_wire_error},
};

/// Trusted envelope created only after ComponentSession authentication.
#[derive(Clone)]
pub struct TrustedRequest<T> {
    subject: Arc<AuthenticatedSubjectContext>,
    authorization_state: AuthorizationState,
    relay_hop: bool,
    request: T,
}

impl<T> TrustedRequest<T> {
    /// Bind a decoded request to authenticated session and live policy state.
    pub fn from_component_session(
        subject: Arc<AuthenticatedSubjectContext>,
        authorization_state: AuthorizationState,
        relay_hop: bool,
        request: T,
    ) -> Self {
        Self {
            subject,
            authorization_state,
            relay_hop,
            request,
        }
    }

    pub const fn request(&self) -> &T {
        &self.request
    }
}

impl<T> core::fmt::Debug for TrustedRequest<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TrustedRequest(<redacted>)")
    }
}

/// Controller-owned upgrade dispatch seam.
pub trait UpgradeDispatcher: Send + Sync {
    fn dispatch(
        &self,
        request: AuthorizedUpgrade,
    ) -> impl Future<Output = Result<UpgradeResult, ResourceError>> + Send;
}

/// Authorized upgrade request passed to the owning controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedUpgrade {
    pub operation: StoreOperationContext,
    pub zone: ZoneId,
    pub target: ResourceRef,
    pub action: UpgradeAction,
    pub recursive: bool,
    pub expected_revision: ZoneRevision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeAction {
    Assess,
    Plan,
    Execute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeResult {
    pub resource: StoredResource,
    pub plan: Vec<d2b_resource_store::StoreResolvedIdentity>,
    pub revision: ZoneRevision,
}

/// Default until the controller dispatch slice lands.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableUpgradeDispatcher;

impl UpgradeDispatcher for UnavailableUpgradeDispatcher {
    async fn dispatch(&self, _request: AuthorizedUpgrade) -> Result<UpgradeResult, ResourceError> {
        Err(ResourceError::terminal(
            ResourceErrorKind::ResourceProviderUnavailable,
            "upgrade controller is unavailable",
        ))
    }
}

/// Resource API over one concrete store and one native authorization engine.
#[derive(Debug)]
pub struct ResourceService<S, U = UnavailableUpgradeDispatcher> {
    store: Arc<S>,
    authorizer: Arc<NativeAuthorizer>,
    upgrade: Arc<U>,
}

impl<S> ResourceService<S, UnavailableUpgradeDispatcher> {
    pub fn new(store: Arc<S>, authorizer: Arc<NativeAuthorizer>) -> Self {
        Self {
            store,
            authorizer,
            upgrade: Arc::new(UnavailableUpgradeDispatcher),
        }
    }
}

impl<S, U> ResourceService<S, U> {
    pub fn with_upgrade(store: Arc<S>, authorizer: Arc<NativeAuthorizer>, upgrade: Arc<U>) -> Self {
        Self {
            store,
            authorizer,
            upgrade,
        }
    }
}

impl<S, U> ResourceService<S, U>
where
    S: ResourceStore,
    U: UpgradeDispatcher,
{
    pub async fn get(&self, trusted: TrustedRequest<wire::GetRequest>) -> wire::GetResponse {
        let identity = match parse_identity(trusted.request.target.as_ref()) {
            Ok(identity) => identity,
            Err(error) => return get_error(error),
        };
        let auth = authorization_for_identity(
            ApiMethod::Get,
            ResourceVerb::Get,
            &identity,
            trusted.relay_hop,
        );
        if let Err(error) = self.authorize(&trusted, auth) {
            return get_error(error);
        }
        if let Err(error) = validate_request(&trusted.request) {
            return get_error(error);
        }
        let operation = match operation_context(
            trusted.request.meta.as_ref(),
            false,
            &trusted.authorization_state,
        ) {
            Ok(operation) => operation,
            Err(error) => return get_error(error),
        };
        let projection = match parse_projection(trusted.request.projection.as_ref()) {
            Ok(projection) => projection,
            Err(error) => return get_error(error),
        };
        match self
            .store
            .get(StoreGetRequest {
                operation,
                zone: identity.zone,
                target: identity.resource_ref,
                expected_uid: identity.uid,
                projection,
            })
            .await
        {
            Ok(resource) if resource.canonical_json.len() <= MAX_RESPONSE_CANONICAL_BYTES => {
                let mut response = wire::GetResponse::new();
                response.resource = MessageField::some(to_wire_resource(resource));
                response
            }
            Ok(_) => get_error(schema_error("resource response exceeds its byte bound")),
            Err(error) => get_error(map_store_error(error)),
        }
    }

    pub async fn list(&self, trusted: TrustedRequest<wire::ListRequest>) -> wire::ListResponse {
        let targets = match parse_collection_targets(
            &trusted.request.resource_types,
            &trusted.request.filters,
            ResourceVerb::List,
        ) {
            Ok(targets) => targets,
            Err(error) => return list_error(error),
        };
        let auth = AuthorizationRequest {
            method: ApiMethod::List,
            zone: subject_zone(&trusted),
            targets,
            relay_hop: trusted.relay_hop,
        };
        if let Err(error) = self.authorize(&trusted, auth) {
            return list_error(error);
        }
        if let Err(error) = validate_request(&trusted.request) {
            return list_error(error);
        }
        let operation = match operation_context(
            trusted.request.meta.as_ref(),
            false,
            &trusted.authorization_state,
        ) {
            Ok(operation) => operation,
            Err(error) => return list_error(error),
        };
        let parsed = match parse_collection_request(
            &trusted.request.resource_types,
            &trusted.request.filters,
            MAX_LIST_RESOURCE_TYPES,
            MAX_LIST_FILTERS,
        ) {
            Ok(parsed) => parsed,
            Err(error) => return list_error(error),
        };
        let page_size = if trusted.request.page_size == 0 {
            DEFAULT_LIST_PAGE_SIZE
        } else {
            trusted.request.page_size
        };
        if page_size > MAX_LIST_PAGE_SIZE {
            return list_error(schema_error("page size exceeds its bound"));
        }
        let cursor = trusted
            .request
            .cursor
            .as_ref()
            .map(|cursor| cursor.value.clone())
            .filter(|cursor| !cursor.is_empty());
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.len() > MAX_PAGE_CURSOR_BYTES)
        {
            return list_error(schema_error("page cursor exceeds its bound"));
        }
        let projection = match parse_projection(trusted.request.projection.as_ref()) {
            Ok(projection) => projection,
            Err(error) => return list_error(error),
        };
        match self
            .store
            .list(StoreListRequest {
                operation,
                zone: subject_zone(&trusted),
                resource_types: parsed.resource_types,
                resource_names: parsed.resource_names,
                filters: parsed.filters,
                page_size,
                cursor,
                projection,
            })
            .await
        {
            Ok(result) => {
                let mut response = wire::ListResponse::new();
                response.resources = result.resources.into_iter().map(to_wire_resource).collect();
                response.snapshot_revision = result.snapshot_revision.get();
                if let Some(cursor) = result.next_cursor {
                    let mut page = wire::PageCursor::new();
                    page.value = cursor;
                    response.next_cursor = MessageField::some(page);
                }
                response.truncated = result.truncated;
                if response.compute_size() as usize > MAX_RESPONSE_CANONICAL_BYTES {
                    list_error(schema_error(
                        "list store result was not truncated at the byte bound",
                    ))
                } else {
                    response
                }
            }
            Err(error) => list_error(map_store_error(error)),
        }
    }

    pub async fn watch(&self, trusted: TrustedRequest<wire::WatchRequest>) -> wire::WatchResponse {
        let targets = match parse_collection_targets(
            &trusted.request.resource_types,
            &trusted.request.filters,
            ResourceVerb::Watch,
        ) {
            Ok(targets) => targets,
            Err(error) => return watch_error(error),
        };
        let auth = AuthorizationRequest {
            method: ApiMethod::Watch,
            zone: subject_zone(&trusted),
            targets,
            relay_hop: trusted.relay_hop,
        };
        if let Err(error) = self.authorize(&trusted, auth) {
            return watch_error(error);
        }
        if let Err(error) = validate_request(&trusted.request) {
            return watch_error(error);
        }
        let operation = match operation_context(
            trusted.request.meta.as_ref(),
            false,
            &trusted.authorization_state,
        ) {
            Ok(operation) => operation,
            Err(error) => return watch_error(error),
        };
        let parsed = match parse_collection_request(
            &trusted.request.resource_types,
            &trusted.request.filters,
            MAX_WATCH_RESOURCE_TYPES,
            MAX_WATCH_FILTERS,
        ) {
            Ok(parsed) => parsed,
            Err(error) => return watch_error(error),
        };
        let credits = trusted
            .request
            .credits
            .as_ref()
            .map_or(DEFAULT_WATCH_CREDITS, |credits| credits.initial);
        if credits == 0 || credits > MAX_WATCH_CREDITS {
            return watch_error(schema_error("watch credits exceed their bound"));
        }
        let projection = match parse_projection(trusted.request.projection.as_ref()) {
            Ok(projection) => projection,
            Err(error) => return watch_error(error),
        };
        match self
            .store
            .watch(StoreWatchRequest {
                operation,
                zone: subject_zone(&trusted),
                resource_types: parsed.resource_types,
                resource_names: parsed.resource_names,
                filters: parsed.filters,
                after_revision: ZoneRevision::new(trusted.request.after_revision),
                initial_credits: credits,
                projection,
            })
            .await
        {
            Ok(receipt) => {
                let mut response = wire::WatchResponse::new();
                response.stream_name = receipt.stream_name;
                response.snapshot_revision = receipt.snapshot_revision.get();
                response
            }
            Err(error) => watch_error(map_store_error(error)),
        }
    }

    pub async fn create(
        &self,
        trusted: TrustedRequest<wire::CreateRequest>,
    ) -> wire::CreateResponse {
        match self
            .commit_one(&trusted, ApiMethod::Create, ResourceMutationKind::Create)
            .await
        {
            Ok(result) => mutation_response(result, trusted.request.mutation.as_ref(), true),
            Err(error) => create_error(error),
        }
    }

    pub async fn update_spec(
        &self,
        trusted: TrustedRequest<wire::UpdateSpecRequest>,
    ) -> wire::UpdateSpecResponse {
        match self
            .commit_one(
                &trusted,
                ApiMethod::UpdateSpec,
                ResourceMutationKind::UpdateSpec,
            )
            .await
        {
            Ok(result) => {
                let common = mutation_response(result, trusted.request.mutation.as_ref(), true);
                copy_update_spec_response(common)
            }
            Err(error) => update_spec_error(error),
        }
    }

    pub async fn update_status(
        &self,
        trusted: TrustedRequest<wire::UpdateStatusRequest>,
    ) -> wire::UpdateStatusResponse {
        match self
            .commit_one(
                &trusted,
                ApiMethod::UpdateStatus,
                ResourceMutationKind::UpdateStatus,
            )
            .await
        {
            Ok(result) => copy_update_status_response(mutation_response(
                result,
                trusted.request.mutation.as_ref(),
                false,
            )),
            Err(error) => update_status_error(error),
        }
    }

    pub async fn update_metadata(
        &self,
        trusted: TrustedRequest<wire::UpdateMetadataRequest>,
    ) -> wire::UpdateMetadataResponse {
        match self
            .commit_one(
                &trusted,
                ApiMethod::UpdateMetadata,
                ResourceMutationKind::UpdateMetadata,
            )
            .await
        {
            Ok(result) => copy_update_metadata_response(mutation_response(
                result,
                trusted.request.mutation.as_ref(),
                false,
            )),
            Err(error) => update_metadata_error(error),
        }
    }

    pub async fn update_finalizers(
        &self,
        trusted: TrustedRequest<wire::UpdateFinalizersRequest>,
    ) -> wire::UpdateFinalizersResponse {
        match self
            .commit_one(
                &trusted,
                ApiMethod::UpdateFinalizers,
                ResourceMutationKind::UpdateFinalizers,
            )
            .await
        {
            Ok(result) => copy_update_finalizers_response(mutation_response(
                result,
                trusted.request.mutation.as_ref(),
                false,
            )),
            Err(error) => update_finalizers_error(error),
        }
    }

    pub async fn delete(
        &self,
        trusted: TrustedRequest<wire::DeleteRequest>,
    ) -> wire::DeleteResponse {
        match self
            .commit_one(&trusted, ApiMethod::Delete, ResourceMutationKind::Delete)
            .await
        {
            Ok(result) => {
                let mut response = wire::DeleteResponse::new();
                response.revision = result.revision.get();
                if let Some(resource) = result.resources.into_iter().next() {
                    response.resource = MessageField::some(to_wire_identity(&resource));
                }
                if trusted
                    .request
                    .mutation
                    .as_ref()
                    .is_some_and(|mutation| mutation.wait_for_reconcile)
                {
                    response.error = MessageField::some(to_wire_error(&ResourceError::terminal(
                        ResourceErrorKind::ExpeditedReconcilePending,
                        "resource committed and reconcile remains pending",
                    )));
                }
                response
            }
            Err(error) => delete_error(error),
        }
    }

    pub async fn commit_batch(
        &self,
        trusted: TrustedRequest<wire::CommitBatchRequest>,
    ) -> wire::CommitBatchResponse {
        if trusted.request.mutations.is_empty() {
            return batch_error(schema_error("batch mutation count exceeds its bound"));
        }
        let routes = match trusted
            .request
            .mutations
            .iter()
            .map(|mutation| parse_mutation_route(mutation, None, &trusted))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(routes) => routes,
            Err(error) => return batch_error(error),
        };
        let auth = AuthorizationRequest {
            method: ApiMethod::CommitBatch,
            zone: subject_zone(&trusted),
            targets: routes
                .iter()
                .flat_map(|item| item.authorizations.iter().cloned())
                .collect(),
            relay_hop: trusted.relay_hop,
        };
        let grant = match self.authorize(&trusted, auth) {
            Ok(grant) => grant,
            Err(error) => return batch_error(error),
        };
        if let Err(error) = validate_request(&trusted.request) {
            return batch_error(error);
        }
        if trusted.request.mutations.len() > MAX_BATCH_MUTATIONS {
            return batch_error(schema_error("batch mutation count exceeds its bound"));
        }
        let parsed = match trusted
            .request
            .mutations
            .iter()
            .zip(&routes)
            .map(|(mutation, route)| parse_mutation(mutation, route, &trusted))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(parsed) => parsed,
            Err(error) => return batch_error(error),
        };
        let operation = match operation_context(
            trusted.request.meta.as_ref(),
            true,
            &trusted.authorization_state,
        ) {
            Ok(operation) => operation,
            Err(error) => return batch_error(error),
        };
        let admitted = AdmittedMutation::new(
            parsed.into_iter().map(|item| item.store).collect(),
            grant.admitted,
            trusted.authorization_state.snapshot,
            operation,
            grant.allow,
        );
        match self.store.commit(admitted).await {
            Ok(result) => {
                let mut response = wire::CommitBatchResponse::new();
                response.resources = result.resources.into_iter().map(to_wire_resource).collect();
                response.revision = result.revision.get();
                response
            }
            Err(error) => batch_error(map_store_error_with_revision_visibility(
                error,
                self.can_read_revision(&trusted, &routes),
            )),
        }
    }

    pub async fn resolve_ref(
        &self,
        trusted: TrustedRequest<wire::ResolveRefRequest>,
    ) -> wire::ResolveRefResponse {
        let identity = match parse_identity(trusted.request.target.as_ref()) {
            Ok(identity) => identity,
            Err(error) => return resolve_error(error),
        };
        if let Err(error) = self.authorize(
            &trusted,
            authorization_for_identity(
                ApiMethod::ResolveRef,
                ResourceVerb::Get,
                &identity,
                trusted.relay_hop,
            ),
        ) {
            return resolve_error(error);
        }
        if let Err(error) = validate_request(&trusted.request) {
            return resolve_error(error);
        }
        let operation = match operation_context(
            trusted.request.meta.as_ref(),
            false,
            &trusted.authorization_state,
        ) {
            Ok(operation) => operation,
            Err(error) => return resolve_error(error),
        };
        match self
            .store
            .resolve_ref(StoreResolveRequest {
                operation,
                zone: identity.zone,
                target: identity.resource_ref,
                expected_uid: identity.uid,
            })
            .await
        {
            Ok(identity) => {
                let mut response = wire::ResolveRefResponse::new();
                response.resource = MessageField::some(to_wire_resolved_identity(identity));
                response
            }
            Err(error) => resolve_error(map_store_error(error)),
        }
    }

    pub async fn inspect_schema(
        &self,
        trusted: TrustedRequest<wire::InspectSchemaRequest>,
    ) -> wire::InspectSchemaResponse {
        let resource_type = match ResourceTypeName::parse(&trusted.request.resource_type) {
            Ok(resource_type) => resource_type,
            Err(_) => return inspect_error(ref_error("ResourceType is invalid")),
        };
        let auth = AuthorizationRequest {
            method: ApiMethod::InspectSchema,
            zone: subject_zone(&trusted),
            targets: vec![AuthorizationTarget {
                resource_type: resource_type.clone(),
                resource_name: None,
                verb: ResourceVerb::Get,
                subresource: Some("schema".to_owned()),
                execution_ref: None,
            }],
            relay_hop: trusted.relay_hop,
        };
        if let Err(error) = self.authorize(&trusted, auth) {
            return inspect_error(error);
        }
        if let Err(error) = validate_request(&trusted.request) {
            return inspect_error(error);
        }
        let operation = match operation_context(
            trusted.request.meta.as_ref(),
            false,
            &trusted.authorization_state,
        ) {
            Ok(operation) => operation,
            Err(error) => return inspect_error(error),
        };
        match self
            .store
            .inspect_schema(StoreInspectSchemaRequest {
                operation,
                zone: subject_zone(&trusted),
                resource_type,
            })
            .await
        {
            Ok(schema) => {
                let mut identity = wire::ResourceIdentity::new();
                identity.zone = subject_zone(&trusted).to_string();
                identity.resource_type = schema.resource_type.to_string();
                let mut body = wire::ResourceEnvelopeBytes::new();
                body.identity = MessageField::some(identity);
                body.canonical_json = schema.canonical_json;
                body.payload_digest = schema.payload_digest;
                let mut response = wire::InspectSchemaResponse::new();
                response.schema = MessageField::some(body);
                response
            }
            Err(error) => inspect_error(map_store_error(error)),
        }
    }

    pub async fn upgrade(
        &self,
        trusted: TrustedRequest<wire::UpgradeRequest>,
    ) -> wire::UpgradeResponse {
        let identity = match parse_identity(trusted.request.target.as_ref()) {
            Ok(identity) => identity,
            Err(error) => return upgrade_error(error),
        };
        let auth = authorization_for_identity(
            ApiMethod::Upgrade,
            ResourceVerb::UpdateSpec,
            &identity,
            trusted.relay_hop,
        );
        if let Err(error) = self.authorize(&trusted, auth) {
            return upgrade_error(error);
        }
        if let Err(error) = validate_request(&trusted.request) {
            return upgrade_error(error);
        }
        let operation = match operation_context(
            trusted.request.meta.as_ref(),
            false,
            &trusted.authorization_state,
        ) {
            Ok(operation) => operation,
            Err(error) => return upgrade_error(error),
        };
        let expected_revision = match parse_precondition(trusted.request.precondition.as_ref()) {
            Ok(ExpectedRevision::Exact(revision)) => revision,
            _ => return upgrade_error(schema_error("upgrade requires an exact revision")),
        };
        let action = match trusted.request.action.enum_value() {
            Ok(wire::UpgradeAction::UPGRADE_ACTION_ASSESS) => UpgradeAction::Assess,
            Ok(wire::UpgradeAction::UPGRADE_ACTION_PLAN) => UpgradeAction::Plan,
            Ok(wire::UpgradeAction::UPGRADE_ACTION_EXECUTE) => UpgradeAction::Execute,
            _ => return upgrade_error(schema_error("upgrade action is unspecified")),
        };
        match self
            .upgrade
            .dispatch(AuthorizedUpgrade {
                operation,
                zone: identity.zone,
                target: identity.resource_ref,
                action,
                recursive: trusted.request.recursive,
                expected_revision,
            })
            .await
        {
            Ok(result) => {
                let mut response = wire::UpgradeResponse::new();
                response.resource = MessageField::some(to_wire_resource(result.resource));
                response.plan = result
                    .plan
                    .into_iter()
                    .map(to_wire_resolved_identity)
                    .collect();
                response.revision = result.revision.get();
                response
            }
            Err(error) => upgrade_error(error),
        }
    }

    async fn commit_one<T>(
        &self,
        trusted: &TrustedRequest<T>,
        method: ApiMethod,
        expected_kind: ResourceMutationKind,
    ) -> Result<StoreCommitResult, ResourceError>
    where
        T: MutationRequest + StrictResourceRequest,
    {
        let mutation = trusted
            .mutation()
            .ok_or_else(|| schema_error("mutation is required"))?;
        let route = parse_mutation_route(mutation, Some(expected_kind), trusted)?;
        let grant = self.authorize(
            trusted,
            AuthorizationRequest {
                method,
                zone: route.identity.zone.clone(),
                targets: route.authorizations.clone(),
                relay_hop: trusted.relay_hop,
            },
        )?;
        validate_request(&trusted.request)?;
        let parsed = parse_mutation(mutation, &route, trusted)?;
        let operation = operation_context(trusted.meta(), true, &trusted.authorization_state)?;
        match self
            .store
            .commit(AdmittedMutation::new(
                vec![parsed.store],
                grant.admitted,
                trusted.authorization_state.snapshot,
                operation,
                grant.allow,
            ))
            .await
        {
            Ok(result) => Ok(result),
            Err(error) => Err(map_store_error_with_revision_visibility(
                error,
                self.can_read_revision(trusted, std::slice::from_ref(&route)),
            )),
        }
    }

    fn authorize<T>(
        &self,
        trusted: &TrustedRequest<T>,
        request: AuthorizationRequest,
    ) -> Result<crate::authz::AuthorizationGrant, ResourceError> {
        self.authorizer
            .authorize(&trusted.subject, &request, &trusted.authorization_state)
            .map_err(|denial| {
                ResourceError::terminal(
                    denial.resource_error_kind(),
                    match denial.resource_error_kind() {
                        ResourceErrorKind::RelayDenied => "relay authorization denied",
                        _ => "resource authorization denied",
                    },
                )
            })
    }

    fn can_read_revision<T>(
        &self,
        trusted: &TrustedRequest<T>,
        routes: &[ParsedMutationRoute],
    ) -> bool {
        self.authorizer
            .authorize(
                &trusted.subject,
                &AuthorizationRequest {
                    method: ApiMethod::Get,
                    zone: subject_zone(trusted),
                    targets: routes
                        .iter()
                        .map(|route| AuthorizationTarget {
                            resource_type: route.identity.resource_ref.resource_type().clone(),
                            resource_name: Some(route.identity.resource_ref.name().clone()),
                            verb: ResourceVerb::Get,
                            subresource: None,
                            execution_ref: trusted.subject.execution_ref().cloned(),
                        })
                        .collect(),
                    relay_hop: trusted.relay_hop,
                },
                &trusted.authorization_state,
            )
            .is_ok()
    }
}

trait MutationRequest {
    fn meta(&self) -> Option<&wire::RequestMeta>;
    fn mutation(&self) -> Option<&wire::Mutation>;
}

trait StrictResourceRequest: Message {
    fn has_unknown_fields(&self) -> bool;
}

macro_rules! impl_mutation_request {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl MutationRequest for $ty {
                fn meta(&self) -> Option<&wire::RequestMeta> {
                    self.meta.as_ref()
                }

                fn mutation(&self) -> Option<&wire::Mutation> {
                    self.mutation.as_ref()
                }
            }
        )+
    };
}

impl_mutation_request!(
    wire::CreateRequest,
    wire::UpdateSpecRequest,
    wire::UpdateStatusRequest,
    wire::UpdateMetadataRequest,
    wire::UpdateFinalizersRequest,
    wire::DeleteRequest,
);

fn has_unknown<M: Message>(message: &M) -> bool {
    message
        .special_fields()
        .unknown_fields()
        .iter()
        .next()
        .is_some()
}

fn field_has_unknown<M: Message>(field: &MessageField<M>) -> bool {
    field.as_ref().is_some_and(has_unknown)
}

fn identity_has_unknown(field: &MessageField<wire::ResourceIdentity>) -> bool {
    field_has_unknown(field)
}

fn meta_has_unknown(field: &MessageField<wire::RequestMeta>) -> bool {
    field_has_unknown(field)
}

fn envelope_has_unknown(field: &MessageField<wire::ResourceEnvelopeBytes>) -> bool {
    field
        .as_ref()
        .is_some_and(|value| has_unknown(value) || identity_has_unknown(&value.identity))
}

fn mutation_has_unknown(value: &wire::Mutation) -> bool {
    has_unknown(value)
        || identity_has_unknown(&value.target)
        || field_has_unknown(&value.precondition)
        || envelope_has_unknown(&value.resource)
        || identity_has_unknown(&value.owner)
}

macro_rules! impl_strict_mutation_request {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl StrictResourceRequest for $ty {
                fn has_unknown_fields(&self) -> bool {
                    has_unknown(self)
                        || meta_has_unknown(&self.meta)
                        || self.mutation.as_ref().is_some_and(mutation_has_unknown)
                }
            }
        )+
    };
}

impl_strict_mutation_request!(
    wire::CreateRequest,
    wire::UpdateSpecRequest,
    wire::UpdateStatusRequest,
    wire::UpdateMetadataRequest,
    wire::UpdateFinalizersRequest,
    wire::DeleteRequest,
);

impl StrictResourceRequest for wire::GetRequest {
    fn has_unknown_fields(&self) -> bool {
        has_unknown(self)
            || meta_has_unknown(&self.meta)
            || identity_has_unknown(&self.target)
            || field_has_unknown(&self.projection)
    }
}

impl StrictResourceRequest for wire::ListRequest {
    fn has_unknown_fields(&self) -> bool {
        has_unknown(self)
            || meta_has_unknown(&self.meta)
            || self.filters.iter().any(has_unknown)
            || field_has_unknown(&self.cursor)
            || field_has_unknown(&self.projection)
    }
}

impl StrictResourceRequest for wire::WatchRequest {
    fn has_unknown_fields(&self) -> bool {
        has_unknown(self)
            || meta_has_unknown(&self.meta)
            || self.filters.iter().any(has_unknown)
            || field_has_unknown(&self.credits)
            || field_has_unknown(&self.projection)
    }
}

impl StrictResourceRequest for wire::CommitBatchRequest {
    fn has_unknown_fields(&self) -> bool {
        has_unknown(self)
            || meta_has_unknown(&self.meta)
            || self.mutations.iter().any(mutation_has_unknown)
    }
}

impl StrictResourceRequest for wire::ResolveRefRequest {
    fn has_unknown_fields(&self) -> bool {
        has_unknown(self) || meta_has_unknown(&self.meta) || identity_has_unknown(&self.target)
    }
}

impl StrictResourceRequest for wire::InspectSchemaRequest {
    fn has_unknown_fields(&self) -> bool {
        has_unknown(self) || meta_has_unknown(&self.meta)
    }
}

impl StrictResourceRequest for wire::UpgradeRequest {
    fn has_unknown_fields(&self) -> bool {
        has_unknown(self)
            || meta_has_unknown(&self.meta)
            || identity_has_unknown(&self.target)
            || field_has_unknown(&self.precondition)
    }
}

impl<T> TrustedRequest<T> {
    fn meta(&self) -> Option<&wire::RequestMeta>
    where
        T: MutationRequest,
    {
        self.request.meta()
    }

    fn mutation(&self) -> Option<&wire::Mutation>
    where
        T: MutationRequest,
    {
        self.request.mutation()
    }
}

#[derive(Debug)]
struct ParsedIdentity {
    zone: ZoneId,
    resource_ref: ResourceRef,
    uid: Option<ResourceUid>,
}

#[derive(Debug)]
struct ParsedMutation {
    store: StoreMutation,
}

#[derive(Debug)]
struct ParsedMutationRoute {
    identity: ParsedIdentity,
    owner: Option<ParsedIdentity>,
    kind: ResourceMutationKind,
    authorizations: Vec<AuthorizationTarget>,
}

fn parse_identity(value: Option<&wire::ResourceIdentity>) -> Result<ParsedIdentity, ResourceError> {
    let value = value.ok_or_else(|| ref_error("resource identity is required"))?;
    let zone = ZoneId::parse(&value.zone).map_err(|_| ref_error("resource Zone is invalid"))?;
    let resource_type = ResourceTypeName::parse(&value.resource_type)
        .map_err(|_| ref_error("ResourceType is invalid"))?;
    let name =
        ResourceName::parse(&value.name).map_err(|_| ref_error("resource name is invalid"))?;
    let uid = value
        .uid
        .as_ref()
        .map(|value| ResourceUid::parse(value.as_str()))
        .transpose()
        .map_err(|_| ref_error("resource UID is invalid"))?;
    Ok(ParsedIdentity {
        zone,
        resource_ref: ResourceRef::new(resource_type, name),
        uid,
    })
}

fn authorization_for_identity(
    method: ApiMethod,
    verb: ResourceVerb,
    identity: &ParsedIdentity,
    relay_hop: bool,
) -> AuthorizationRequest {
    AuthorizationRequest {
        method,
        zone: identity.zone.clone(),
        targets: vec![AuthorizationTarget {
            resource_type: identity.resource_ref.resource_type().clone(),
            resource_name: Some(identity.resource_ref.name().clone()),
            verb,
            subresource: None,
            execution_ref: None,
        }],
        relay_hop,
    }
}

fn parse_mutation_route<T>(
    mutation: &wire::Mutation,
    expected_kind: Option<ResourceMutationKind>,
    trusted: &TrustedRequest<T>,
) -> Result<ParsedMutationRoute, ResourceError> {
    let identity = parse_identity(mutation.target.as_ref())?;
    let (kind, verb) = if let Some(kind) = expected_kind {
        (kind, mutation_verb(kind))
    } else {
        parse_mutation_kind(mutation)?
    };
    let owner = mutation
        .owner
        .as_ref()
        .map(|owner| parse_identity(Some(owner)))
        .transpose()?;
    let mut authorizations = vec![AuthorizationTarget {
        resource_type: identity.resource_ref.resource_type().clone(),
        resource_name: Some(identity.resource_ref.name().clone()),
        verb,
        subresource: match kind {
            ResourceMutationKind::UpdateStatus => Some("status".to_owned()),
            ResourceMutationKind::UpdateFinalizers => Some("finalizers".to_owned()),
            _ => None,
        },
        execution_ref: trusted.subject.execution_ref().cloned(),
    }];
    if identity.resource_ref.resource_type().as_str() == "Credential" {
        let subresource = match kind {
            ResourceMutationKind::Create => Some("create"),
            ResourceMutationKind::UpdateSpec => Some("update-spec"),
            ResourceMutationKind::Delete => Some("delete"),
            _ => None,
        };
        if let Some(subresource) = subresource {
            authorizations.push(AuthorizationTarget {
                resource_type: identity.resource_ref.resource_type().clone(),
                resource_name: Some(identity.resource_ref.name().clone()),
                verb: ResourceVerb::AdminCredential,
                subresource: Some(subresource.to_owned()),
                execution_ref: trusted.subject.execution_ref().cloned(),
            });
        }
    }
    if let Some(owner) = &owner {
        authorizations.push(AuthorizationTarget {
            resource_type: owner.resource_ref.resource_type().clone(),
            resource_name: Some(owner.resource_ref.name().clone()),
            verb: ResourceVerb::Get,
            subresource: Some("owner".to_owned()),
            execution_ref: trusted.subject.execution_ref().cloned(),
        });
    }
    Ok(ParsedMutationRoute {
        authorizations,
        identity,
        owner,
        kind,
    })
}

fn parse_mutation_kind(
    mutation: &wire::Mutation,
) -> Result<(ResourceMutationKind, ResourceVerb), ResourceError> {
    match mutation.kind.enum_value() {
        Ok(wire::MutationKind::MUTATION_KIND_CREATE) => {
            Ok((ResourceMutationKind::Create, ResourceVerb::Create))
        }
        Ok(wire::MutationKind::MUTATION_KIND_UPDATE_SPEC) => {
            Ok((ResourceMutationKind::UpdateSpec, ResourceVerb::UpdateSpec))
        }
        Ok(wire::MutationKind::MUTATION_KIND_UPDATE_STATUS) => Ok((
            ResourceMutationKind::UpdateStatus,
            ResourceVerb::UpdateStatus,
        )),
        Ok(wire::MutationKind::MUTATION_KIND_UPDATE_METADATA) => Ok((
            ResourceMutationKind::UpdateMetadata,
            ResourceVerb::UpdateMetadata,
        )),
        Ok(wire::MutationKind::MUTATION_KIND_UPDATE_FINALIZERS) => Ok((
            ResourceMutationKind::UpdateFinalizers,
            ResourceVerb::UpdateFinalizers,
        )),
        Ok(wire::MutationKind::MUTATION_KIND_DELETE) => {
            Ok((ResourceMutationKind::Delete, ResourceVerb::Delete))
        }
        _ => Err(schema_error("mutation kind is unspecified")),
    }
}

const fn mutation_verb(kind: ResourceMutationKind) -> ResourceVerb {
    match kind {
        ResourceMutationKind::Create => ResourceVerb::Create,
        ResourceMutationKind::UpdateSpec => ResourceVerb::UpdateSpec,
        ResourceMutationKind::UpdateStatus => ResourceVerb::UpdateStatus,
        ResourceMutationKind::UpdateMetadata => ResourceVerb::UpdateMetadata,
        ResourceMutationKind::UpdateFinalizers => ResourceVerb::UpdateFinalizers,
        ResourceMutationKind::Delete => ResourceVerb::Delete,
    }
}

fn parse_mutation<T>(
    mutation: &wire::Mutation,
    route: &ParsedMutationRoute,
    trusted: &TrustedRequest<T>,
) -> Result<ParsedMutation, ResourceError> {
    let (kind, _) = parse_mutation_kind(mutation)?;
    if route.kind != kind {
        return Err(schema_error("mutation kind does not match the API method"));
    }
    let identity = &route.identity;
    if route.owner.is_some()
        && !matches!(
            kind,
            ResourceMutationKind::Create | ResourceMutationKind::UpdateMetadata
        )
    {
        return Err(schema_error(
            "owner changes require Create or UpdateMetadata",
        ));
    }
    if route
        .owner
        .as_ref()
        .is_some_and(|owner| owner.zone != identity.zone)
    {
        return Err(ref_error("owner and resource Zones differ"));
    }
    let expected = parse_precondition(mutation.precondition.as_ref())?;
    if matches!(kind, ResourceMutationKind::Create)
        != matches!(expected, ExpectedRevision::CreateAbsent)
    {
        return Err(schema_error(
            "mutation precondition does not match its kind",
        ));
    }
    let expected_uid = mutation
        .precondition
        .as_ref()
        .and_then(|precondition| precondition.expected_uid.as_ref())
        .map(|value| ResourceUid::parse(value.as_str()))
        .transpose()
        .map_err(|_| ref_error("precondition UID is invalid"))?;
    if kind == ResourceMutationKind::Create && (identity.uid.is_some() || expected_uid.is_some()) {
        return Err(schema_error("resource UID is store-generated on create"));
    }
    if identity.uid.is_some() && expected_uid.is_some() && identity.uid != expected_uid {
        return Err(ref_error("identity and precondition UIDs differ"));
    }

    let needs_body = matches!(
        kind,
        ResourceMutationKind::Create
            | ResourceMutationKind::UpdateSpec
            | ResourceMutationKind::UpdateStatus
            | ResourceMutationKind::UpdateMetadata
    );
    let body = mutation.resource.as_ref();
    if needs_body != body.is_some() {
        return Err(schema_error("mutation body does not match its kind"));
    }
    let canonical_resource = if let Some(body) = body {
        if body.canonical_json.len() > d2b_contracts::v3::MAX_RESOURCE_ENVELOPE_BYTES {
            return Err(schema_error("resource envelope exceeds its byte bound"));
        }
        let body_identity = parse_identity(body.identity.as_ref())?;
        if body_identity.zone != identity.zone
            || body_identity.resource_ref != identity.resource_ref
            || (identity.uid.is_some() && body_identity.uid != identity.uid)
        {
            return Err(schema_error(
                "resource body identity does not match its target",
            ));
        }
        let envelope = ResourceEnvelope::from_json(&body.canonical_json)
            .map_err(|_| schema_error("resource envelope is malformed"))?;
        if envelope.resource_type() != identity.resource_ref.resource_type()
            || envelope.metadata().name() != identity.resource_ref.name()
            || envelope.metadata().zone() != &identity.zone
            || identity
                .uid
                .as_ref()
                .is_some_and(|uid| uid != envelope.metadata().uid())
            || body.payload_digest != envelope.digest().unwrap_or_default()
        {
            return Err(schema_error(
                "resource envelope identity or digest does not match",
            ));
        }
        if matches!(
            kind,
            ResourceMutationKind::Create | ResourceMutationKind::UpdateMetadata
        ) && route.owner.as_ref().map(|owner| &owner.resource_ref)
            != envelope.metadata().owner_ref()
        {
            return Err(schema_error("typed owner does not match resource metadata"));
        }
        Some(body.canonical_json.clone())
    } else {
        None
    };

    let mut add_finalizers = mutation
        .add_finalizers
        .iter()
        .map(FinalizerId::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| schema_error("finalizer ID is invalid"))?;
    let mut remove_finalizers = mutation
        .remove_finalizers
        .iter()
        .map(FinalizerId::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| schema_error("finalizer ID is invalid"))?;
    add_finalizers.sort();
    add_finalizers.dedup();
    remove_finalizers.sort();
    remove_finalizers.dedup();
    if matches!(kind, ResourceMutationKind::UpdateFinalizers) {
        if add_finalizers.is_empty() && remove_finalizers.is_empty() {
            return Err(schema_error("finalizer update is empty"));
        }
    } else if !add_finalizers.is_empty() || !remove_finalizers.is_empty() {
        return Err(schema_error("finalizers require UpdateFinalizers"));
    }

    if kind == ResourceMutationKind::UpdateStatus
        && (trusted.subject.controller_generation().is_none()
            || trusted.subject.controller_generation()
                != trusted.authorization_state.snapshot.controller_generation)
    {
        return Err(ResourceError::terminal(
            ResourceErrorKind::ResourceStatusOwnerMismatch,
            "status controller generation does not match",
        ));
    }
    if mutation.wait_for_reconcile {
        if !matches!(
            kind,
            ResourceMutationKind::Create
                | ResourceMutationKind::UpdateSpec
                | ResourceMutationKind::Delete
        ) {
            return Err(schema_error(
                "expedited reconcile is not valid for this mutation",
            ));
        }
        if mutation.reconcile_deadline_ms == 0
            || mutation.reconcile_deadline_ms > MAX_EXPEDITED_DEADLINE_MS
        {
            return Err(schema_error(
                "expedited reconcile deadline exceeds its bound",
            ));
        }
        let expedited_subject = trusted.subject.evidence_class()
            == d2b_contracts::v3::EvidenceClass::UnixPeer
            && (trusted.subject.subject_ref().resource_type().as_str() == "User"
                || trusted.subject.subject_ref().to_string() == "Provider/system-core");
        if !expedited_subject {
            return Err(ResourceError::terminal(
                ResourceErrorKind::ExpeditedNotAuthorized,
                "expedited reconcile is not authorized",
            ));
        }
    } else if mutation.reconcile_deadline_ms != 0 {
        return Err(schema_error("reconcile deadline requires expedited mode"));
    }

    Ok(ParsedMutation {
        store: StoreMutation {
            kind,
            zone: identity.zone.clone(),
            target: identity.resource_ref.clone(),
            expected,
            expected_uid: identity.uid.clone().or(expected_uid),
            owner: route.owner.as_ref().map(|owner| owner.resource_ref.clone()),
            canonical_resource,
            add_finalizers,
            remove_finalizers,
            wait_for_reconcile: mutation.wait_for_reconcile,
            reconcile_deadline_ms: mutation
                .wait_for_reconcile
                .then_some(mutation.reconcile_deadline_ms),
        },
    })
}

fn parse_precondition(
    value: Option<&wire::Precondition>,
) -> Result<ExpectedRevision, ResourceError> {
    let value = value.ok_or_else(|| schema_error("precondition is required"))?;
    match value.kind.enum_value() {
        Ok(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT)
            if value.expected_revision.is_none() =>
        {
            Ok(ExpectedRevision::CreateAbsent)
        }
        Ok(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION) => value
            .expected_revision
            .filter(|revision| *revision != 0)
            .map(|revision| ExpectedRevision::Exact(ZoneRevision::new(revision)))
            .ok_or_else(|| schema_error("exact precondition requires a nonzero revision")),
        _ => Err(schema_error(
            "precondition kind is unspecified or inconsistent",
        )),
    }
}

struct ParsedCollection {
    resource_types: Vec<ResourceTypeName>,
    resource_names: Vec<ResourceName>,
    filters: Vec<StoreFilter>,
}

fn parse_collection_targets(
    resource_types: &[String],
    filters: &[wire::ListFilter],
    verb: ResourceVerb,
) -> Result<Vec<AuthorizationTarget>, ResourceError> {
    if resource_types.is_empty() {
        return Err(schema_error("at least one ResourceType is required"));
    }
    let resource_types = resource_types
        .iter()
        .map(ResourceTypeName::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ref_error("ResourceType is invalid"))?;
    let resource_names = filters
        .iter()
        .filter(|filter| filter.field == "metadata.name")
        .flat_map(|filter| filter.values.iter())
        .map(ResourceName::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ref_error("resource-name filter is invalid"))?;
    let mut targets = Vec::new();
    for resource_type in resource_types {
        if resource_names.is_empty() {
            targets.push(AuthorizationTarget {
                resource_type,
                resource_name: None,
                verb,
                subresource: None,
                execution_ref: None,
            });
        } else {
            targets.extend(resource_names.iter().cloned().map(|resource_name| {
                AuthorizationTarget {
                    resource_type: resource_type.clone(),
                    resource_name: Some(resource_name),
                    verb,
                    subresource: None,
                    execution_ref: None,
                }
            }));
        }
    }
    Ok(targets)
}

fn parse_collection_request(
    resource_types: &[String],
    filters: &[wire::ListFilter],
    max_resource_types: usize,
    max_filters: usize,
) -> Result<ParsedCollection, ResourceError> {
    if resource_types.is_empty() || resource_types.len() > max_resource_types {
        return Err(schema_error("ResourceType count exceeds its bound"));
    }
    let resource_types = resource_types
        .iter()
        .map(ResourceTypeName::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ref_error("ResourceType is invalid"))?;
    if filters.len() > max_filters
        || filters
            .iter()
            .any(|filter| filter.values.len() > MAX_FILTER_VALUES)
    {
        return Err(schema_error("filter count exceeds its bound"));
    }
    let resource_names = filters
        .iter()
        .filter(|filter| filter.field == "metadata.name")
        .flat_map(|filter| filter.values.iter())
        .map(ResourceName::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ref_error("resource-name filter is invalid"))?;
    Ok(ParsedCollection {
        resource_types,
        resource_names,
        filters: filters
            .iter()
            .map(|filter| StoreFilter {
                field: filter.field.clone(),
                values: filter.values.clone(),
            })
            .collect(),
    })
}

fn parse_projection(value: Option<&wire::Projection>) -> Result<StoreProjection, ResourceError> {
    match value.and_then(|projection| projection.kind.enum_value().ok()) {
        Some(wire::ProjectionKind::PROJECTION_KIND_FULL) => Ok(StoreProjection::Full),
        Some(wire::ProjectionKind::PROJECTION_KIND_BASE_ONLY) => Ok(StoreProjection::BaseOnly),
        Some(wire::ProjectionKind::PROJECTION_KIND_METADATA_ONLY) => {
            Ok(StoreProjection::MetadataOnly)
        }
        _ => Err(schema_error("projection is unspecified")),
    }
}

fn operation_context(
    meta: Option<&wire::RequestMeta>,
    mutation: bool,
    _state: &AuthorizationState,
) -> Result<StoreOperationContext, ResourceError> {
    let meta = meta.ok_or_else(|| schema_error("request metadata is required"))?;
    for value in [&meta.operation_id, &meta.correlation_id] {
        if !valid_id(value) {
            return Err(schema_error("operation metadata is invalid"));
        }
    }
    if mutation && !valid_id(&meta.idempotency_key) {
        return Err(schema_error("mutation idempotency key is required"));
    }
    if !meta.trace_id.is_empty() && !valid_id(&meta.trace_id) {
        return Err(schema_error("trace identity is invalid"));
    }
    let deadline_ms = if meta.deadline_ms == 0 {
        DEFAULT_REQUEST_DEADLINE_MS
    } else {
        meta.deadline_ms
    };
    if deadline_ms > MAX_REQUEST_DEADLINE_MS {
        return Err(schema_error("request deadline exceeds its bound"));
    }
    Ok(StoreOperationContext {
        operation_id: meta.operation_id.clone(),
        idempotency_key: mutation.then(|| meta.idempotency_key.clone()),
        correlation_id: meta.correlation_id.clone(),
        trace_id: (!meta.trace_id.is_empty()).then(|| meta.trace_id.clone()),
        deadline_ms,
    })
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn validate_request<T: StrictResourceRequest>(request: &T) -> Result<(), ResourceError> {
    if request.has_unknown_fields() {
        Err(schema_error("request contains unknown protobuf fields"))
    } else if request.compute_size() as usize > MAX_REQUEST_CANONICAL_BYTES {
        Err(schema_error("request exceeds its byte bound"))
    } else {
        Ok(())
    }
}

fn subject_zone<T>(trusted: &TrustedRequest<T>) -> ZoneId {
    ZoneId::parse(trusted.subject.zone_ref().name().as_str())
        .expect("authenticated Zone ref already carries a validated name")
}

fn to_wire_resource(resource: StoredResource) -> wire::ResourceEnvelopeBytes {
    let identity = to_wire_identity(&resource);
    let mut result = wire::ResourceEnvelopeBytes::new();
    result.identity = MessageField::some(identity);
    result.canonical_json = resource.canonical_json;
    result.payload_digest = resource.payload_digest;
    result
}

fn to_wire_identity(resource: &StoredResource) -> wire::ResourceIdentity {
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = resource.zone.to_string();
    identity.resource_type = resource.resource_ref.resource_type().to_string();
    identity.name = resource.resource_ref.name().to_string();
    identity.uid = Some(resource.uid.as_str().to_owned());
    identity.generation = Some(resource.generation.get());
    identity.revision = Some(resource.revision.get());
    identity
}

fn to_wire_resolved_identity(
    resource: d2b_resource_store::StoreResolvedIdentity,
) -> wire::ResourceIdentity {
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = resource.zone.to_string();
    identity.resource_type = resource.resource_ref.resource_type().to_string();
    identity.name = resource.resource_ref.name().to_string();
    identity.uid = Some(resource.uid.as_str().to_owned());
    identity.generation = Some(resource.generation.get());
    identity.revision = Some(resource.revision.get());
    identity
}

fn mutation_response(
    result: StoreCommitResult,
    mutation: Option<&wire::Mutation>,
    expedited_capable: bool,
) -> wire::CreateResponse {
    let mut response = wire::CreateResponse::new();
    response.revision = result.revision.get();
    if let Some(resource) = result.resources.into_iter().next() {
        response.resource = MessageField::some(to_wire_resource(resource));
    }
    if expedited_capable && mutation.is_some_and(|mutation| mutation.wait_for_reconcile) {
        response.error = MessageField::some(to_wire_error(&ResourceError::terminal(
            ResourceErrorKind::ExpeditedReconcilePending,
            "resource committed and reconcile remains pending",
        )));
    }
    response
}

fn copy_update_spec_response(value: wire::CreateResponse) -> wire::UpdateSpecResponse {
    let mut response = wire::UpdateSpecResponse::new();
    response.resource = value.resource;
    response.revision = value.revision;
    response.error = value.error;
    response.disposition = value.disposition;
    response.status_persistence = value.status_persistence;
    response.last_persisted_status_revision = value.last_persisted_status_revision;
    response.reconcile_projection = value.reconcile_projection;
    response
}

fn copy_update_status_response(value: wire::CreateResponse) -> wire::UpdateStatusResponse {
    let mut response = wire::UpdateStatusResponse::new();
    response.resource = value.resource;
    response.revision = value.revision;
    response.error = value.error;
    response
}

fn copy_update_metadata_response(value: wire::CreateResponse) -> wire::UpdateMetadataResponse {
    let mut response = wire::UpdateMetadataResponse::new();
    response.resource = value.resource;
    response.revision = value.revision;
    response.error = value.error;
    response
}

fn copy_update_finalizers_response(value: wire::CreateResponse) -> wire::UpdateFinalizersResponse {
    let mut response = wire::UpdateFinalizersResponse::new();
    response.resource = value.resource;
    response.revision = value.revision;
    response.error = value.error;
    response
}

fn schema_error(reason: &'static str) -> ResourceError {
    ResourceError::terminal(ResourceErrorKind::ResourceSchemaInvalid, reason)
}

fn ref_error(reason: &'static str) -> ResourceError {
    ResourceError::terminal(ResourceErrorKind::ResourceRefInvalid, reason)
}

macro_rules! response_error {
    ($name:ident, $ty:ty) => {
        fn $name(error: ResourceError) -> $ty {
            let mut response = <$ty>::new();
            response.error = MessageField::some(to_wire_error(&error));
            response
        }
    };
}

response_error!(get_error, wire::GetResponse);
response_error!(list_error, wire::ListResponse);
response_error!(watch_error, wire::WatchResponse);
response_error!(create_error, wire::CreateResponse);
response_error!(update_spec_error, wire::UpdateSpecResponse);
response_error!(update_status_error, wire::UpdateStatusResponse);
response_error!(update_metadata_error, wire::UpdateMetadataResponse);
response_error!(update_finalizers_error, wire::UpdateFinalizersResponse);
response_error!(delete_error, wire::DeleteResponse);
response_error!(batch_error, wire::CommitBatchResponse);
response_error!(resolve_error, wire::ResolveRefResponse);
response_error!(inspect_error, wire::InspectSchemaResponse);
response_error!(upgrade_error, wire::UpgradeResponse);

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    };

    use d2b_contracts::v3::{
        BindingDigest, ConfigurationGeneration, ControllerGeneration, EvidenceClass, Locality,
        ReconnectGeneration, ResourceGeneration, SchemaFingerprint, ServiceName, SessionBinding,
        SessionPurpose, TranscriptHash, TransportBinding,
    };
    use d2b_resource_store::{
        StoreError, StoreErrorKind, StoreListResult, StoreResolvedIdentity, StoreWatchReceipt,
        StoredSchema,
    };
    use protobuf::EnumOrUnknown;

    use crate::authz::{
        BindingScope, BoundSubject, CompiledRole, CompiledRoleBinding, PolicyRule, PolicySet,
        RelayGrantAuthority,
    };

    const GOLDEN_HOST: &[u8] = br#"{"apiVersion":"resources.d2bus.org/v3","metadata":{"configurationGeneration":7,"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"host-system","ownerRef":null,"revision":1,"uid":"123e4567-e89b-42d3-a456-426614174000","updatedAt":"2026-07-22T00:00:00.000Z","zone":"dev"},"spec":{"providerRef":"Provider/system-core","updatePolicy":{"disruptive":"manual","nonDisruptive":"automatic"}},"status":{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{},"startedAt":null,"update":{"dependencies":{"count":0,"refs":[]},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{"count":0,"refs":[]},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}},"type":"Host"}"#;

    #[derive(Debug, Clone, Copy)]
    enum CommitMode {
        Success,
        Conflict,
    }

    #[derive(Debug)]
    struct FakeStore {
        mode: Mutex<CommitMode>,
        commits: AtomicUsize,
        mutation_count: AtomicUsize,
        configuration_revision: AtomicU64,
    }

    impl FakeStore {
        fn new(mode: CommitMode) -> Self {
            Self {
                mode: Mutex::new(mode),
                commits: AtomicUsize::new(0),
                mutation_count: AtomicUsize::new(0),
                configuration_revision: AtomicU64::new(0),
            }
        }

        fn unavailable() -> StoreError {
            StoreError::new(
                StoreErrorKind::ResourcePlaneUnavailable,
                None,
                None,
                d2b_contracts::v3::RetryClass::AfterDelay,
                "fake-unavailable",
            )
        }
    }

    impl ResourceStore for FakeStore {
        async fn get(&self, _request: StoreGetRequest) -> Result<StoredResource, StoreError> {
            Err(Self::unavailable())
        }

        async fn list(&self, _request: StoreListRequest) -> Result<StoreListResult, StoreError> {
            Err(Self::unavailable())
        }

        async fn watch(
            &self,
            _request: StoreWatchRequest,
        ) -> Result<StoreWatchReceipt, StoreError> {
            Err(Self::unavailable())
        }

        async fn resolve_ref(
            &self,
            _request: StoreResolveRequest,
        ) -> Result<StoreResolvedIdentity, StoreError> {
            Err(Self::unavailable())
        }

        async fn inspect_schema(
            &self,
            _request: StoreInspectSchemaRequest,
        ) -> Result<StoredSchema, StoreError> {
            Err(Self::unavailable())
        }

        async fn commit(
            &self,
            mutation: AdmittedMutation,
        ) -> Result<StoreCommitResult, StoreError> {
            self.commits.fetch_add(1, Ordering::SeqCst);
            self.mutation_count
                .store(mutation.mutations().len(), Ordering::SeqCst);
            self.configuration_revision.store(
                mutation
                    .policy_snapshot()
                    .active_configuration_revision
                    .get(),
                Ordering::SeqCst,
            );
            match *self.mode.lock().unwrap() {
                CommitMode::Success => Ok(StoreCommitResult {
                    resources: Vec::new(),
                    revision: ZoneRevision::new(9),
                }),
                CommitMode::Conflict => Err(StoreError::new(
                    StoreErrorKind::ResourceConflict,
                    Some(ZoneRevision::new(8)),
                    None,
                    d2b_contracts::v3::RetryClass::Reauthorize,
                    "revision-changed",
                )),
            }
        }
    }

    fn subject(controller_generation: Option<u64>) -> Arc<AuthenticatedSubjectContext> {
        let context = AuthenticatedSubjectContext::new(
            ResourceRef::parse("Provider/system-core").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap(),
            ResourceRef::parse("Zone/dev").unwrap(),
            EvidenceClass::UnixPeer,
            SessionPurpose::parse("resource-api").unwrap(),
            ServiceName::parse("d2b.resource.v3").unwrap(),
            SessionBinding::new(
                SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
                TransportBinding::new(
                    Locality::Local,
                    BindingDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
                ),
                ReconnectGeneration::new(1).unwrap(),
                TranscriptHash::from_bytes([3; 32]),
            ),
        );
        Arc::new(match controller_generation {
            Some(value) => {
                context.with_controller_generation(ControllerGeneration::new(value).unwrap())
            }
            None => context,
        })
    }

    fn state(controller_generation: Option<u64>) -> AuthorizationState {
        AuthorizationState {
            snapshot: d2b_resource_store::PolicySnapshot {
                policy_revision: 4,
                api_catalog_revision: 5,
                active_configuration_revision: ConfigurationGeneration::new(6).unwrap(),
                controller_generation: controller_generation
                    .map(|value| ControllerGeneration::new(value).unwrap()),
            },
            zone_policy_revision: ZoneRevision::new(7),
            bootstrap_phase: crate::authz::BootstrapPhase::Disabled,
            now_tick: 1,
        }
    }

    fn authorizer(verbs: impl IntoIterator<Item = ResourceVerb>) -> Arc<NativeAuthorizer> {
        let context = subject(None);
        let verbs = verbs.into_iter().collect::<Vec<_>>();
        let subresources = if verbs.contains(&ResourceVerb::UpdateStatus) {
            vec!["status".to_owned()]
        } else if verbs.contains(&ResourceVerb::UpdateFinalizers) {
            vec!["finalizers".to_owned()]
        } else {
            Vec::new()
        };
        let role = CompiledRole::new(
            ResourceRef::parse("Role/test").unwrap(),
            vec![
                PolicyRule::new(
                    [ResourceTypeName::parse("Host").unwrap()],
                    verbs,
                    [],
                    subresources,
                    [ResourceName::parse("host-system").unwrap()],
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
        Arc::new(NativeAuthorizer::new(Some(
            PolicySet::new(4, vec![role], vec![binding]).unwrap(),
        )))
    }

    fn request_meta() -> MessageField<wire::RequestMeta> {
        let mut meta = wire::RequestMeta::new();
        meta.operation_id = "operation-1".to_owned();
        meta.idempotency_key = "idempotency-1".to_owned();
        meta.correlation_id = "correlation-1".to_owned();
        MessageField::some(meta)
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
        let mut precondition = wire::Precondition::new();
        if kind == wire::MutationKind::MUTATION_KIND_CREATE {
            precondition.kind =
                EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT);
        } else {
            precondition.kind =
                EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
            precondition.expected_revision = Some(1);
        }
        mutation.precondition = MessageField::some(precondition);
        mutation
    }

    fn body(bytes: Vec<u8>) -> MessageField<wire::ResourceEnvelopeBytes> {
        let envelope = ResourceEnvelope::from_json(GOLDEN_HOST).unwrap();
        let mut body = wire::ResourceEnvelopeBytes::new();
        body.identity = identity();
        body.payload_digest = envelope.digest().unwrap();
        body.canonical_json = bytes;
        MessageField::some(body)
    }

    fn trusted<T>(request: T, controller_generation: Option<u64>) -> TrustedRequest<T> {
        TrustedRequest::from_component_session(
            subject(controller_generation),
            state(controller_generation),
            false,
            request,
        )
    }

    fn error_kind(error: &MessageField<wire::ResourceError>) -> wire::ResourceErrorKind {
        error.as_ref().unwrap().kind.enum_value().unwrap()
    }

    #[tokio::test]
    async fn native_authorization_precedes_body_validation() {
        let store = Arc::new(FakeStore::new(CommitMode::Success));
        let service = ResourceService::new(Arc::clone(&store), authorizer([]));
        let mut request = wire::CreateRequest::new();
        request.meta = request_meta();
        let mut value = mutation(wire::MutationKind::MUTATION_KIND_CREATE);
        value.resource = body(b"{}".to_vec());
        request.mutation = MessageField::some(value);

        let response = service.create(trusted(request, None)).await;
        assert_eq!(
            error_kind(&response.error),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_AUTHORIZATION_DENIED
        );
        assert_eq!(store.commits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn owner_reference_requires_an_independent_read_grant() {
        let store = Arc::new(FakeStore::new(CommitMode::Success));
        let service = ResourceService::new(Arc::clone(&store), authorizer([ResourceVerb::Create]));
        let mut request = wire::CreateRequest::new();
        request.meta = request_meta();
        let mut value = mutation(wire::MutationKind::MUTATION_KIND_CREATE);
        value.resource = body(GOLDEN_HOST.to_vec());
        let mut owner = wire::ResourceIdentity::new();
        owner.zone = "dev".to_owned();
        owner.resource_type = "Provider".to_owned();
        owner.name = "system-core".to_owned();
        value.owner = MessageField::some(owner);
        request.mutation = MessageField::some(value);

        let response = service.create(trusted(request, None)).await;
        assert_eq!(
            error_kind(&response.error),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_AUTHORIZATION_DENIED
        );
        assert_eq!(store.commits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn malformed_and_oversize_envelopes_never_reach_the_store() {
        let store = Arc::new(FakeStore::new(CommitMode::Success));
        let service = ResourceService::new(Arc::clone(&store), authorizer([ResourceVerb::Create]));
        for bytes in [
            b"{}".to_vec(),
            vec![b'x'; d2b_contracts::v3::MAX_RESOURCE_ENVELOPE_BYTES + 1],
        ] {
            let mut request = wire::CreateRequest::new();
            request.meta = request_meta();
            let mut value = mutation(wire::MutationKind::MUTATION_KIND_CREATE);
            value.resource = body(bytes);
            request.mutation = MessageField::some(value);
            let response = service.create(trusted(request, None)).await;
            assert_eq!(
                error_kind(&response.error),
                wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_SCHEMA_INVALID
            );
        }
        assert_eq!(store.commits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn create_rejects_every_caller_supplied_uid_field() {
        let store = Arc::new(FakeStore::new(CommitMode::Success));
        let service = ResourceService::new(Arc::clone(&store), authorizer([ResourceVerb::Create]));
        for uid_location in 0..2 {
            let mut request = wire::CreateRequest::new();
            request.meta = request_meta();
            let mut value = mutation(wire::MutationKind::MUTATION_KIND_CREATE);
            value.resource = body(GOLDEN_HOST.to_vec());
            if uid_location == 0 {
                value.target.mut_or_insert_default().uid =
                    Some("123e4567-e89b-42d3-a456-426614174000".to_owned());
            } else {
                value.precondition.mut_or_insert_default().expected_uid =
                    Some("123e4567-e89b-42d3-a456-426614174000".to_owned());
            }
            request.mutation = MessageField::some(value);
            let response = service.create(trusted(request, None)).await;
            assert_eq!(
                error_kind(&response.error),
                wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_SCHEMA_INVALID
            );
        }
        assert_eq!(store.commits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unknown_protobuf_fields_are_rejected_after_authorization() {
        let store = Arc::new(FakeStore::new(CommitMode::Success));
        let service = ResourceService::new(Arc::clone(&store), authorizer([ResourceVerb::Create]));
        let mut request = wire::CreateRequest::parse_from_bytes(&[0x98, 0x06, 0x01]).unwrap();
        request.meta = request_meta();
        let mut value = mutation(wire::MutationKind::MUTATION_KIND_CREATE);
        value.resource = body(GOLDEN_HOST.to_vec());
        request.mutation = MessageField::some(value);

        let response = service.create(trusted(request, None)).await;
        assert_eq!(
            error_kind(&response.error),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_SCHEMA_INVALID
        );
        assert_eq!(store.commits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn conflict_returns_only_safe_revision_metadata() {
        let store = Arc::new(FakeStore::new(CommitMode::Conflict));
        let service = ResourceService::new(
            Arc::clone(&store),
            authorizer([ResourceVerb::Create, ResourceVerb::Get]),
        );
        let mut request = wire::CreateRequest::new();
        request.meta = request_meta();
        let mut value = mutation(wire::MutationKind::MUTATION_KIND_CREATE);
        value.resource = body(GOLDEN_HOST.to_vec());
        request.mutation = MessageField::some(value);

        let response = service.create(trusted(request, None)).await;
        let error = response.error.as_ref().unwrap();
        assert_eq!(
            error.kind.enum_value().unwrap(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_CONFLICT
        );
        assert_eq!(error.current_revision, Some(8));
        assert!(response.resource.is_none());
    }

    #[tokio::test]
    async fn conflict_hides_revision_without_read_authority() {
        let store = Arc::new(FakeStore::new(CommitMode::Conflict));
        let service = ResourceService::new(Arc::clone(&store), authorizer([ResourceVerb::Create]));
        let mut request = wire::CreateRequest::new();
        request.meta = request_meta();
        let mut value = mutation(wire::MutationKind::MUTATION_KIND_CREATE);
        value.resource = body(GOLDEN_HOST.to_vec());
        request.mutation = MessageField::some(value);

        let response = service.create(trusted(request, None)).await;
        let error = response.error.as_ref().unwrap();
        assert_eq!(
            error.kind.enum_value().unwrap(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_CONFLICT
        );
        assert_eq!(error.current_revision, None);
        assert!(response.resource.is_none());
    }

    #[tokio::test]
    async fn status_owner_generation_is_checked_after_authorization() {
        let store = Arc::new(FakeStore::new(CommitMode::Success));
        let service =
            ResourceService::new(Arc::clone(&store), authorizer([ResourceVerb::UpdateStatus]));
        let mut request = wire::UpdateStatusRequest::new();
        request.meta = request_meta();
        let mut value = mutation(wire::MutationKind::MUTATION_KIND_UPDATE_STATUS);
        value.resource = body(GOLDEN_HOST.to_vec());
        request.mutation = MessageField::some(value);

        let response = service.update_status(trusted(request, None)).await;
        assert_eq!(
            error_kind(&response.error),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_STATUS_OWNER_MISMATCH
        );
        assert_eq!(store.commits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn finalizers_are_separate_and_batch_is_one_admitted_commit() {
        let store = Arc::new(FakeStore::new(CommitMode::Success));
        let metadata_service = ResourceService::new(
            Arc::clone(&store),
            authorizer([ResourceVerb::UpdateMetadata]),
        );
        let mut request = wire::UpdateMetadataRequest::new();
        request.meta = request_meta();
        let mut value = mutation(wire::MutationKind::MUTATION_KIND_UPDATE_METADATA);
        value.resource = body(GOLDEN_HOST.to_vec());
        value.add_finalizers.push("core.cleanup".to_owned());
        request.mutation = MessageField::some(value);
        let response = metadata_service
            .update_metadata(trusted(request, None))
            .await;
        assert_eq!(
            error_kind(&response.error),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_SCHEMA_INVALID
        );

        let batch_service =
            ResourceService::new(Arc::clone(&store), authorizer([ResourceVerb::Delete]));
        let mut batch = wire::CommitBatchRequest::new();
        batch.meta = request_meta();
        batch.mutations = vec![
            mutation(wire::MutationKind::MUTATION_KIND_DELETE),
            mutation(wire::MutationKind::MUTATION_KIND_DELETE),
        ];
        let response = batch_service.commit_batch(trusted(batch, None)).await;
        assert!(response.error.is_none());
        assert_eq!(response.revision, 9);
        assert_eq!(store.commits.load(Ordering::SeqCst), 1);
        assert_eq!(store.mutation_count.load(Ordering::SeqCst), 2);
        assert_eq!(store.configuration_revision.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn status_owner_matching_generation_is_representable() {
        let context = subject(Some(11));
        assert_eq!(
            context.controller_generation(),
            Some(ControllerGeneration::new(11).unwrap())
        );
        let _: ResourceGeneration = ResourceGeneration::new(11).unwrap();
    }
}
