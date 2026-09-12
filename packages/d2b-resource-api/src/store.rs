//! Checked resource-store backend boundary.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use d2b_contracts::identity::{WrongPlane, refuse_wrong_plane};
use d2b_contracts_resource::v3::{ResourceTypeName, RetryClass};
use d2b_resource_store::{
    SealedMutation, StoreCommitResult, StoreError, StoreErrorKind, StoreGetRequest,
    StoreInspectSchemaRequest, StoreListRequest, StoreListResult, StoreMutation,
    StoreResolveRequest, StoreResolvedIdentity, StoreWatchReceipt, StoreWatchRequest, StoredResource,
    StoredSchema,
};

use crate::admission::{AdmittedMutation, StoreAdmissionBinding};

/// Reason code the legacy store facade renders for a wrong-plane refusal.
///
/// The wire kind is `resource-plane-unavailable` and the retry class is
/// `Never`; this reason names the partition failure (issue #507) so no reader
/// can mistake the refusal for absence, `ResourceNotFound`, or a transient
/// class. The typed refusal ([`WrongPlane`]) additionally names the resource
/// type and the refusing entry point; the closed store error shape carries
/// only the fixed reason code, so the typed detail is logged by the facade.
pub const WRONG_PLANE_REASON: &str = "wrong-plane-manager";

/// Caller tokens naming the legacy store facade's storage entry points: the
/// `caller` half of every [`WrongPlane`] refusal it renders.
pub const LEGACY_CALLER_GET: &str = "d2b-resource-api::store::legacy-get";
pub const LEGACY_CALLER_LIST: &str = "d2b-resource-api::store::legacy-list";
pub const LEGACY_CALLER_WATCH: &str = "d2b-resource-api::store::legacy-watch";
pub const LEGACY_CALLER_RESOLVE_REF: &str = "d2b-resource-api::store::legacy-resolve-ref";
pub const LEGACY_CALLER_INSPECT_SCHEMA: &str = "d2b-resource-api::store::legacy-inspect-schema";
pub const LEGACY_CALLER_COMMIT: &str = "d2b-resource-api::store::legacy-commit";

/// The legacy-plane fence posture of one store binding (issue #507).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyPlaneFence {
    /// Not the pre-v3 durable store (the manager plane, a Guest-local store,
    /// or a fixture backend): converted types are not this binding's to
    /// refuse.
    NotLegacy,
    /// A production legacy binding: every read or write whose subject
    /// resolves to the manager plane is refused with [`WrongPlane`].
    Enforced,
    /// A fixture/bench binding that deliberately drives legacy-plane behavior
    /// over rows of any type: the fence is not enforced. Production bindings
    /// never use this posture.
    FixtureOnly,
}

impl LegacyPlaneFence {
    /// Whether this posture refuses converted-type subjects.
    pub const fn is_enforced(self) -> bool {
        matches!(self, Self::Enforced)
    }
}

/// The typed wrong-plane refusal for one `get` request, or `Ok(())` when the
/// subject routes to the legacy plane.
pub fn legacy_get_refusal(request: &StoreGetRequest) -> Result<(), WrongPlane> {
    refuse_wrong_plane(request.target.resource_type().as_str(), LEGACY_CALLER_GET)
}

/// The typed wrong-plane refusal for one `resolve_ref` request.
pub fn legacy_resolve_ref_refusal(request: &StoreResolveRequest) -> Result<(), WrongPlane> {
    refuse_wrong_plane(
        request.target.resource_type().as_str(),
        LEGACY_CALLER_RESOLVE_REF,
    )
}

/// The typed wrong-plane refusal for one seeded-type collection request
/// (`list`/`watch`): the first requested type that routes to the manager
/// plane refuses the whole request, because a legacy collection over any
/// converted type would silently miss that type's rows.
pub fn legacy_collection_refusal(
    resource_types: &[ResourceTypeName],
    caller: &'static str,
) -> Result<(), WrongPlane> {
    for resource_type in resource_types {
        refuse_wrong_plane(resource_type.as_str(), caller)?;
    }
    Ok(())
}

