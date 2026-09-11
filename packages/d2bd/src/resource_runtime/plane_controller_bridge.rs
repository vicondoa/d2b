//! Bridge from the old-plane controller readers onto the new plane's
//! manager view (G5, KTD3/KTD4).
//!
//! Since KTD4 the bundle's controller-class `Process` rows (and every other
//! converted type) are served by the per-zone manager, while the controller
//! session machinery and the Core `Provider` handler still read the redb
//! store. Two readers are blinded by that split:
//!
//! - the controller-session path (`controller_context_is_current`,
//!   `persist_controller_session_evidence`, `fence_process_resources`), which
//!   must see a manager-served controller row as current, and
//! - the Core `Provider` handler's dependency observation, which must see the
//!   controller `Process` rows it owns (and its provider `Volume` rows)
//!   before it can report dependencies ready.
//!
//! This module owns the read-only seam over the manager's existing view
//! surface ([`ResourceManagerClient::get`] / [`ResourceManagerClient::list`])
//! plus the two adapters the daemon wires:
//!
//! - [`ControllerPlaneView`] answers one `Process` row read for the
//!   controller-session path; a manager RPC failure is never reported as
//!   absence, and a row the manager does not hold (`Ok(None)`) keeps the
//!   caller on the durable store path.
//! - [`PlaneAwareControllerApi`] decorates the registered controller API:
//!   for a `Provider` target it appends one synthesized
//!   [`DependencySnapshot`] per manager-served `Process`/`Volume` row the
//!   Provider owns. Readiness comes from
//!   [`ResourceView::observed_status`] (generation-filtered: a stale `Ready`
//!   never passes), and a controller `Process` row's session evidence is the
//!   live admitted session ([`LiveControllerSessionEvidence`]) - the
//!   manager has no durable status channel (R11/AE6), so the live session
//!   *is* the evidence and every unknown fails closed (`ready: false`).
//!
//! Nothing here writes: converted rows keep the manager's single-writer
//! discipline, and rows the manager does not serve keep the durable store.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
};
use d2b_core_controller::{
    ChangeRecord, CommitOutcome, ControllerDescriptor, DependencySnapshot, FreshSnapshot,
    InitialList, OperationContext, ReconcileContext, ReconcilePlan, ReconcileProjection,
    ReconcileResult, RegisteredControllerApi, ResourceKey, ResourceSnapshot, SourceError,
    StatusPersistence, WatchFailure,
};
use d2b_resource_runtime::error::ResourceError;
use d2b_resource_runtime::identity::ResourceKey as ManagerKey;
use d2b_resource_runtime::manager::{
    ResourceManagerClient, ResourceSelector, ResourceView,
};
use d2b_resource_runtime::resource::ResourceStatus;
use serde_json::{Value, json};

/// The converted dependency types a `Provider` consumes from the owner-scoped
/// core handler set: the controller `Process` rows and the provider `Volume`
/// rows are manager rows since KTD4.
pub(crate) const PROVIDER_OWNED_CONVERTED_TYPES: [&str; 2] = ["Process", "Volume"];

// ---------------------------------------------------------------------------
// Controller-session path seam
// ---------------------------------------------------------------------------

/// The manager view the controller-session path reads one `Process` row
/// from. `Ok(None)` means the manager does not serve the row (an unconverted
/// or legacy row: the durable store owns it); an RPC failure is an error and
/// never absence.
#[async_trait]
pub(crate) trait ControllerPlaneView: Send + Sync + 'static {
    async fn process_view(&self, process_ref: &ResourceRef) -> Result<Option<ResourceView>, ResourceError>;
}

/// Production seam over one zone's manager client (the plane's published
/// client; cloned cheaply per read).
#[derive(Clone)]
pub(crate) struct ManagerControllerPlaneView {
    client: ResourceManagerClient,
    zone: ZoneId,
}

impl ManagerControllerPlaneView {
    pub(crate) fn new(client: ResourceManagerClient, zone: ZoneId) -> Self {
        Self { client, zone }
    }
}

#[async_trait]
impl ControllerPlaneView for ManagerControllerPlaneView {
    async fn process_view(&self, process_ref: &ResourceRef) -> Result<Option<ResourceView>, ResourceError> {
        if process_ref.resource_type().as_str() != "Process" {
            return Ok(None);
        }
        self.client
            .get(ManagerKey::new(
                self.zone.as_str(),
                process_ref.resource_type().as_str(),
                process_ref.name().as_str(),
            ))
            .await
    }
}

