//! Target-local Resource API for an authenticated Guest ComponentSession.
//!
//! Guest mode has no Zone store.  Controller-created Process-family resources
//! therefore live in this bounded in-memory store and are exposed only through
//! the currently admitted parent-Zone ComponentSession.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    sync::{Arc, Mutex},
};

use d2b_contracts_resource::v3::{
    ConfigurationGeneration, ControllerGeneration, ResourceEnvelope, ResourceGeneration,
    ResourceRef, ResourceTypeName, ResourceUid, RetryClass, ZoneId,
    ZoneRevision,
    activation_nixos::NIXOS_GENERATION_RESOURCE_TYPE,
    resource_schema::SCHEMA_DOMAIN_TAG,
};
use d2b_resource_api::{
    ResourceApiClient, ResourceBusAdapter, ResourceService, ResourceStoreBackend,
    authz::{
        ApiCatalog, AuthorizationState, BindingScope, BootstrapPhase, BoundSubject, CompiledRole,
        CompiledRoleBinding, NativeAuthorizer, PolicyRule, PolicySet, RelayGrantAuthority,
        ResourceVerb, SessionVerb,
    },
    service::UnavailableUpgradeDispatcher,
};
use d2b_resource_store::{
    ExpectedRevision, MutationSealBody, ResourceMutationKind, SealedMutation, StoreCommitResult,
    StoreError, StoreErrorKind, StoreGetRequest, StoreInspectSchemaRequest, StoreListRequest,
    StoreListResult, StoreResolveRequest, StoreResolvedIdentity, StoreWatchReceipt,
    StoreWatchRequest, StoredResource, StoredSchema,
    mutation_seal::MutationSealAcceptor,
};
use d2b_resource_store_redb::{
    RedbResourceStore, StoreIdentity, write_provisioning_marker,
};

use crate::{
    guest_mode::GuestIdentity,
    resource_runtime_support::{initial_policy_snapshot, store_identity},
};

#[cfg(test)]
const STORE_SLOT: u32 = 0;
const STORE_FILE_NAME: &str = "resource-store.redb";
const STORE_MARKER_NAME: &str = "resource-store.marker";
const ROLE_REF: &str = "Role/guest-component-session";
const WATCH_STREAM_PREFIX: &str = "guest-watch";
const SCHEMA_BYTES: &[u8] = br#"{"apiVersion":"d2b-cjson/v1","resourceType":"target-local"}"#;
type CommitFence = Arc<dyn Fn() -> Result<(), StoreError> + Send + Sync>;

/// Authenticated target-local resource runtime for Guest mode.
#[derive(Clone)]
pub struct GuestResourceRuntime {
    identity: GuestIdentity,
    store: Arc<GuestResourceStore>,
    authorizer: Arc<NativeAuthorizer>,
    authorization_state: AuthorizationState,
    active_generation: Arc<Mutex<Option<u64>>>,
}

impl core::fmt::Debug for GuestResourceRuntime {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("GuestResourceRuntime(<redacted>)")
    }
}