/// The typed wrong-plane refusal for one `inspect_schema` request.
pub fn legacy_inspect_schema_refusal(
    request: &StoreInspectSchemaRequest,
) -> Result<(), WrongPlane> {
    refuse_wrong_plane(
        request.resource_type.as_str(),
        LEGACY_CALLER_INSPECT_SCHEMA,
    )
}

/// The typed wrong-plane refusal for one verified mutation: every mutation
/// target is checked before the sealed commit reaches the legacy store.
pub fn legacy_mutation_refusal(mutations: &[StoreMutation]) -> Result<(), WrongPlane> {
    for mutation in mutations {
        refuse_wrong_plane(mutation.target.resource_type().as_str(), LEGACY_CALLER_COMMIT)?;
    }
    Ok(())
}

/// Render one typed refusal into the store facade's closed error shape: the
/// distinct `resource-plane-unavailable` kind, `RetryClass::Never`, and the
/// `wrong-plane-manager` reason. Never `ResourceNotFound`, never retryable.
pub fn wrong_plane_store_error(refusal: &WrongPlane) -> StoreError {
    tracing::warn!(
        resource_type = refusal.resource_type(),
        caller = refusal.caller(),
        "legacy store access refused: the resource type is served by the manager plane",
    );
    StoreError::new(
        StoreErrorKind::ResourcePlaneUnavailable,
        None,
        None,
        RetryClass::Never,
        WRONG_PLANE_REASON,
    )
}

/// Map one typed refusal check through a fence posture: `Enforced` renders the
/// refusal (and logs the type + caller), every other posture admits the
/// access untouched.
pub fn fenced_result(
    posture: LegacyPlaneFence,
    check: Result<(), WrongPlane>,
) -> Result<(), StoreError> {
    match (posture.is_enforced(), check) {
        (true, Err(refusal)) => Err(wrong_plane_store_error(&refusal)),
        _ => Ok(()),
    }
}

/// Trusted persistence seam reached only after instance-bound admission verification.
///
/// A correctly wired production store accepts only evidence from its paired
/// issuer. A caller can construct a locally paired seal for a store it owns,
/// but foreign locally-paired seals are inert here: this store's acceptor
/// rejects them before the evidence reaches
/// [`ResourceStoreBackend::commit_verified`]. In production, the paired issuer
/// is retained by the native authorization path, so accepted evidence follows
/// a successful native authorization evaluation and is verified against this
/// store's identity.
///
/// This seal does not constrain the backend implementation. A backend could
/// ignore a verified mutation, change storage through another path, or omit
/// required transaction checks. Implementations are therefore part of the
/// trusted computing base: they must mutate only from the supplied
/// [`SealedMutation`], recheck its captured revisions in the write
/// transaction, preserve the store's structural and atomicity invariants, and
/// expose no independent mutation path. A production backend requires security
/// review and conformance tests for these obligations before it is registered.
pub trait ResourceStoreBackend: Send + Sync {
    /// The legacy-plane fence posture of this backend (issue #507).
    ///
    /// The checked store consults this before sealing a commit and before
    /// dispatching a read, so a converted-type access is refused at the
    /// storage boundary even when the backend's own implementation does not
    /// re-check. Store bindings that are not the pre-v3 durable store keep the
    /// default: the manager plane and Guest-local stores own types of both
    /// planes and are not the legacy partition's to refuse.
    fn legacy_plane_fence(&self) -> LegacyPlaneFence {
        LegacyPlaneFence::NotLegacy
    }

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
        mutation: SealedMutation,
    ) -> impl Future<Output = Result<StoreCommitResult, StoreError>> + Send;
}

/// API bridge that owns the concrete mutation-seal store binding.
///
/// A caller can construct a locally paired seal, but foreign locally-paired
/// seals are inert: a correctly wired production store accepts only evidence
/// from the issuer paired with its own acceptor.
///
/// ```compile_fail
/// use d2b_resource_api::RedbBackend;
/// use d2b_resource_store::SealedMutation;
///
/// fn forge() -> SealedMutation {
///     SealedMutation {}
/// }
/// ```
pub struct RedbBackend {
    store: Arc<d2b_resource_store_redb::RedbResourceStore>,
    plane_fence: LegacyPlaneFence,
    /// Latched true once the owning Zone runtime publishes its manager plane:
    /// from that moment the per-type fence is enforced (issue #507). A binding
    /// without a latch keeps its declared posture.
    manager_published: Option<Arc<AtomicBool>>,
}