/// Production seam over the composition's published per-zone plane table.
///
/// The composition hands this table to a runtime at the top of its per-zone
/// loop and fills it only after the loop, so a view resolved at attach time
/// is permanently empty. This seam resolves the zone's plane per read - the
/// same lazy lookup [`ManagerPlaneDependencyRows`] uses - so the
/// controller-session path sees manager-served rows no matter when the
/// composition publishes them.
pub(crate) struct PublishedPlaneControllerView {
    planes: Arc<parking_lot::Mutex<HashMap<String, Arc<crate::resource_plane_v3::ResourcePlaneV3>>>>,
    zone: ZoneId,
}

impl PublishedPlaneControllerView {
    pub(crate) fn new(
        planes: Arc<parking_lot::Mutex<HashMap<String, Arc<crate::resource_plane_v3::ResourcePlaneV3>>>>,
        zone: ZoneId,
    ) -> Self {
        Self { planes, zone }
    }
}

#[async_trait]
impl ControllerPlaneView for PublishedPlaneControllerView {
    async fn process_view(&self, process_ref: &ResourceRef) -> Result<Option<ResourceView>, ResourceError> {
        let Some(plane) = self.planes.lock().get(self.zone.as_str()).cloned() else {
            return Ok(None);
        };
        ManagerControllerPlaneView::new(plane.client().clone(), self.zone.clone())
            .process_view(process_ref)
            .await
    }
}

// ---------------------------------------------------------------------------
// Live controller-session evidence
// ---------------------------------------------------------------------------

/// The live admitted controller session for one controller `Process` row.
///
/// The manager-served row carries no durable status (R11/AE6), so the
/// evidence the Core `Provider` handler reads as
/// `status.resource.controllerSession` is provided from the authoritative
/// live state: the admitted session and its live service task. Every
/// uncertainty - not admitted, a different row identity or generation, a
/// finished task - answers `None`, so no caller can synthesize `ready: true`.
pub(crate) trait LiveControllerSessionEvidence: Send + Sync + 'static {
    fn controller_session_evidence(
        &self,
        process_ref: &ResourceRef,
        process_uid: &ResourceUid,
        generation: ResourceGeneration,
    ) -> Option<Value>;
}

// ---------------------------------------------------------------------------
// Provider dependency rows
// ---------------------------------------------------------------------------

/// The manager rows of one converted type in the plane's zone.
#[async_trait]
pub(crate) trait PlaneDependencyRows: Send + Sync + 'static {
    async fn rows(&self, resource_type: &str) -> Result<Vec<ResourceView>, ResourceError>;
}

/// Production row source over the plane's manager client.
pub(crate) struct ManagerPlaneDependencyRows {
    client: ResourceManagerClient,
    zone: String,
}

impl ManagerPlaneDependencyRows {
    pub(crate) fn new(client: ResourceManagerClient, zone: &ZoneId) -> Self {
        Self {
            client,
            zone: zone.as_str().to_owned(),
        }
    }
}

#[async_trait]
impl PlaneDependencyRows for ManagerPlaneDependencyRows {
    async fn rows(&self, resource_type: &str) -> Result<Vec<ResourceView>, ResourceError> {
        self.client
            .list(ResourceSelector {
                zone: Some(self.zone.clone()),
                type_name: Some(resource_type.to_owned()),
                owner: None,
            })
            .await
    }
}

// ---------------------------------------------------------------------------
// The Provider dependency merge (the reader bridge)
// ---------------------------------------------------------------------------

/// Merge the manager-served rows one `Provider` owns into a Core dependency
/// list, so `provider_observation` counts them.
pub(crate) struct PlaneProviderDependencies {
    rows: Arc<dyn PlaneDependencyRows>,
    sessions: Arc<dyn LiveControllerSessionEvidence>,
}

impl PlaneProviderDependencies {
    pub(crate) fn new(
        rows: Arc<dyn PlaneDependencyRows>,
        sessions: Arc<dyn LiveControllerSessionEvidence>,
    ) -> Self {
        Self { rows, sessions }
    }