impl GuestResourceRuntime {
    /// Build the target-local Resource API with Guest-owned durable state.
    pub async fn new(
        identity: GuestIdentity,
        state_dir: impl AsRef<Path>,
    ) -> Result<Self, GuestResourceRuntimeError> {
        let zone = identity.zone().clone();
        let activation_type = ResourceTypeName::parse(NIXOS_GENERATION_RESOURCE_TYPE)
            .map_err(|_| GuestResourceRuntimeError::Policy)?;
        let catalog = ApiCatalog::with_extensions([activation_type.clone()])
            .map_err(|_| GuestResourceRuntimeError::Policy)?;
        let resource_types = [
            "Process",
            "EphemeralProcess",
            "Endpoint",
            NIXOS_GENERATION_RESOURCE_TYPE,
        ]
        .into_iter()
        .map(ResourceTypeName::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| GuestResourceRuntimeError::Policy)?;
        let resource_verbs = [
            ResourceVerb::Get,
            ResourceVerb::List,
            ResourceVerb::Watch,
            ResourceVerb::Create,
            ResourceVerb::UpdateSpec,
            ResourceVerb::UpdateStatus,
            ResourceVerb::UpdateMetadata,
            ResourceVerb::UpdateFinalizers,
            ResourceVerb::Delete,
        ];
        let session_verbs = [
            SessionVerb::Connect,
            SessionVerb::Invoke,
            SessionVerb::OpenStream,
            SessionVerb::Cancel,
            SessionVerb::Observe,
        ];
        let rules = resource_types
            .chunks(16)
            .map(|resource_types| {
                PolicyRule::new(
                    &catalog,
                    resource_types.iter().cloned(),
                    resource_verbs,
                    session_verbs,
                    [],
                    [],
                    [zone.clone()],
                    [],
                )
                .map_err(|_| GuestResourceRuntimeError::Policy)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let role_ref =
            ResourceRef::parse(ROLE_REF).map_err(|_| GuestResourceRuntimeError::Policy)?;
        let role =
            CompiledRole::new(role_ref.clone(), rules)
                .map_err(|_| GuestResourceRuntimeError::Policy)?;
        let binding_scope = BindingScope {
            zones: [zone.clone()].into_iter().collect(),
            ..BindingScope::default()
        };
        let binding = CompiledRoleBinding::new(
            role_ref,
            [BoundSubject {
                subject_ref: identity.guest_ref().clone(),
                subject_uid: identity.guest_uid().clone(),
            }],
            binding_scope,
            RelayGrantAuthority::None,
        )
        .map_err(|_| GuestResourceRuntimeError::Policy)?;
        let policy_revision = 1;
        let policy =
            PolicySet::new(&catalog, policy_revision, vec![role], vec![binding])
                .map_err(|_| GuestResourceRuntimeError::Policy)?;
        let authorization_state = AuthorizationState {
            snapshot: d2b_resource_store::PolicySnapshot {
                policy_revision,
                api_catalog_revision: 1,
                active_configuration_revision: ConfigurationGeneration::new(1)
                    .map_err(|_| GuestResourceRuntimeError::Policy)?,
                controller_generation: Some(
                    ControllerGeneration::new(identity.controller_generation())
                        .map_err(|_| GuestResourceRuntimeError::Policy)?,
                ),
            },
            zone_policy_revision: ZoneRevision::new(1),
            bootstrap_phase: BootstrapPhase::Disabled,
            now_tick: 1,
        };
        let authorizer = Arc::new(
            NativeAuthorizer::new(catalog, Some(policy))
                .map_err(|_| GuestResourceRuntimeError::Policy)?,
        );
        let store_identity = store_identity(&zone, &format!("guest-target:{}", identity.guest_uid()))
            .map_err(|_| GuestResourceRuntimeError::Store)?
            .with_revisions(
                initial_policy_snapshot().map_err(|_| GuestResourceRuntimeError::Store)?,
            );
        let acceptor = authorizer
            .take_store_seal(store_identity.seal_identity())
            .map_err(|_| GuestResourceRuntimeError::Store)?;
        let backend = Arc::new(
            GuestResourceStore::open_durable(
                zone,
                identity.guest_ref().clone(),
                state_dir.as_ref(),
                store_identity,
                acceptor,
            )
            .await?,
        );
        let active_generation = Arc::new(Mutex::new(None));
        Ok(Self {
            identity,
            store: backend,
            authorizer,
            authorization_state,
            active_generation,
        })
    }

    /// This runtime is intentionally not backed by a local Zone store.
    pub const fn is_target_local(&self) -> bool {
        true
    }

    pub(crate) fn active_generation(&self) -> Arc<Mutex<Option<u64>>> {
        Arc::clone(&self.active_generation)
    }

    /// Bind the Resource API to one already authenticated session route.
    pub fn bind_session(
        &self,
        route: &d2b_session::AuthenticatedSessionRouteBinding,
    ) -> Result<GuestResourceSession, GuestResourceRuntimeError> {
        self.identity
            .validate_route(route)
            .map_err(|_| GuestResourceRuntimeError::SessionBinding)?;
        let subject = self
            .authorizer
            .issue_authenticated_subject(route.context().clone(), self.authorization_state.clone())
            .map_err(|_| GuestResourceRuntimeError::Authorization)?;
        let backend = Arc::new(SessionBoundStore {
            store: Arc::clone(&self.store),
            active_generation: Arc::clone(&self.active_generation),
            generation: route.reconnect_generation().get(),
        });
        let session_store = Arc::clone(&backend);
        let service = Arc::new(
            ResourceService::new(backend, Arc::clone(&self.authorizer))
                .map_err(|_| GuestResourceRuntimeError::Store)?,
        );
        let adapter = ResourceBusAdapter::bind_component_session(service, subject)
            .map_err(|_| GuestResourceRuntimeError::Authorization)?;
        Ok(GuestResourceSession {
            store: session_store,
            adapter: Arc::new(adapter),
            generation: route.reconnect_generation().get(),
        })
    }
}

/// Resource API capability bound to one Guest session generation.
pub struct GuestResourceSession {
    store: Arc<SessionBoundStore>,
    adapter: Arc<ResourceBusAdapter<SessionBoundStore, UnavailableUpgradeDispatcher>>,
    generation: u64,
}

impl core::fmt::Debug for GuestResourceSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GuestResourceSession")
            .field("generation", &"<redacted>")
            .finish()
    }
}