impl core::fmt::Debug for RedbBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RedbBackend(<redacted>)")
    }
}

impl RedbBackend {
    /// Bind one owned legacy store with the per-type plane fence enforced
    /// (issue #507): every read or write whose subject resolves to the manager
    /// plane is refused with [`WrongPlane`].
    pub fn new(store: d2b_resource_store_redb::RedbResourceStore) -> Self {
        Self {
            store: Arc::new(store),
            plane_fence: LegacyPlaneFence::Enforced,
            manager_published: None,
        }
    }

    /// Bind the API to a store whose lifetime is owned by a Zone runtime.
    ///
    /// This is the fixture/bench binding: it deliberately drives legacy-plane
    /// behavior over rows of any type (contract tests for the pre-v3 adapter,
    /// operator-activation fixtures, toolkit benches), so the per-type fence
    /// is not enforced. Production bindings use [`Self::production`].
    pub const fn from_arc(store: Arc<d2b_resource_store_redb::RedbResourceStore>) -> Self {
        Self {
            store,
            plane_fence: LegacyPlaneFence::FixtureOnly,
            manager_published: None,
        }
    }

    /// Bind the API to a Zone-runtime-owned legacy store with the per-type
    /// plane fence enforced (issue #507). This is the always-enforced
    /// production binding.
    pub const fn production(store: Arc<d2b_resource_store_redb::RedbResourceStore>) -> Self {
        Self {
            store,
            plane_fence: LegacyPlaneFence::Enforced,
            manager_published: None,
        }
    }

    /// Bind the API to a Zone-runtime-owned legacy store for a zone whose
    /// manager plane publishes later (the daemon binding): before `published`
    /// latches, the binding behaves as a fixture binding, so a runtime that
    /// never publishes a plane (legacy boot, in-process fixtures) keeps the
    /// durable plane; once the manager authority exists, every converted-type
    /// access through this binding is refused (issue #507).
    pub const fn production_when_published(
        store: Arc<d2b_resource_store_redb::RedbResourceStore>,
        published: Arc<AtomicBool>,
    ) -> Self {
        Self {
            store,
            plane_fence: LegacyPlaneFence::FixtureOnly,
            manager_published: Some(published),
        }
    }

    /// The effective legacy-plane fence posture of this binding: a latched
    /// manager publication upgrades a deferred binding to `Enforced`.
    pub fn plane_fence(&self) -> LegacyPlaneFence {
        match &self.manager_published {
            Some(published) if published.load(Ordering::Acquire) => LegacyPlaneFence::Enforced,
            _ => self.plane_fence,
        }
    }

    /// The typed wrong-plane check for one subject type, naming the refusing
    /// access path. `Ok(())` when the type routes to the legacy plane or the
    /// binding's effective posture is not enforced.
    pub fn check_plane(
        &self,
        resource_type: &str,
        caller: &'static str,
    ) -> Result<(), WrongPlane> {
        if self.plane_fence().is_enforced() {
            refuse_wrong_plane(resource_type, caller)
        } else {
            Ok(())
        }
    }

    pub(crate) fn store_arc(&self) -> Arc<d2b_resource_store_redb::RedbResourceStore> {
        Arc::clone(&self.store)
    }
}

/// A backend backed by the durable Redb resource store. Controller-source
/// adapters bind through it (R35/F1: the Zone API backend may be wrapped by
/// the per-type partition and still expose its Redb store).
pub trait RedbStoreSource {
    fn redb_store_arc(&self) -> Result<Arc<d2b_resource_store_redb::RedbResourceStore>, StoreBindingError>;
}

impl RedbStoreSource for RedbBackend {
    fn redb_store_arc(&self) -> Result<Arc<d2b_resource_store_redb::RedbResourceStore>, StoreBindingError> {
        Ok(self.store_arc())
    }
}