    /// Append one synthesized snapshot per manager row of a converted type
    /// whose authored owner is `provider_ref`. Rows already present in the
    /// list keep the list's existing entry; a manager failure fails the read
    /// (the runner retries) instead of silently dropping the Provider's
    /// children.
    pub(crate) async fn merge(
        &self,
        provider_ref: &ResourceRef,
        provider: &ResourceSnapshot,
        dependencies: &mut Vec<DependencySnapshot>,
    ) -> Result<(), SourceError> {
        for resource_type in PROVIDER_OWNED_CONVERTED_TYPES {
            let rows = self.rows.rows(resource_type).await.map_err(|error| {
                tracing::warn!(
                    provider = %provider_ref.to_canonical_string(),
                    resource_type,
                    error = %error,
                    "provider dependency bridge: manager rows unavailable",
                );
                SourceError::Unavailable
            })?;
            for view in rows {
                if crate::resource_plane_v3::decode_metadata_owner_ref(&view.metadata)
                    .as_ref()
                    != Some(provider_ref)
                {
                    continue;
                }
                let Some(snapshot) = self.synthesize(&view, provider) else {
                    continue;
                };
                if dependencies
                    .iter()
                    .any(|existing| existing.resource().key() == snapshot.resource().key())
                {
                    continue;
                }
                dependencies.push(snapshot);
            }
        }
        Ok(())
    }

    /// One manager row as the Core dependency snapshot `provider_observation`
    /// reads: identity and owner from the row, `phase`/`observedGeneration`
    /// from the generation-filtered live status, and the live session
    /// evidence for a controller `Process` row.
    fn synthesize(
        &self,
        view: &ResourceView,
        provider: &ResourceSnapshot,
    ) -> Option<DependencySnapshot> {
        let resource_ref = ResourceRef::parse(&format!(
            "{}/{}",
            view.key.type_name, view.key.name
        ))
        .ok()?;
        let zone = ZoneId::parse(&view.key.zone).ok()?;
        let uid = row_uid(&view.uid)?;
        let generation = ResourceGeneration::new(view.generation).ok()?;
        let metadata: Value = serde_json::from_slice(&view.metadata).ok()?;
        let spec: Value = serde_json::from_slice(&view.spec).ok()?;
        let mut status = self.observed_status(view);
        if resource_ref.resource_type().as_str() == "Process"
            && let Some(evidence) = self
                .sessions
                .controller_session_evidence(&resource_ref, &uid, generation)
        {
            status["resource"] = json!({ "controllerSession": evidence });
        }
        let canonical = json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": view.key.type_name,
            "metadata": metadata,
            "spec": spec,
            "status": status,
        });
        let canonical = serde_json::to_vec(&canonical).ok()?;
        Some(DependencySnapshot::new(
            ResourceSnapshot::new(
                ResourceKey::new(zone, resource_ref, uid),
                ZoneRevision::new(view.generation),
                generation,
                canonical,
                view.deleting,
            )
            .with_owner_identity(Some(provider.key().uid().clone()), Some(provider.generation())),
        ))
    }

    /// The status projection for one row. `observed_status()` is the only
    /// status the manager vouches for: a status published for an older row
    /// generation is not observed state of the current row and therefore is
    /// never reported as `Ready`.
    fn observed_status(&self, view: &ResourceView) -> Value {
        match view.observed_status() {
            Some(ResourceStatus::Ready) => json!({
                "phase": "Ready",
                "observedGeneration": view.generation,
            }),
            // A failed driver is the closed classification the old plane
            // reported as a degraded component; it is never `Ready`.
            Some(ResourceStatus::Failed(_)) => json!({ "phase": "Degraded" }),
            Some(_) | None => json!({ "phase": "Pending" }),
        }
    }

    #[allow(dead_code)]
    fn provider_ref_of(view: &ResourceView) -> Option<ResourceRef> {
        crate::resource_plane_v3::decode_metadata_owner_ref(&view.metadata)
    }
}

/// Map a manager row's 16-byte deterministic uid onto the contracts crate's
/// UUIDv4-shaped `ResourceUid` (same mapping the converted drivers use).
pub(crate) fn row_uid(bytes: &[u8; 16]) -> Option<ResourceUid> {
    let mut bytes = *bytes;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).ok()
}

// ---------------------------------------------------------------------------
// Registered-API decorator
// ---------------------------------------------------------------------------

/// Decorate one registered controller API with the manager dependency bridge.
///
/// Everything delegates to the wrapped API except the fresh read of a
/// `Provider` target, which merges the Provider's manager-served children
/// into the dependency list. Only the owner-scoped Core `Provider` handler
/// reads `Process`/`Volume` children off a `Provider` target, so no other
/// caller's dependency list changes.
pub(crate) struct PlaneAwareControllerApi<A> {
    inner: Arc<A>,
    merge: Option<Arc<PlaneProviderDependencies>>,
}