impl GuestResourceSession {
    /// Return the authenticated reconnect generation for diagnostics.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Build the generated ResourceService map for the session server.
    pub fn ttrpc_services(&self) -> std::collections::HashMap<String, ttrpc::r#async::Service> {
        Arc::clone(&self.adapter).ttrpc_services()
    }

    /// Return the in-process client used by target-local controllers.
    pub fn client(&self) -> ResourceApiClient<SessionBoundStore, UnavailableUpgradeDispatcher> {
        self.adapter.client()
    }

    /// Return the store backend fenced to this authenticated session.
    ///
    /// Target-local controllers use the same backend as the bus adapter so
    /// relists and status mutations cannot outlive the session generation.
    pub fn store_backend(&self) -> Arc<SessionBoundStore> {
        Arc::clone(&self.store)
    }
}

/// Closed construction and binding failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestResourceRuntimeError {
    Policy,
    Store,
    StoreQuarantined,
    SessionBinding,
    Authorization,
}

impl core::fmt::Display for GuestResourceRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Policy => "guest-resource-policy-unavailable",
            Self::Store => "guest-resource-store-unavailable",
            Self::StoreQuarantined => "guest-resource-store-quarantined",
            Self::SessionBinding => "guest-resource-session-binding-invalid",
            Self::Authorization => "guest-resource-authorization-denied",
        })
    }
}

impl std::error::Error for GuestResourceRuntimeError {}

struct GuestStoreState {
    revision: u64,
    resources: BTreeMap<ResourceRef, StoredResource>,
    next_watch: u64,
}

enum GuestStoreBackend {
    Durable(Arc<RedbResourceStore>),
    Memory {
        acceptor: MutationSealAcceptor,
        state: Mutex<GuestStoreState>,
    },
}

/// Target-local store owned by one Guest.
pub struct GuestResourceStore {
    zone: ZoneId,
    target: Option<ResourceRef>,
    backend: GuestStoreBackend,
}

impl core::fmt::Debug for GuestResourceStore {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GuestResourceStore")
            .field("resource_count", &self.resource_count())
            .finish()
    }
}

impl GuestResourceStore {
    fn new_in_memory(zone: ZoneId, acceptor: MutationSealAcceptor) -> Self {
        Self {
            zone,
            target: None,
            backend: GuestStoreBackend::Memory {
                acceptor,
                state: Mutex::new(GuestStoreState {
                    revision: 0,
                    resources: BTreeMap::new(),
                    next_watch: 0,
                }),
            },
        }
    }

    async fn open_durable(
        zone: ZoneId,
        target: ResourceRef,
        state_dir: &Path,
        identity: StoreIdentity,
        acceptor: MutationSealAcceptor,
    ) -> Result<Self, GuestResourceRuntimeError> {
        let metadata = fs::symlink_metadata(state_dir)
            .map_err(|_| GuestResourceRuntimeError::Store)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.mode() & 0o002 != 0 {
            return Err(GuestResourceRuntimeError::Store);
        }
        let database_path = state_dir.join(STORE_FILE_NAME);
        let marker_path = state_dir.join(STORE_MARKER_NAME);
        let database_present = fs::symlink_metadata(&database_path)
            .map(|metadata| !metadata.file_type().is_symlink())
            .unwrap_or(false);
        let marker_present = fs::symlink_metadata(&marker_path)
            .map(|metadata| !metadata.file_type().is_symlink())
            .unwrap_or(false);
        if database_present != marker_present {
            return Err(GuestResourceRuntimeError::StoreQuarantined);
        }
        let database = open_owned_file(&database_path)?;
        let mut marker = open_owned_file(&marker_path)?;
        let database_empty = database
            .metadata()
            .map_err(|_| GuestResourceRuntimeError::Store)?
            .len()
            == 0;
        let marker_empty = marker
            .metadata()
            .map_err(|_| GuestResourceRuntimeError::Store)?
            .len()
            == 0;
        let store = if database_empty && marker_empty {
            write_provisioning_marker(&mut marker, &identity)
                .map_err(|_| GuestResourceRuntimeError::Store)?;
            RedbResourceStore::provision_owned(database, marker, identity, acceptor)
                .await
                .map_err(map_store_error)?
        } else if database_empty || marker_empty {
            return Err(GuestResourceRuntimeError::StoreQuarantined);
        } else {
            drop(marker);
            RedbResourceStore::open_owned(database, identity, acceptor)
                .await
                .map_err(map_store_error)?
        };
        Ok(Self {
            zone,
            target: Some(target),
            backend: GuestStoreBackend::Durable(Arc::new(store)),
        })
    }