/// One sealed-commit call boxed over the concrete backend (R35/F1: the Zone
/// API backend may be partition-wrapped; the controller-source commit path
/// captures the backend generically).
pub type BoxedCommitFn = std::sync::Arc<
    dyn Fn(AdmittedMutation) -> std::pin::Pin<
            Box<dyn Future<Output = Result<StoreCommitResult, StoreError>> + Send>,
        > + Send
        + Sync,
>;

/// Capture one checked store's sealed-commit path as a backend-agnostic
/// boxed call.
pub fn box_commit<S>(checked: Arc<CheckedResourceStore<S>>) -> BoxedCommitFn
where
    S: ResourceStoreBackend + 'static,
{
    std::sync::Arc::new(move |admitted| {
        let checked = Arc::clone(&checked);
        Box::pin(async move { checked.commit(admitted).await })
    })
}

impl ResourceStoreBackend for RedbBackend {
    fn legacy_plane_fence(&self) -> LegacyPlaneFence {
        self.plane_fence()
    }

    async fn get(&self, request: StoreGetRequest) -> Result<StoredResource, StoreError> {
        fenced_result(self.plane_fence(), legacy_get_refusal(&request))?;
        self.store.get(request).await
    }

    async fn list(&self, request: StoreListRequest) -> Result<StoreListResult, StoreError> {
        fenced_result(
            self.plane_fence(),
            legacy_collection_refusal(&request.resource_types, LEGACY_CALLER_LIST),
        )?;
        self.store.list(request).await
    }

    async fn watch(&self, request: StoreWatchRequest) -> Result<StoreWatchReceipt, StoreError> {
        fenced_result(
            self.plane_fence(),
            legacy_collection_refusal(&request.resource_types, LEGACY_CALLER_WATCH),
        )?;
        self.store.watch(request).await
    }

    async fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> Result<StoreResolvedIdentity, StoreError> {
        fenced_result(self.plane_fence(), legacy_resolve_ref_refusal(&request))?;
        self.store.resolve_ref(request).await
    }

    async fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> Result<StoredSchema, StoreError> {
        fenced_result(self.plane_fence(), legacy_inspect_schema_refusal(&request))?;
        self.store.inspect_schema(request).await
    }

    async fn commit_verified(
        &self,
        mutation: SealedMutation,
    ) -> Result<StoreCommitResult, StoreError> {
        // The sealed body is opaque at this layer; the mutation targets are
        // fenced before sealing in `CheckedResourceStore::commit`.
        self.store.commit_verified(mutation).await
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

pub(crate) struct CheckedResourceStore<S> {
    backend: Arc<S>,
    admission: StoreAdmissionBinding,
}

impl<S> Clone for CheckedResourceStore<S> {
    fn clone(&self) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
            admission: self.admission.clone(),
        }
    }
}

impl<S> CheckedResourceStore<S> {
    pub(super) const fn new(backend: Arc<S>, admission: StoreAdmissionBinding) -> Self {
        Self { backend, admission }
    }

    pub(crate) fn backend(&self) -> Arc<S> {
        Arc::clone(&self.backend)
    }
}