impl<A> PlaneAwareControllerApi<A> {
    pub(crate) fn new(inner: Arc<A>, merge: Option<Arc<PlaneProviderDependencies>>) -> Self {
        Self { inner, merge }
    }
}

#[async_trait]
impl<A: RegisteredControllerApi> RegisteredControllerApi for PlaneAwareControllerApi<A> {
    fn register(
        &self,
        descriptor: &ControllerDescriptor,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.inner.register(descriptor)
    }

    fn list_initial(
        &self,
        descriptor: &ControllerDescriptor,
    ) -> impl Future<Output = Result<InitialList, SourceError>> + Send {
        self.inner.list_initial(descriptor)
    }

    fn open_watch(
        &self,
        descriptor: &ControllerDescriptor,
        after_revision: ZoneRevision,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.inner.open_watch(descriptor, after_revision)
    }

    fn stop_watch(&self) {
        self.inner.stop_watch();
    }

    fn has_watch_stream(&self) -> bool {
        self.inner.has_watch_stream()
    }

    fn receive_watch_change(
        &self,
    ) -> impl Future<Output = Result<Option<(ChangeRecord, OperationContext)>, WatchFailure>> + Send
    {
        self.inner.receive_watch_change()
    }

    fn read_fresh(
        &self,
        key: &ResourceKey,
    ) -> impl Future<Output = Result<FreshSnapshot, SourceError>> + Send {
        let key = key.clone();
        let inner = Arc::clone(&self.inner);
        let merge = self.merge.clone();
        async move {
            let snapshot = inner.read_fresh(&key).await?;
            let FreshSnapshot::Present {
                target,
                mut dependencies,
            } = snapshot
            else {
                return Ok(snapshot);
            };
            let is_provider = key.resource_ref().resource_type().as_str() == "Provider";
            if is_provider
                && let Some(merge) = merge.as_deref()
            {
                merge.merge(key.resource_ref(), &target, &mut dependencies).await?;
            }
            Ok(FreshSnapshot::Present {
                target,
                dependencies,
            })
        }
    }

    fn write_starting(
        &self,
        context: &ReconcileContext,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.inner.write_starting(context)
    }

    fn accept_effect(
        &self,
        context: &ReconcileContext,
        plan: &ReconcilePlan,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.inner.accept_effect(context, plan)
    }

    fn accepted_effect_operation(
        &self,
        context: &ReconcileContext,
    ) -> impl Future<Output = Result<Option<OperationContext>, SourceError>> + Send {
        self.inner.accepted_effect_operation(context)
    }

    fn complete_effect(
        &self,
        context: &ReconcileContext,
        result: &ReconcileResult,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.inner.complete_effect(context, result)
    }

    fn verify_expedited_commit(
        &self,
        context: &ReconcileContext,
    ) -> impl Future<Output = Result<bool, SourceError>> + Send {
        self.inner.verify_expedited_commit(context)
    }

    fn commit_result(
        &self,
        context: &ReconcileContext,
        result: &ReconcileResult,
    ) -> impl Future<Output = Result<CommitOutcome, SourceError>> + Send {
        self.inner.commit_result(context, result)
    }

    fn complete_expedited(
        &self,
        context: &ReconcileContext,
        projection: &ReconcileProjection,
        status_persistence: StatusPersistence,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.inner
            .complete_expedited(context, projection, status_persistence)
    }

    fn persist_outcome(
        &self,
        projection: &ReconcileProjection,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.inner.persist_outcome(projection)
    }

    fn persist_outcome_with_operation(
        &self,
        projection: &ReconcileProjection,
        operation: &OperationContext,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.inner.persist_outcome_with_operation(projection, operation)
    }

    fn checkpoint(
        &self,
        context: &ReconcileContext,
        revision: ZoneRevision,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.inner.checkpoint(context, revision)
    }