    fn resource_count(&self) -> usize {
        match &self.backend {
            GuestStoreBackend::Durable(_) => 0,
            GuestStoreBackend::Memory { state, .. } => state
                .lock()
                .map(|state| state.resources.len())
                .unwrap_or(0),
        }
    }

    fn is_target_local_type(resource_type: &ResourceTypeName) -> bool {
        matches!(
            resource_type.as_str(),
            "Process" | "EphemeralProcess" | "Endpoint" | NIXOS_GENERATION_RESOURCE_TYPE
        )
    }

    fn forbidden() -> StoreError {
        StoreError::new(
            StoreErrorKind::AuthorizationDenied,
            None,
            None,
            RetryClass::Never,
            "guest-target-resource-type-denied",
        )
    }

    fn unavailable(reason: &'static str) -> StoreError {
        StoreError::new(
            StoreErrorKind::ResourcePlaneUnavailable,
            None,
            None,
            RetryClass::AfterDelay,
            reason,
        )
    }

    fn invalid(reason: &'static str) -> StoreError {
        StoreError::new(
            StoreErrorKind::ResourceSchemaInvalid,
            None,
            None,
            RetryClass::Never,
            reason,
        )
    }

    fn not_found() -> StoreError {
        StoreError::new(
            StoreErrorKind::ResourceNotFound,
            None,
            None,
            RetryClass::Never,
            "guest-target-resource-not-found",
        )
    }

    fn conflict(revision: u64) -> StoreError {
        StoreError::new(
            StoreErrorKind::ResourceConflict,
            Some(ZoneRevision::new(revision)),
            None,
            RetryClass::Reauthorize,
            "guest-target-resource-revision-changed",
        )
    }

    fn parse_resource(
        &self,
        target: &ResourceRef,
        canonical: &[u8],
    ) -> Result<(ResourceUid, ResourceGeneration), StoreError> {
        let envelope = ResourceEnvelope::from_json(canonical).map_err(|_| Self::invalid(
            "guest-target-resource-envelope-invalid",
        ))?;
        let envelope_ref = ResourceRef::new(
            envelope.resource_type().clone(),
            envelope.metadata().name().clone(),
        );
        if &envelope_ref != target || envelope.metadata().zone() != &self.zone {
            return Err(Self::invalid("guest-target-resource-identity-mismatch"));
        }
        if matches!(
            target.resource_type().as_str(),
            "Process" | "EphemeralProcess" | NIXOS_GENERATION_RESOURCE_TYPE
        ) {
            let execution_ref = envelope
                .spec()
                .base()
                .get("executionRef")
                .and_then(|value| match value {
                    d2b_contracts_resource::v3::CanonicalJsonValue::String(value) => {
                        ResourceRef::parse(value).ok()
                    }
                    _ => None,
                })
                .ok_or_else(|| Self::invalid("guest-target-execution-ref-missing"))?;
            if self.target.as_ref() != Some(&execution_ref) {
                return Err(Self::invalid("guest-target-execution-ref-mismatch"));
            }
        }
        Ok((
            envelope.metadata().uid().clone(),
            envelope.metadata().generation(),
        ))
    }

    fn validate_mutation_body(&self, body: &MutationSealBody) -> Result<(), StoreError> {
        if body.authorization.zone != self.zone {
            return Err(Self::invalid("guest-target-authorization-zone-mismatch"));
        }
        for prepared in &body.mutations {
            let mutation = prepared.mutation();
            if mutation.zone != self.zone {
                return Err(Self::invalid("guest-target-resource-zone-mismatch"));
            }
            if !Self::is_target_local_type(mutation.target.resource_type()) {
                return Err(Self::forbidden());
            }
            if self.target.as_ref().is_some_and(|target| {
                !body.authorization.targets.iter().any(|authorization| {
                    authorization.resource_type == *mutation.target.resource_type()
                        && authorization
                            .resource_name
                            .as_ref()
                            .is_some_and(|name| name == mutation.target.name())
                        && authorization.execution_ref.as_ref() == Some(target)
                })
            }) {
                return Err(Self::invalid("guest-target-authorization-target-mismatch"));
            }
            if !matches!(
                mutation.target.resource_type().as_str(),
                "Process" | "EphemeralProcess" | NIXOS_GENERATION_RESOURCE_TYPE
            ) {
                continue;
            }
            if let Some(canonical) = mutation.canonical_resource.as_deref() {
                self.parse_resource(&mutation.target, canonical)?;
            }
        }
        Ok(())
    }