impl<S> CheckedResourceStore<S>
where
    S: ResourceStoreBackend,
{
    pub(crate) fn get(
        &self,
        request: StoreGetRequest,
    ) -> impl Future<Output = Result<StoredResource, StoreError>> + Send {
        let fence = fenced_result(
            self.backend.legacy_plane_fence(),
            legacy_get_refusal(&request),
        );
        async move {
            fence?;
            self.backend.get(request).await
        }
    }

    pub(crate) fn list(
        &self,
        request: StoreListRequest,
    ) -> impl Future<Output = Result<StoreListResult, StoreError>> + Send {
        let fence = fenced_result(
            self.backend.legacy_plane_fence(),
            legacy_collection_refusal(&request.resource_types, LEGACY_CALLER_LIST),
        );
        async move {
            fence?;
            self.backend.list(request).await
        }
    }

    pub(crate) fn watch(
        &self,
        request: StoreWatchRequest,
    ) -> impl Future<Output = Result<StoreWatchReceipt, StoreError>> + Send {
        let fence = fenced_result(
            self.backend.legacy_plane_fence(),
            legacy_collection_refusal(&request.resource_types, LEGACY_CALLER_WATCH),
        );
        async move {
            fence?;
            self.backend.watch(request).await
        }
    }

    pub(crate) fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> impl Future<Output = Result<StoreResolvedIdentity, StoreError>> + Send {
        let fence = fenced_result(
            self.backend.legacy_plane_fence(),
            legacy_resolve_ref_refusal(&request),
        );
        async move {
            fence?;
            self.backend.resolve_ref(request).await
        }
    }

    pub(crate) fn inspect_schema(
        &self,
        request: StoreInspectSchemaRequest,
    ) -> impl Future<Output = Result<StoredSchema, StoreError>> + Send {
        let fence = fenced_result(
            self.backend.legacy_plane_fence(),
            legacy_inspect_schema_refusal(&request),
        );
        async move {
            fence?;
            self.backend.inspect_schema(request).await
        }
    }

    pub(crate) fn commit(
        &self,
        mutation: AdmittedMutation,
    ) -> impl Future<Output = Result<StoreCommitResult, StoreError>> + Send {
        // The mutation targets are visible here, before the seal hides them:
        // this is the only layer that can fence a commit whose subject is a
        // converted type (issue #507).
        let fence = fenced_result(
            self.backend.legacy_plane_fence(),
            legacy_mutation_refusal(mutation.mutations()),
        );
        let sealed = fence.and_then(|()| {
            self.admission
                .verify(mutation)
                .and_then(|body| self.admission.seal(body))
        });
        async move { self.backend.commit_verified(sealed?).await }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::identity::{ResourcePlane, resource_plane};
    use d2b_contracts_resource::v3::{
        ConfigurationGeneration, ResourceRef, ResourceUid, Timestamp, V3_CONVERTED_RESOURCE_TYPES,
        ZoneId,
    };
    use d2b_resource_store::mutation_seal::mutation_seal_pair;
    use d2b_resource_store::{
        ExpectedRevision, PolicySnapshot, ResourceMutationKind, StoreOperationContext,
        StoreProjection, StoreSlot,
    };
    use d2b_resource_store_redb::{RedbResourceStore, StoreIdentity, write_provisioning_marker};
    use std::fs::OpenOptions;

    const TEST_ZONE: &str = "work";
    /// A stable type outside the converted registry: the legacy plane owns it.
    const LEGACY_TYPE: &str = "vendor-extension.d2bus.org.Report";

    fn identity() -> StoreIdentity {
        StoreIdentity::new(
            StoreSlot::new(0).unwrap(),
            ResourceUid::parse("11111111-1111-4111-8111-111111111111").unwrap(),
            ZoneId::parse(TEST_ZONE).unwrap(),
            ResourceUid::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            Timestamp::parse("2026-07-31T00:00:00.000Z").unwrap(),
            PolicySnapshot {
                policy_revision: 7,
                api_catalog_revision: 8,
                active_configuration_revision: ConfigurationGeneration::new(9).unwrap(),
                controller_generation: None,
            },
        )
    }

    async fn provision_store() -> (
        tempfile::TempDir,
        Arc<RedbResourceStore>,
        d2b_resource_store::mutation_seal::MutationSealIssuer,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.path().join("store.redb"))
            .unwrap();
        let mut marker = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(directory.path().join("store.marker"))
            .unwrap();
        let identity = identity();
        write_provisioning_marker(&mut marker, &identity).unwrap();
        let (issuer, acceptor) = mutation_seal_pair(identity.seal_identity());
        let store = RedbResourceStore::provision_owned(file, marker, identity, acceptor)
            .await
            .unwrap();
        (directory, Arc::new(store), issuer)
    }

    fn get_request(resource_type: &str) -> StoreGetRequest {
        StoreGetRequest {
            operation: StoreOperationContext {
                operation_id: "wrong-plane-table-get".to_owned(),
                idempotency_key: None,
                correlation_id: "wrong-plane-table-get".to_owned(),
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: ZoneId::parse(TEST_ZONE).unwrap(),
            target: ResourceRef::parse(&format!("{resource_type}/table-row")).unwrap(),
            expected_uid: None,
            projection: StoreProjection::Full,
        }
    }

    fn list_request(resource_type: &str) -> StoreListRequest {
        StoreListRequest {
            operation: StoreOperationContext {
                operation_id: "wrong-plane-table-list".to_owned(),
                idempotency_key: None,
                correlation_id: "wrong-plane-table-list".to_owned(),
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: ZoneId::parse(TEST_ZONE).unwrap(),
            resource_types: vec![ResourceTypeName::parse(resource_type).unwrap()],
            resource_names: Vec::new(),
            filters: Vec::new(),
            page_size: 16,
            cursor: None,
            projection: StoreProjection::Full,
        }
    }

    fn resolve_request(resource_type: &str) -> StoreResolveRequest {
        StoreResolveRequest {
            operation: StoreOperationContext {
                operation_id: "wrong-plane-table-resolve".to_owned(),
                idempotency_key: None,
                correlation_id: "wrong-plane-table-resolve".to_owned(),
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: ZoneId::parse(TEST_ZONE).unwrap(),
            target: ResourceRef::parse(&format!("{resource_type}/table-row")).unwrap(),
            expected_uid: None,
        }
    }

    fn inspect_request(resource_type: &str) -> StoreInspectSchemaRequest {
        StoreInspectSchemaRequest {
            operation: StoreOperationContext {
                operation_id: "wrong-plane-table-inspect".to_owned(),
                idempotency_key: None,
                correlation_id: "wrong-plane-table-inspect".to_owned(),
                trace_id: None,
                deadline_ms: 10_000,
            },
            zone: ZoneId::parse(TEST_ZONE).unwrap(),
            resource_type: ResourceTypeName::parse(resource_type).unwrap(),
        }
    }

    fn delete_mutation(resource_type: &str) -> StoreMutation {
        StoreMutation {
            kind: ResourceMutationKind::Delete,
            zone: ZoneId::parse(TEST_ZONE).unwrap(),
            target: ResourceRef::parse(&format!("{resource_type}/table-row")).unwrap(),
            expected: ExpectedRevision::CreateAbsent,
            expected_uid: None,
            owner: None,
            canonical_resource: None,
            add_finalizers: Vec::new(),
            remove_finalizers: Vec::new(),
            wait_for_reconcile: false,
            reconcile_deadline_ms: None,
            configuration_generation: None,
            assignment: None,
        }
    }

    /// The wrong-plane refusal's closed store rendering: distinct kind, never
    /// retryable, and never `ResourceNotFound`.
    fn assert_wrong_plane_store_error(error: &StoreError, resource_type: &str) {
        assert_eq!(
            error.kind(),
            StoreErrorKind::ResourcePlaneUnavailable,
            "{resource_type}: the refusal must be the distinct plane kind"
        );
        assert_eq!(
            error.retry_class(),
            RetryClass::Never,
            "{resource_type}: a wrong-plane access is never retryable"
        );
        assert_eq!(error.retry_after_ms(), None, "{resource_type}");
        assert_eq!(error.reason_code(), WRONG_PLANE_REASON, "{resource_type}");
        assert_ne!(
            error.kind(),
            StoreErrorKind::ResourceNotFound,
            "{resource_type}: a wrong-plane access is never absence"
        );
    }

    /// Table-driven fence at the API store facade (issue #507): for every
    /// converted type, the legacy path refuses with `WrongPlane` naming the
    /// type and the caller - and renders as the non-retryable plane error -
    /// while a type outside the registry still reaches the legacy store.
    #[tokio::test]
    async fn every_converted_type_is_refused_by_the_fenced_legacy_facade() {
        let (_directory, store, _issuer) = provision_store().await;
        let backend = RedbBackend::production(Arc::clone(&store));

        for resource_type in V3_CONVERTED_RESOURCE_TYPES {
            assert_eq!(
                resource_plane(resource_type),
                ResourcePlane::Manager,
                "{resource_type}: the converted registry is the plane authority"
            );

            let refusal = backend
                .check_plane(resource_type, LEGACY_CALLER_GET)
                .expect_err("a converted type cannot be read through the legacy facade");
            assert_eq!(refusal.resource_type(), resource_type);
            assert_eq!(refusal.caller(), LEGACY_CALLER_GET);
            assert!(!refusal.retryable(), "{resource_type}");

            let error = backend
                .get(get_request(resource_type))
                .await
                .expect_err("the legacy get must refuse a converted type");
            assert_wrong_plane_store_error(&error, resource_type);

            let error = backend
                .list(list_request(resource_type))
                .await
                .expect_err("the legacy list must refuse a converted type");
            assert_wrong_plane_store_error(&error, resource_type);

            let error = backend
                .resolve_ref(resolve_request(resource_type))
                .await
                .expect_err("the legacy resolve_ref must refuse a converted type");
            assert_wrong_plane_store_error(&error, resource_type);

            let error = backend
                .inspect_schema(inspect_request(resource_type))
                .await
                .expect_err("the legacy schema read must refuse a converted type");
            assert_wrong_plane_store_error(&error, resource_type);

            let refusal = legacy_mutation_refusal(&[delete_mutation(resource_type)])
                .expect_err("the legacy commit must refuse a converted target");
            assert_eq!(refusal.resource_type(), resource_type);
            assert_eq!(refusal.caller(), LEGACY_CALLER_COMMIT);
        }

        // The non-converted control: the same facade still serves the legacy
        // store for a type outside the registry (its honest answer is the
        // store's own not-found, not the partition refusal).
        assert_eq!(backend.check_plane(LEGACY_TYPE, LEGACY_CALLER_GET), Ok(()));
        let error = backend
            .get(get_request(LEGACY_TYPE))
            .await
            .expect_err("the fixture store holds no such row");
        assert_eq!(
            error.kind(),
            StoreErrorKind::ResourceNotFound,
            "a legacy-owned type reaches the store and keeps its own answer"
        );
        let page = backend
            .list(list_request(LEGACY_TYPE))
            .await
            .expect("a legacy-owned type collection is served by the store");
        assert!(page.resources.is_empty());
        assert!(legacy_mutation_refusal(&[delete_mutation(LEGACY_TYPE)]).is_ok());
    }

    /// A fixture/bench legacy binding keeps the raw behavior (the explicit
    /// escape hatch for legacy-plane contract tests), so the fence cannot be
    /// confused with a store-level restriction.
    #[tokio::test]
    async fn fixture_legacy_binding_keeps_the_raw_store_behavior() {
        let (_directory, store, _issuer) = provision_store().await;
        let backend = RedbBackend::from_arc(Arc::clone(&store));
        assert_eq!(backend.plane_fence(), LegacyPlaneFence::FixtureOnly);
        let error = backend
            .get(get_request("Volume"))
            .await
            .expect_err("the fixture store holds no Volume row");
        assert_eq!(
            error.kind(),
            StoreErrorKind::ResourceNotFound,
            "the fixture binding is deliberately not fenced"
        );
    }

    /// The daemon binding's deferred posture: dormant until the zone's manager
    /// plane publishes (legacy boot and fixtures keep the durable plane), then
    /// enforced for every converted type - so production cannot bypass the
    /// partition once the manager authority exists.
    #[tokio::test]
    async fn deferred_binding_fences_from_the_manager_publication_latch() {
        let (_directory, store, _issuer) = provision_store().await;
        let published = Arc::new(AtomicBool::new(false));
        let backend = RedbBackend::production_when_published(
            Arc::clone(&store),
            Arc::clone(&published),
        );

        assert_eq!(backend.plane_fence(), LegacyPlaneFence::FixtureOnly);
        let error = backend
            .get(get_request("Volume"))
            .await
            .expect_err("the fixture store holds no Volume row");
        assert_eq!(
            error.kind(),
            StoreErrorKind::ResourceNotFound,
            "before publication the deferred binding keeps the durable plane"
        );
        assert_eq!(backend.check_plane("Volume", LEGACY_CALLER_GET), Ok(()));

        published.store(true, Ordering::Release);
        assert_eq!(backend.plane_fence(), LegacyPlaneFence::Enforced);
        let refusal = backend
            .check_plane("Volume", LEGACY_CALLER_GET)
            .expect_err("after publication the converted type is refused");
        assert_eq!(refusal.resource_type(), "Volume");
        assert_eq!(refusal.caller(), LEGACY_CALLER_GET);
        let error = backend
            .get(get_request("Volume"))
            .await
            .expect_err("the fenced binding refuses before the store");
        assert_wrong_plane_store_error(&error, "Volume");
    }
}