    fn schedule_requeue(
        &self,
        key: &ResourceKey,
        at_tick: u64,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.inner.schedule_requeue(key, at_tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use async_trait::async_trait;
    use d2b_contracts_resource::v3::{ResourceTypeName, ZoneId};
    use d2b_core_controller::provider_observation;

    const PROVIDER_REF: &str = "Provider/runtime-cloud-hypervisor";
    const PROCESS_UID: &str = "11111111-1111-4111-8111-111111111111";
    const PROVIDER_UID: &str = "22222222-2222-4222-8222-222222222222";
    const SESSION_GENERATION: u64 = 7;

    struct FixedRows {
        rows: BTreeMap<String, Vec<ResourceView>>,
    }

    #[async_trait]
    impl PlaneDependencyRows for FixedRows {
        async fn rows(&self, resource_type: &str) -> Result<Vec<ResourceView>, ResourceError> {
            Ok(self
                .rows
                .get(resource_type)
                .cloned()
                .unwrap_or_default())
        }
    }

    #[derive(Default)]
    struct FixedSessions {
        evidence: Option<Value>,
    }

    impl LiveControllerSessionEvidence for FixedSessions {
        fn controller_session_evidence(
            &self,
            _process_ref: &ResourceRef,
            _process_uid: &ResourceUid,
            _generation: ResourceGeneration,
        ) -> Option<Value> {
            self.evidence.clone()
        }
    }

    fn row_uid_bytes(uid: &str) -> [u8; 16] {
        let uid = ResourceUid::parse(uid).unwrap();
        let hex = uid.as_str().replace('-', "");
        let mut bytes = [0u8; 16];
        for (index, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
        }
        bytes
    }

    fn provider_snapshot() -> ResourceSnapshot {
        let zone = ZoneId::parse("work").unwrap();
        let key = ResourceKey::new(
            zone,
            ResourceRef::parse(PROVIDER_REF).unwrap(),
            ResourceUid::parse(PROVIDER_UID).unwrap(),
        );
        ResourceSnapshot::new(
            key,
            ZoneRevision::new(9),
            ResourceGeneration::new(4).unwrap(),
            serde_json::to_vec(&json!({
                "spec": {"artifactId": "runtime-cloud-hypervisor", "config": {}},
                "status": {"observedGeneration": 4},
            }))
            .unwrap(),
            false,
        )
    }

    fn controller_process_view(status: Option<(ResourceStatus, u64)>) -> ResourceView {
        ResourceView {
            key: ManagerKey::new("work", "Process", "controller-runtime-cloud-hypervisor-ch"),
            uid: row_uid_bytes(PROCESS_UID),
            generation: 3,
            deleting: false,
            provenance: d2b_resource_runtime::spec_store::ResourceProvenance::Nix,
            spec: serde_json::to_vec(&json!({
                "providerRef": "Provider/system-minijail",
                "processClass": "controller",
                "template": "controller-runtime-cloud-hypervisor-cloud-hypervisor-controller",
                "executionRef": "Host/host-system",
            }))
            .unwrap(),
            metadata: serde_json::to_vec(&json!({
                "ownerRef": PROVIDER_REF,
                "labels": {},
                "annotations": {},
            }))
            .unwrap(),
            owner_key: None,
            status: status.map(|(status, _)| status),
            status_generation: status.map(|(_, generation)| generation),
        }
    }

    fn live_session_evidence() -> Value {
        json!({
            "ready": true,
            "providerRef": PROVIDER_REF,
            "providerUid": PROVIDER_UID,
            "providerGeneration": 4,
            "processRef": "Process/controller-runtime-cloud-hypervisor-ch",
            "processUid": PROCESS_UID,
            "processGeneration": 3,
            "controllerGeneration": 1,
            "sessionGeneration": SESSION_GENERATION,
            "artifactReady": true,
            "descriptorReady": true,
            "registrationReady": true,
        })
    }

    fn merge_engine(
        status: Option<(ResourceStatus, u64)>,
        evidence: Option<Value>,
    ) -> PlaneProviderDependencies {
        let rows = FixedRows {
            rows: BTreeMap::from([(
                "Process".to_owned(),
                vec![controller_process_view(status)],
            )]),
        };
        PlaneProviderDependencies::new(
            Arc::new(rows),
            Arc::new(FixedSessions { evidence }),
        )
    }

    #[tokio::test]
    async fn manager_controller_row_is_counted_and_reports_dependencies_ready() {
        let provider = provider_snapshot();
        let mut dependencies = Vec::new();
        merge_engine(
            Some((ResourceStatus::Ready, 3)),
            Some(live_session_evidence()),
        )
        .merge(
            &ResourceRef::parse(PROVIDER_REF).unwrap(),
            &provider,
            &mut dependencies,
        )
        .await
        .unwrap();

        assert_eq!(dependencies.len(), 1, "the manager row must be merged in");
        let dependency = &dependencies[0];
        assert_eq!(
            dependency.resource().key().resource_ref().to_canonical_string(),
            "Process/controller-runtime-cloud-hypervisor-ch"
        );
        assert_eq!(
            dependency.resource().owner_uid(),
            Some(&ResourceUid::parse(PROVIDER_UID).unwrap()),
            "the Provider owns the row: owner identity must be attached"
        );
        let value: Value = serde_json::from_slice(dependency.resource().canonical_json()).unwrap();
        assert_eq!(value.pointer("/status/phase").and_then(Value::as_str), Some("Ready"));
        assert_eq!(
            value.pointer("/status/observedGeneration").and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            value
                .pointer("/status/resource/controllerSession/ready")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value
                .pointer("/status/resource/controllerSession/sessionGeneration")
                .and_then(Value::as_u64),
            Some(SESSION_GENERATION)
        );

        let observation = provider_observation(&provider, &dependencies)
            .expect("provider observation parses the merged dependency list");
        assert!(
            !observation.components_drained,
            "the Provider must count its manager-served controller Process row"
        );
        assert!(
            observation.required_dependencies_ready,
            "an observed-Ready controller row makes the Provider's dependencies ready"
        );
        assert!(
            observation.required_components_ready,
            "the live admitted session makes the controller component ready"
        );
    }

    #[tokio::test]
    async fn stale_status_generation_is_never_read_as_ready() {
        // The actor published Ready for generation 2; the row is generation 3,
        // so the status is not observed state of the current row.
        let provider = provider_snapshot();
        let mut dependencies = Vec::new();
        merge_engine(
            Some((ResourceStatus::Ready, 2)),
            Some(live_session_evidence()),
        )
        .merge(
            &ResourceRef::parse(PROVIDER_REF).unwrap(),
            &provider,
            &mut dependencies,
        )
        .await
        .unwrap();

        let value: Value =
            serde_json::from_slice(dependencies[0].resource().canonical_json()).unwrap();
        assert_ne!(
            value.pointer("/status/phase").and_then(Value::as_str),
            Some("Ready"),
            "a status published for an older generation is not observed state"
        );
        let observation = provider_observation(&provider, &dependencies).unwrap();
        assert!(!observation.required_dependencies_ready);
    }

    #[tokio::test]
    async fn missing_live_session_keeps_the_component_not_ready() {
        let provider = provider_snapshot();
        let mut dependencies = Vec::new();
        merge_engine(Some((ResourceStatus::Ready, 3)), None)
            .merge(
                &ResourceRef::parse(PROVIDER_REF).unwrap(),
                &provider,
                &mut dependencies,
            )
            .await
            .unwrap();

        let value: Value =
            serde_json::from_slice(dependencies[0].resource().canonical_json()).unwrap();
        assert!(
            value.pointer("/status/resource/controllerSession").is_none(),
            "no live session means no synthesized evidence, never ready: true"
        );
        let observation = provider_observation(&provider, &dependencies).unwrap();
        assert!(observation.required_dependencies_ready);
        assert!(
            !observation.required_components_ready,
            "the session fence stays: process-ready is not session-ready"
        );
    }

    #[tokio::test]
    async fn rows_of_another_owner_are_not_merged() {
        let mut view = controller_process_view(Some((ResourceStatus::Ready, 3)));
        view.metadata = serde_json::to_vec(&json!({"ownerRef": "Provider/other"})).unwrap();
        let engine = PlaneProviderDependencies::new(
            Arc::new(FixedRows {
                rows: BTreeMap::from([("Process".to_owned(), vec![view])]),
            }),
            Arc::new(FixedSessions { evidence: None }),
        );
        let provider = provider_snapshot();
        let mut dependencies = Vec::new();
        engine
            .merge(
                &ResourceRef::parse(PROVIDER_REF).unwrap(),
                &provider,
                &mut dependencies,
            )
            .await
            .unwrap();
        assert!(dependencies.is_empty());
    }

    /// The decorator merges only for `Provider` targets; every other fresh
    /// read keeps the wrapped API's answer.
    #[tokio::test]
    async fn decorator_merges_only_provider_targets() {
        struct FakeApi {
            dependencies: Vec<DependencySnapshot>,
        }

        impl FakeApi {
            fn present(&self, key: &ResourceKey) -> FreshSnapshot {
                FreshSnapshot::Present {
                    target: ResourceSnapshot::new(
                        key.clone(),
                        ZoneRevision::new(1),
                        ResourceGeneration::new(1).unwrap(),
                        b"{}".to_vec(),
                        false,
                    ),
                    dependencies: self.dependencies.clone(),
                }
            }
        }

        #[async_trait]
        impl RegisteredControllerApi for FakeApi {
            fn register(
                &self,
                _descriptor: &ControllerDescriptor,
            ) -> impl Future<Output = Result<(), SourceError>> + Send {
                async { Ok(()) }
            }

            fn list_initial(
                &self,
                _descriptor: &ControllerDescriptor,
            ) -> impl Future<Output = Result<InitialList, SourceError>> + Send {
                async { unimplemented!() }
            }

            fn open_watch(
                &self,
                _descriptor: &ControllerDescriptor,
                _after_revision: ZoneRevision,
            ) -> impl Future<Output = Result<(), SourceError>> + Send {
                async { Ok(()) }
            }

            fn read_fresh(
                &self,
                key: &ResourceKey,
            ) -> impl Future<Output = Result<FreshSnapshot, SourceError>> + Send {
                let snapshot = self.present(key);
                async move { Ok(snapshot) }
            }

            fn write_starting(
                &self,
                _context: &ReconcileContext,
            ) -> impl Future<Output = Result<(), SourceError>> + Send {
                async { Ok(()) }
            }

            fn commit_result(
                &self,
                _context: &ReconcileContext,
                _result: &ReconcileResult,
            ) -> impl Future<Output = Result<CommitOutcome, SourceError>> + Send {
                async { Ok(CommitOutcome::Committed(ZoneRevision::new(1))) }
            }

            fn complete_expedited(
                &self,
                _context: &ReconcileContext,
                _projection: &ReconcileProjection,
                _status_persistence: StatusPersistence,
            ) -> impl Future<Output = Result<(), SourceError>> + Send {
                async { Ok(()) }
            }

            fn persist_outcome(
                &self,
                _projection: &ReconcileProjection,
            ) -> impl Future<Output = Result<(), SourceError>> + Send {
                async { Ok(()) }
            }

            fn checkpoint(
                &self,
                _context: &ReconcileContext,
                _revision: ZoneRevision,
            ) -> impl Future<Output = Result<(), SourceError>> + Send {
                async { Ok(()) }
            }

            fn schedule_requeue(
                &self,
                _key: &ResourceKey,
                _at_tick: u64,
            ) -> impl Future<Output = Result<(), SourceError>> + Send {
                async { Ok(()) }
            }
        }

        let engine: Arc<PlaneProviderDependencies> = Arc::new(merge_engine(
            Some((ResourceStatus::Ready, 3)),
            Some(live_session_evidence()),
        ));
        let api = Arc::new(PlaneAwareControllerApi::new(
            Arc::new(FakeApi {
                dependencies: Vec::new(),
            }),
            Some(engine),
        ));

        let provider_key = ResourceKey::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse(PROVIDER_REF).unwrap(),
            ResourceUid::parse(PROVIDER_UID).unwrap(),
        );
        let merged = api.read_fresh(&provider_key).await.unwrap();
        let FreshSnapshot::Present { dependencies, .. } = merged else {
            panic!("present snapshot");
        };
        assert_eq!(dependencies.len(), 1, "Provider targets merge the plane rows");

        let volume_key = ResourceKey::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse("Volume/state").unwrap(),
            ResourceUid::parse("33333333-3333-4333-8333-333333333333").unwrap(),
        );
        let untouched = api.read_fresh(&volume_key).await.unwrap();
        let FreshSnapshot::Present { dependencies, .. } = untouched else {
            panic!("present snapshot");
        };
        assert!(dependencies.is_empty(), "non-Provider targets are untouched");
    }

    #[test]
    fn provider_owned_types_cover_the_two_owner_scoped_dependency_types() {
        assert_eq!(
            PROVIDER_OWNED_CONVERTED_TYPES,
            ["Process", "Volume"],
            "the owner-scoped core Provider dependency set"
        );
        assert_eq!(
            ResourceTypeName::parse(PROVIDER_OWNED_CONVERTED_TYPES[0])
                .unwrap()
                .as_str(),
            "Process"
        );
    }
}