    async fn commit_verified_with_fence(
        &self,
        sealed: SealedMutation,
        commit_fence: Option<CommitFence>,
    ) -> Result<StoreCommitResult, StoreError> {
        match &self.backend {
            GuestStoreBackend::Durable(store) => {
                if let Some(commit_fence) = commit_fence {
                    store
                        .commit_verified_with_fence(
                            sealed,
                            |body| self.validate_mutation_body(body),
                            move || commit_fence(),
                        )
                        .await
                } else {
                    store
                        .commit_verified_with(sealed, |body| self.validate_mutation_body(body))
                        .await
                }
            }
            GuestStoreBackend::Memory { acceptor, state } => {
                let opened = acceptor.open(sealed)?;
                self.validate_mutation_body(opened.body())?;
                let body = opened.into_body();
                let mut state = state
                    .lock()
                    .map_err(|_| Self::unavailable("guest-target-store-poisoned"))?;
                let mut resources = state.resources.clone();
                let next_revision = state
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| Self::unavailable("guest-target-store-revision-exhausted"))?;
                let mut changed = Vec::new();
                for prepared in body.mutations {
                    let mutation = prepared.mutation();
                    if mutation.zone != self.zone {
                        return Err(Self::invalid("guest-target-resource-zone-mismatch"));
                    }
                    if !Self::is_target_local_type(mutation.target.resource_type()) {
                        return Err(Self::forbidden());
                    }
                    let current = resources.get(&mutation.target).cloned();
                    match mutation.expected {
                        ExpectedRevision::CreateAbsent if current.is_some() => {
                            return Err(StoreError::new(
                                StoreErrorKind::ResourceAlreadyExists,
                                Some(ZoneRevision::new(state.revision)),
                                None,
                                RetryClass::Never,
                                "guest-target-resource-already-exists",
                            ));
                        }
                        ExpectedRevision::Exact(expected)
                            if current
                                .as_ref()
                                .is_none_or(|resource| resource.revision != expected) =>
                        {
                            return Err(Self::conflict(state.revision));
                        }
                        _ => {}
                    }
                    if let Some(expected_uid) = mutation.expected_uid.as_ref()
                        && current
                            .as_ref()
                            .is_some_and(|resource| &resource.uid != expected_uid)
                    {
                        return Err(Self::conflict(state.revision));
                    }
                    if mutation.kind == ResourceMutationKind::Delete {
                        let removed =
                            resources.remove(&mutation.target).ok_or_else(Self::not_found)?;
                        changed.push(removed);
                        continue;
                    }
                    let canonical = mutation
                        .canonical_resource
                        .clone()
                        .or_else(|| current.as_ref().map(|resource| resource.canonical_json.clone()))
                        .ok_or_else(|| Self::invalid("guest-target-resource-body-missing"))?;
                    let (envelope_uid, generation) = self.parse_resource(&mutation.target, &canonical)?;
                    let uid = prepared
                        .resource_uid()
                        .cloned()
                        .unwrap_or(envelope_uid);
                    if current
                        .as_ref()
                        .is_some_and(|resource| resource.uid != uid)
                    {
                        return Err(Self::conflict(state.revision));
                    }
                    let payload_digest = prepared
                        .payload_digest()
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            d2b_contracts_resource::v3::canonical_digest(
                                d2b_contracts_resource::v3::resource_schema::RESOURCE_ENVELOPE_DOMAIN_TAG,
                                &canonical,
                            )
                        });
                    let resource = StoredResource {
                        resource_ref: mutation.target.clone(),
                        zone: self.zone.clone(),
                        uid,
                        generation,
                        revision: ZoneRevision::new(next_revision),
                        canonical_json: canonical,
                        payload_digest,
                    };
                    resources.insert(mutation.target.clone(), resource.clone());
                    changed.push(resource);
                }
                state.revision = next_revision;
                state.resources = resources;
                Ok(StoreCommitResult {
                    resources: changed,
                    revision: ZoneRevision::new(next_revision),
                })
            }
        }
    }
}

impl ResourceStoreBackend for GuestResourceStore {
    async fn get(&self, request: StoreGetRequest) -> Result<StoredResource, StoreError> {
        if request.zone != self.zone {
            return Err(Self::not_found());
        }
        if !Self::is_target_local_type(request.target.resource_type()) {
            return Err(Self::forbidden());
        }
        match &self.backend {
            GuestStoreBackend::Durable(store) => store.get(request).await,
            GuestStoreBackend::Memory { state, .. } => {
                let state = state
                    .lock()
                    .map_err(|_| Self::unavailable("guest-target-store-poisoned"))?;
                let resource = state
                    .resources
                    .get(&request.target)
                    .ok_or_else(Self::not_found)?;
                if request
                    .expected_uid
                    .as_ref()
                    .is_some_and(|uid| uid != &resource.uid)
                {
                    return Err(Self::not_found());
                }
                Ok(resource.clone())
            }
        }
    }

    async fn list(&self, request: StoreListRequest) -> Result<StoreListResult, StoreError> {
        if request.zone != self.zone {
            return Err(Self::not_found());
        }
        if request
            .resource_types
            .iter()
            .any(|resource_type| !Self::is_target_local_type(resource_type))
        {
            return Err(Self::forbidden());
        }
        match &self.backend {
            GuestStoreBackend::Durable(store) => store.list(request).await,
            GuestStoreBackend::Memory { state, .. } => {
                let state = state
                    .lock()
                    .map_err(|_| Self::unavailable("guest-target-store-poisoned"))?;
                let mut resources = state
                    .resources
                    .values()
                    .filter(|resource| {
                        (request.resource_types.is_empty()
                            || request
                                .resource_types
                                .iter()
                                .any(|kind| kind == resource.resource_ref.resource_type()))
                            && (request.resource_names.is_empty()
                                || request
                                    .resource_names
                                    .iter()
                                    .any(|name| name == resource.resource_ref.name()))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let page_size = usize::try_from(request.page_size).unwrap_or(usize::MAX);
                let truncated = resources.len() > page_size;
                resources.truncate(page_size);
                Ok(StoreListResult {
                    resources,
                    snapshot_revision: ZoneRevision::new(state.revision),
                    next_cursor: None,
                    truncated,
                })
            }
        }
    }

    async fn watch(&self, request: StoreWatchRequest) -> Result<StoreWatchReceipt, StoreError> {
        if request.zone != self.zone {
            return Err(Self::not_found());
        }
        if request
            .resource_types
            .iter()
            .any(|resource_type| !Self::is_target_local_type(resource_type))
        {
            return Err(Self::forbidden());
        }
        match &self.backend {
            GuestStoreBackend::Durable(store) => store.watch(request).await,
            GuestStoreBackend::Memory { state, .. } => {
                let mut state = state
                    .lock()
                    .map_err(|_| Self::unavailable("guest-target-store-poisoned"))?;
                state.next_watch = state.next_watch.saturating_add(1);
                Ok(StoreWatchReceipt {
                    stream_name: format!("{WATCH_STREAM_PREFIX}-{}", state.next_watch),
                    snapshot_revision: ZoneRevision::new(state.revision),
                })
            }
        }
    }

    async fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> Result<StoreResolvedIdentity, StoreError> {
        let resource = self
            .get(StoreGetRequest {
                operation: request.operation,
                zone: request.zone,
                target: request.target,
                expected_uid: request.expected_uid,
                projection: d2b_resource_store::StoreProjection::MetadataOnly,
            })
            .await?;
        Ok(StoreResolvedIdentity {
            zone: resource.zone,
            resource_ref: resource.resource_ref,
            uid: resource.uid,
            generation: resource.generation,
            revision: resource.revision,
        })
    }

    async fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> Result<StoredSchema, StoreError> {
        if request.zone != self.zone {
            return Err(Self::not_found());
        }
        if !Self::is_target_local_type(&request.resource_type) {
            return Err(Self::forbidden());
        }
        match &self.backend {
            GuestStoreBackend::Durable(store) => store.inspect_schema(request).await,
            GuestStoreBackend::Memory { .. } => {
                let canonical = d2b_contracts_resource::v3::CanonicalJsonValue::parse(SCHEMA_BYTES)
                    .map_err(|_| Self::invalid("guest-target-schema-invalid"))?
                    .to_canonical_bytes();
                Ok(StoredSchema {
                    resource_type: request.resource_type,
                    payload_digest: d2b_contracts_resource::v3::canonical_digest(
                        SCHEMA_DOMAIN_TAG,
                        &canonical,
                    ),
                    canonical_json: canonical,
                })
            }
        }
    }

    async fn commit_verified(
        &self,
        sealed: SealedMutation,
    ) -> Result<StoreCommitResult, StoreError> {
        self.commit_verified_with_fence(sealed, None).await
    }

}

fn open_owned_file(path: &Path) -> Result<File, GuestResourceRuntimeError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| GuestResourceRuntimeError::Store)?;
    let metadata = file
        .metadata()
        .map_err(|_| GuestResourceRuntimeError::Store)?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(GuestResourceRuntimeError::Store);
    }
    Ok(file)
}

fn map_store_error(error: StoreError) -> GuestResourceRuntimeError {
    if error.kind() == StoreErrorKind::StoreQuarantined {
        GuestResourceRuntimeError::StoreQuarantined
    } else {
        GuestResourceRuntimeError::Store
    }
}

pub struct SessionBoundStore {
    store: Arc<GuestResourceStore>,
    active_generation: Arc<Mutex<Option<u64>>>,
    generation: u64,
}

impl SessionBoundStore {
    fn ensure_current(&self) -> Result<(), StoreError> {
        let active = self
            .active_generation
            .lock()
            .map_err(|_| GuestResourceStore::unavailable("guest-target-session-state-poisoned"))?;
        if *active != Some(self.generation) {
            return Err(GuestResourceStore::unavailable(
                "guest-target-session-stale",
            ));
        }
        Ok(())
    }
}

impl ResourceStoreBackend for SessionBoundStore {
    async fn get(&self, request: StoreGetRequest) -> Result<StoredResource, StoreError> {
        self.ensure_current()?;
        self.store.get(request).await
    }

    async fn list(&self, request: StoreListRequest) -> Result<StoreListResult, StoreError> {
        self.ensure_current()?;
        self.store.list(request).await
    }

    async fn watch(&self, request: StoreWatchRequest) -> Result<StoreWatchReceipt, StoreError> {
        self.ensure_current()?;
        self.store.watch(request).await
    }

    async fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> Result<StoreResolvedIdentity, StoreError> {
        self.ensure_current()?;
        self.store.resolve_ref(request).await
    }

    async fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> Result<StoredSchema, StoreError> {
        self.ensure_current()?;
        self.store.inspect_schema(request).await
    }

    async fn commit_verified(
        &self,
        mutation: SealedMutation,
    ) -> Result<StoreCommitResult, StoreError> {
        self.ensure_current()?;
        let active_generation = Arc::clone(&self.active_generation);
        let generation = self.generation;
        let commit_fence: CommitFence = Arc::new(move || {
            let active = active_generation.lock().map_err(|_| {
                GuestResourceStore::unavailable("guest-target-session-state-poisoned")
            })?;
            if *active == Some(generation) {
                Ok(())
            } else {
                Err(GuestResourceStore::unavailable("guest-target-session-stale"))
            }
        });
        self.store
            .commit_verified_with_fence(mutation, Some(commit_fence))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_resource_store::mutation_seal::StoreSealIdentity;

    fn test_identity() -> GuestIdentity {
        GuestIdentity::new(
            ResourceRef::parse("Guest/work").expect("guest ref"),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("guest uid"),
            ZoneId::parse("work").expect("zone"),
            crate::guest_mode::BootIdentity::from_kernel_boot_id("u6-test-boot")
                .expect("boot identity"),
            d2b_contracts_resource::v3::identity::SessionPurpose::parse("zone-link")
                .expect("purpose"),
            d2b_contracts_resource::v3::SchemaFingerprint::parse(
                format!("sha256:{}", "1".repeat(64)),
            )
            .expect("schema"),
            d2b_contracts_resource::v3::identity::ReconnectGeneration::new(1)
                .expect("generation"),
            1,
            1,
            1,
        )
        .expect("identity")
    }

    #[tokio::test]
    async fn target_local_store_is_reopened_from_guest_state() {
        let directory = tempfile::tempdir().expect("state directory");
        let identity = test_identity();
        let first = GuestResourceRuntime::new(identity.clone(), directory.path())
            .await
            .expect("initial target-local runtime");
        assert!(directory.path().join("resource-store.redb").is_file());
        assert!(directory.path().join("resource-store.marker").is_file());
        drop(first);

        let second = GuestResourceRuntime::new(identity, directory.path())
            .await
            .expect("restarted target-local runtime");
        let listed = second
            .store
            .list(StoreListRequest {
                operation: d2b_resource_store::StoreOperationContext {
                    operation_id: "u6-reopen-list".to_owned(),
                    idempotency_key: None,
                    correlation_id: "u6-reopen-list".to_owned(),
                    trace_id: None,
                    deadline_ms: 1_000,
                },
                zone: ZoneId::parse("work").expect("zone"),
                resource_types: Vec::new(),
                resource_names: Vec::new(),
                filters: Vec::new(),
                page_size: 16,
                cursor: None,
                projection: d2b_resource_store::StoreProjection::MetadataOnly,
            })
            .await
            .expect("reopened store list");
        assert!(listed.resources.is_empty());
    }

    #[tokio::test]
    async fn partial_target_local_store_is_quarantined_without_repair() {
        let directory = tempfile::tempdir().expect("state directory");
        File::create(directory.path().join(STORE_FILE_NAME)).expect("database placeholder");
        let error = GuestResourceRuntime::new(test_identity(), directory.path())
            .await
            .expect_err("partial store must fail closed");
        assert_eq!(error, GuestResourceRuntimeError::StoreQuarantined);
        assert!(!directory.path().join(STORE_MARKER_NAME).exists());
    }

    #[test]
    fn target_local_store_rejects_zone_authority_types() {
        assert!(GuestResourceStore::is_target_local_type(
            &ResourceTypeName::parse("Process").expect("Process type")
        ));
        assert!(GuestResourceStore::is_target_local_type(
            &ResourceTypeName::parse("EphemeralProcess").expect("EphemeralProcess type")
        ));
        assert!(GuestResourceStore::is_target_local_type(
            &ResourceTypeName::parse("Endpoint").expect("Endpoint type")
        ));
        assert!(GuestResourceStore::is_target_local_type(
            &ResourceTypeName::parse(NIXOS_GENERATION_RESOURCE_TYPE)
                .expect("NixOS generation type")
        ));
        assert!(!GuestResourceStore::is_target_local_type(
            &ResourceTypeName::parse("Zone").expect("Zone type")
        ));
        assert!(!GuestResourceStore::is_target_local_type(
            &ResourceTypeName::parse("Role").expect("Role type")
        ));
    }

    #[tokio::test]
    async fn target_local_store_rejects_schema_reads_for_zone_types() {
        let zone = ZoneId::parse("work").expect("zone");
        let uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("store UID");
        let store_identity = StoreSealIdentity::new(
            d2b_resource_store::StoreSlot::new(STORE_SLOT).expect("store slot"),
            zone.clone(),
            uid,
        );
        let (_, acceptor) =
            d2b_resource_store::mutation_seal::mutation_seal_pair(store_identity);
        let store = GuestResourceStore::new_in_memory(zone.clone(), acceptor);
        let error = store
            .inspect_schema(StoreInspectSchemaRequest {
                operation: d2b_resource_store::StoreOperationContext {
                    operation_id: "schema".to_owned(),
                    idempotency_key: None,
                    correlation_id: "schema".to_owned(),
                    trace_id: None,
                    deadline_ms: 1,
                },
                zone,
                resource_type: ResourceTypeName::parse("Zone").expect("Zone type"),
            })
            .await
            .expect_err("Zone schema is not target-local");
        assert_eq!(error.kind(), StoreErrorKind::AuthorizationDenied);
    }

    #[tokio::test]
    async fn target_local_store_rejects_watches_for_zone_types() {
        let zone = ZoneId::parse("work").expect("zone");
        let uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("store UID");
        let store_identity = StoreSealIdentity::new(
            d2b_resource_store::StoreSlot::new(STORE_SLOT).expect("store slot"),
            zone.clone(),
            uid,
        );
        let (_, acceptor) =
            d2b_resource_store::mutation_seal::mutation_seal_pair(store_identity);
        let store = GuestResourceStore::new_in_memory(zone.clone(), acceptor);
        let error = store
            .watch(StoreWatchRequest {
                operation: d2b_resource_store::StoreOperationContext {
                    operation_id: "watch".to_owned(),
                    idempotency_key: None,
                    correlation_id: "watch".to_owned(),
                    trace_id: None,
                    deadline_ms: 1,
                },
                zone,
                resource_types: vec![ResourceTypeName::parse("Zone").expect("Zone type")],
                resource_names: Vec::new(),
                filters: Vec::new(),
                after_revision: ZoneRevision::new(0),
                initial_credits: 1,
                projection: d2b_resource_store::StoreProjection::MetadataOnly,
            })
            .await
            .expect_err("Zone watch is not target-local");
        assert_eq!(error.kind(), StoreErrorKind::AuthorizationDenied);
    }
}
