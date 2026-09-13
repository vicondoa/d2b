//! Telemetry Service reconciliation on the v3 resource runtime (U12
//! conversion; KTD3, KTD4, KTD13).
//!
//! The Provider controller stays the authority for the Service's semantics:
//! the telemetry Provider declares the pair's shape and Core materializes it.
//! This module is the KTD3 conversion of the old
//! `TelemetryResourceReconciler`'s Service half (spec section 13 mapping):
//!
//! - `describe` -> [`TelemetryServiceDriverFactory`], registered for
//!   `telemetry.d2bus.org.TelemetryService` in the plane's provider directory.
//! - `validate_spec` -> [`ResourceDriver::validate`]: the stored spec envelope
//!   must decode.
//! - `observe` -> [`ResourceDriver::recover`]: a Service realizes nothing on a
//!   target, so it recovers as adopted.
//! - `plan` + `reconcile` + `execute_effect` -> [`ResourceDriver::reconcile`]:
//!   the declared ingest `Endpoint` rows are re-read, the provider status
//!   projection moves into the driver's in-memory slot
//!   ([`ResourceContext::set_status`], R11: zero persistent writes), and a
//!   route that is not materialized yet re-schedules the preserved resync.
//! - `prepare_finalize` + `execute_finalize` + `finalize` ->
//!   [`ResourceDriver::delete`]. The old
//!   `d2b.d2bus.org/binding-children` finalizer is gone by construction: the
//!   v3 manager already holds a parent row until its owned children retire,
//!   and a Service owns none.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_provider::v3::semantic_services::telemetry::TELEMETRY_SERVICE_RESOURCE_TYPE;
use d2b_contracts_resource::v3::{CanonicalJsonObject, ResourceRef, ResourceSpec};
use d2b_resource_runtime::context::{
    ResourceContext, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_types::{AllowedSources, DriverDescriptor, WellKnownType};

/// The qualified semantic telemetry Service type this factory serves.
pub const TELEMETRY_SERVICE_TYPE: &str = TELEMETRY_SERVICE_RESOURCE_TYPE;

/// Preserved resync period (old `ResyncPolicy::new(None, 5_000)`). The actor
/// owns scheduling now (R13), so the driver re-schedules itself while the
/// route is not materialized instead of polling from a runner.
pub const TELEMETRY_SERVICE_RESYNC: Duration = Duration::from_secs(5);

/// Provider phase spelling for a usable ingest route.
pub const PHASE_READY: &str = "Ready";

/// Provider phase spelling for a route that is not materialized yet.
pub const PHASE_PENDING: &str = "Pending";

/// Provider phase spelling for an ambiguous, revoked, or unavailable route.
pub const PHASE_DEGRADED: &str = "Degraded";

/// The readiness term of the preserved phase predicate.
///
/// CONTRACT FLAG: the term reads a dependency's observed status (an ingest
/// Endpoint's `status.phase`), which the KTD3 driver surface does not expose.
/// It evaluates fail-closed until the surface carries observed state, so
/// `Ready` is never claimed without evidence.
pub const DEPENDENCY_READINESS_PROVEN: bool = false;

// ---------------------------------------------------------------------------
// Driver error
// ---------------------------------------------------------------------------

/// Stable failures from the telemetry Service (old
/// `SemanticBindingRuntimeError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryServiceDriverErrorKind {
    /// The stored spec envelope did not decode.
    InvalidResource,
    /// A manager route (row read) failed.
    Reconcile,
}

impl TelemetryServiceDriverErrorKind {
    /// The stable lower-kebab code for this classification.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidResource => "semantic-binding-resource-invalid",
            Self::Reconcile => "semantic-binding-reconcile-failed",
        }
    }
}

/// Typed driver failure; redacted at the erased boundary through
/// [`ResourceDriver::classify_error`] (R13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelemetryServiceDriverError {
    kind: TelemetryServiceDriverErrorKind,
    op: DriverOp,
}

impl TelemetryServiceDriverError {
    const fn new(kind: TelemetryServiceDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }

    /// The closed failure classification.
    pub const fn kind(self) -> TelemetryServiceDriverErrorKind {
        self.kind
    }

    /// The verb that failed.
    pub const fn op(self) -> DriverOp {
        self.op
    }
}

impl core::fmt::Display for TelemetryServiceDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl std::error::Error for TelemetryServiceDriverError {}

// ---------------------------------------------------------------------------
// In-memory status (R11)
// ---------------------------------------------------------------------------

/// The provider projection the old reconciler persisted through the Resource
/// API, now in-memory only (R11: runtime status is never persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryServiceStatus {
    /// The projected provider phase spelling.
    pub phase: &'static str,
    /// `{serviceRole, serviceReadiness}`; empty when the spec is degraded.
    pub projection: serde_json::Value,
    /// Declared ingest endpoint refs whose rows exist and are not deleting.
    pub present_endpoints: Vec<ResourceRef>,
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one telemetry Service row (KTD2), exactly as
/// persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryServiceSpecEnvelope {
    base: CanonicalJsonObject,
}

impl TelemetryServiceSpecEnvelope {
    /// The type-specific base as a JSON value, as the old reconciler read it.
    fn value(&self) -> Result<serde_json::Value, TelemetryServiceDriverErrorKind> {
        serde_json::from_slice(&self.base.to_canonical_bytes())
            .map_err(|_| TelemetryServiceDriverErrorKind::InvalidResource)
    }
}

/// The manager-wired decode hook for telemetry Service rows.
pub fn telemetry_service_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| TelemetryServiceSpecEnvelope {
            base: spec.base().clone(),
        })
    })
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// [`ResourceDriverFactory`] for the telemetry Service type. Construction is
/// infallible by contract (R3).
#[derive(Debug)]
pub struct TelemetryServiceDriverFactory {
    types: [ResourceTypeName; 1],
}

impl TelemetryServiceDriverFactory {
    /// Construct the Service factory.
    pub fn new() -> Self {
        Self {
            types: [ResourceTypeName::new(TELEMETRY_SERVICE_TYPE)],
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for TelemetryServiceDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(TelemetryServiceDriver::new(key))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One telemetry Service row's driver.
pub struct TelemetryServiceDriver {
    key: ResourceKey,
    /// Targets this driver already registered an internal watch on (R12).
    /// Runtime-only (R13, R6). `WatchCondition::Ready` is one-shot and the
    /// driver cannot observe satisfaction, so one registration per target
    /// keeps the dependency edge (which outlives satisfaction and wakes the
    /// actor on dependency death) without leaking manager watch entries.
    watched: Vec<ResourceKey>,
}

impl TelemetryServiceDriver {
    fn new(key: &ResourceKey) -> Self {
        Self {
            key: key.clone(),
            watched: Vec::new(),
        }
    }

    const fn error(
        &self,
        kind: TelemetryServiceDriverErrorKind,
        op: DriverOp,
    ) -> TelemetryServiceDriverError {
        TelemetryServiceDriverError::new(kind, op)
    }

    /// The manager-routed identity of one referenced row (same zone as this
    /// row; `ResourceRef` carries no zone).
    fn row_key(&self, reference: &ResourceRef) -> ResourceKey {
        ResourceKey::new(
            self.key.zone.clone(),
            reference.resource_type().as_str(),
            reference.name().as_str(),
        )
    }

    fn envelope(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<TelemetryServiceSpecEnvelope, TelemetryServiceDriverError> {
        ctx.spec::<TelemetryServiceSpecEnvelope>()
            .cloned()
            .map_err(|_| self.error(TelemetryServiceDriverErrorKind::InvalidResource, op))
    }

    /// Register one internal dependency watch (R12) exactly once per target.
    ///
    /// Best-effort by design: a dependency that is still an unconverted row
    /// has no actor to watch yet (`register_watch` refuses it), and the
    /// requeue schedule is what re-evaluates those rows until they are served.
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    /// One reconcile pass of the Service projection (old `plan` +
    /// `reconcile` + `execute_effect` for the Service role).
    async fn reconcile_service(
        &mut self,
        ctx: &mut ResourceContext,
        envelope: &TelemetryServiceSpecEnvelope,
    ) -> Result<ReconcileOutcome, TelemetryServiceDriverError> {
        let op = DriverOp::Reconcile;
        let spec = envelope.value().map_err(|kind| self.error(kind, op))?;
        let role = spec
            .get("serviceRole")
            .and_then(serde_json::Value::as_str)
            .filter(|role| matches!(*role, "authority" | "projection"));
        let Some(role) = role else {
            // Old: a Service whose role is absent or unadmitted reports
            // Degraded with an empty projection and mutates nothing.
            ctx.set_status(TelemetryServiceStatus {
                phase: PHASE_DEGRADED,
                projection: serde_json::json!({}),
                present_endpoints: Vec::new(),
            });
            return Ok(ReconcileOutcome::Satisfied);
        };
        if role == "projection" {
            // Old: a projection Service is Ready without an ingest route.
            ctx.set_status(TelemetryServiceStatus {
                phase: PHASE_READY,
                projection: serde_json::json!({
                    "serviceRole": "projection",
                    "serviceReadiness": PHASE_READY,
                }),
                present_endpoints: Vec::new(),
            });
            return Ok(ReconcileOutcome::Satisfied);
        }
        let endpoint_refs = ingest_endpoint_refs(&spec);
        let mut present_endpoints = Vec::with_capacity(endpoint_refs.len());
        let mut all_present = !endpoint_refs.is_empty();
        for endpoint_ref in &endpoint_refs {
            let key = self.row_key(endpoint_ref);
            match ctx.get(&key).await {
                Ok(Some(row)) if !row.deleting => {
                    present_endpoints.push(endpoint_ref.clone());
                    self.watch_once(ctx, key).await;
                }
                Ok(_) => all_present = false,
                Err(_) => return Err(self.error(TelemetryServiceDriverErrorKind::Reconcile, op)),
            }
        }
        let ready = all_present && DEPENDENCY_READINESS_PROVEN;
        let phase = if ready { PHASE_READY } else { PHASE_PENDING };
        ctx.set_status(TelemetryServiceStatus {
            phase,
            projection: serde_json::json!({
                "serviceRole": "authority",
                "serviceReadiness": phase,
            }),
            present_endpoints,
        });
        if !all_present {
            // The route is not even materialized yet: re-evaluate on the old
            // resync cadence until the declared endpoint rows exist.
            ctx.requeue_after(TELEMETRY_SERVICE_RESYNC);
        }
        Ok(ReconcileOutcome::Satisfied)
    }
}

#[async_trait::async_trait]
impl ResourceDriver for TelemetryServiceDriver {
    type Error = TelemetryServiceDriverError;

    fn classify_error(&self, error: &TelemetryServiceDriverError) -> DriverFailure {
        // The old reconciler classified every failure retryable; the actor
        // owns retry/backoff from the closed class (R13).
        DriverFailure::retryable(error.op)
    }

    /// Structural validation only (old `validate_spec`): the stored envelope
    /// must decode.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let _ = self.envelope(ctx, DriverOp::Validate)?;
        Ok(())
    }

    /// Discovery and adoption on the realization target (F2, R15-R16): the
    /// Service realizes nothing on a target (its observed state is the
    /// ingest-endpoint rows reconcile re-reads), so it adopts.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let _ = self.envelope(ctx, DriverOp::Recover)?;
        Ok(RecoveryOutcome::Adopted)
    }

    /// One reconcile pass (old `plan` + `reconcile` + `execute_effect`).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let envelope = self.envelope(ctx, op)?;
        self.reconcile_service(ctx, &envelope).await
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown. A Service owns no child, so the call
    /// converges immediately; the erased boundary runs the same step.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| {
                self.error(
                    TelemetryServiceDriverErrorKind::Reconcile,
                    DriverOp::Delete,
                )
            })?;
        Ok(())
    }

    /// Teardown (old `prepare_finalize` + `execute_finalize` + `finalize`).
    ///
    /// The durable deleting mark is already committed and the manager has
    /// already cascaded the owned children before this pass runs. The Service
    /// realizes nothing else on a target, so there is no further effect to run
    /// here.
    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Spec projection helpers (old `telemetry_endpoint_refs`)
// ---------------------------------------------------------------------------

/// Ingest endpoint refs declared by a Service spec.
fn ingest_endpoint_refs(spec: &serde_json::Value) -> Vec<ResourceRef> {
    spec.get("ingestEndpointRefs")
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| {
                    value
                        .as_str()
                        .and_then(|value| ResourceRef::parse(value).ok())
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The resource verbs the TelemetryService type supports.
///
/// Derived from the v3 resource plane's converted-type verb surface: the
/// closed `RoleResourceVerb` set minus the two Credential-scoped credential
/// verbs (`use-credential`, `admin-credential`), which the plane gates to the
/// `Credential` type. Every converted type is served by the same manager
/// verbs, and Role rules and the typed CLI nouns resolve their gating from
/// this declaration.
const TELEMETRY_SERVICE_VERBS: &[&str] = &[
    "get",
    "list",
    "watch",
    "create",
    "update-spec",
    "update-status",
    "update-metadata",
    "update-finalizers",
    "delete",
];

/// The execution domains the TelemetryService type can be reconciled in.
///
/// Derived from the placement contract: `TelemetryService` names no placement
/// anchor (`PlacementAnchor::canonical_for` resolves none), so a Service row
/// never carries the canonical `spec.executionRef` and the plane reconciles it
/// on its own Host domain.
const TELEMETRY_SERVICE_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the Service realization reads while reconciling.
///
/// Derived from the driver's row reads: the declared ingest routes are
/// `Endpoint` rows.
const TELEMETRY_SERVICE_READS: &[WellKnownType] = &[WellKnownType::ENDPOINT];

/// The TelemetryService type's driver declaration.
///
/// `TelemetryService` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane
/// cannot serve the Zone's telemetry authority without it, so it must be
/// registered before the plane opens. The type is exportable: the qualified
/// `telemetry.d2bus.org.TelemetryService` is exactly the shape
/// `ResourceExport` admits. The driver serves no broker operations and owns
/// no child, so it declares no creation.
pub fn telemetry_service_descriptor() -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::TELEMETRY_SERVICE,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: TELEMETRY_SERVICE_VERBS,
        execution: TELEMETRY_SERVICE_EXECUTION_DOMAINS,
        exportable: true,
        reads: TELEMETRY_SERVICE_READS,
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: telemetry_service_spec_decoder(),
        factory: Arc::new(TelemetryServiceDriverFactory::new()),
    }
}

// ---------------------------------------------------------------------------
// Tests: driver-level behavior over a recording manager endpoint (the same
// shape U7 used) and a requeue recorder. Effects are observed as the manager
// records them: the row reads and the watch registration.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, WatchId,
        WatchRegistration,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::ResourceProvenance;
    use d2b_resource_runtime::spec_store::{EnsureOutcome, StoredDesiredResource};
    use d2b_resource_runtime::target::TargetHandle;
    use tokio::sync::mpsc;

    use super::*;

    // -- fakes ---------------------------------------------------------------

    /// Manager endpoint double: an owned-row store plus the ordered call log
    /// the assertions read.
    struct RecordingManager {
        parent_uid: [u8; 16],
        rows: Mutex<Vec<StoredDesiredResource>>,
        log: Mutex<Vec<String>>,
        watch_targets: Mutex<Vec<ResourceKey>>,
    }

    impl RecordingManager {
        fn new(parent_uid: [u8; 16]) -> Arc<Self> {
            Arc::new(Self {
                parent_uid,
                rows: Mutex::new(Vec::new()),
                log: Mutex::new(Vec::new()),
                watch_targets: Mutex::new(Vec::new()),
            })
        }

        fn seed(&self, row: StoredDesiredResource) {
            self.rows.lock().expect("rows").push(row);
        }

        fn log(&self) -> Vec<String> {
            self.log.lock().expect("log").clone()
        }

        fn watch_targets(&self) -> Vec<ResourceKey> {
            self.watch_targets.lock().expect("watch targets").clone()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            let mut rows = self.rows.lock().expect("rows");
            let row = StoredDesiredResource {
                key: ResourceKey::new("dev", child.type_name.as_str(), child.name.clone()),
                uid: [0x11; 16],
                generation: 1,
                owner_uid: Some(self.parent_uid),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: child.spec.clone(),
                metadata: child.metadata.clone(),
                created_at: 0,
            };
            rows.push(row.clone());
            self.log.lock().expect("log").push(format!(
                "ensure:{}/{}",
                child.type_name.as_str(),
                child.name
            ));
            Ok(EnsureOutcome::Created(row))
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self
                .rows
                .lock()
                .expect("rows")
                .iter()
                .find(|row| row.key == *key)
                .cloned())
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            // Desired rows only: this fixture publishes no runtime status, so
            // it serves no observed state.
            Ok(None)
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.log
                .lock()
                .expect("log")
                .push(format!("delete:{}/{}", key.type_name, key.name));
            if let Some(row) = self
                .rows
                .lock()
                .expect("rows")
                .iter_mut()
                .find(|row| row.key == *key)
            {
                row.deleting = true;
            }
            Ok(())
        }

        async fn list_owned(
            &self,
            owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self
                .rows
                .lock()
                .expect("rows")
                .iter()
                .filter(|row| row.owner_uid == Some(owner_uid))
                .cloned()
                .collect())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            let mut targets = self.watch_targets.lock().expect("watch targets");
            targets.push(registration.target.clone());
            Ok(WatchId(targets.len() as u64))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    /// Requeue recorder (R13): the driver's schedule calls, in order.
    #[derive(Default)]
    struct RecordingRequeue {
        scheduled: Mutex<Vec<Duration>>,
    }

    impl RecordingRequeue {
        fn scheduled(&self) -> Vec<Duration> {
            self.scheduled.lock().expect("scheduled").clone()
        }
    }

    impl RequeueScheduler for RecordingRequeue {
        fn schedule(&self, _key: ResourceKey, after: Duration) -> RequeueId {
            let mut scheduled = self.scheduled.lock().expect("scheduled");
            scheduled.push(after);
            RequeueId(scheduled.len() as u64)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    struct Fixture {
        ctx: ResourceContext,
        manager: Arc<RecordingManager>,
        requeue: Arc<RecordingRequeue>,
    }

    fn fixture(row: StoredDesiredResource) -> Fixture {
        let manager = RecordingManager::new(row.uid);
        let requeue = Arc::new(RecordingRequeue::default());
        let (effects_tx, _effects_rx) = mpsc::unbounded_channel();
        let (watch_tx, _watch_rx) = mpsc::unbounded_channel();
        let ctx = ResourceContext::new(
            row,
            TargetHandle::Host,
            telemetry_service_spec_decoder(),
            Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            Arc::clone(&requeue) as Arc<dyn RequeueScheduler>,
            effects_tx,
            watch_tx,
        );
        Fixture {
            ctx,
            manager,
            requeue,
        }
    }

    async fn driver(fixture: &Fixture) -> Box<dyn DynResourceDriver> {
        TelemetryServiceDriverFactory::new()
            .create(fixture.ctx.key())
            .await
    }

    // -- test data -----------------------------------------------------------

    fn row(zone: &str, reference: &str, spec: serde_json::Value) -> StoredDesiredResource {
        let reference = ResourceRef::parse(reference).expect("resource ref");
        let spec = serde_json::to_vec(&spec).expect("spec bytes");
        StoredDesiredResource {
            key: ResourceKey::new(
                zone,
                reference.resource_type().as_str(),
                reference.name().as_str(),
            ),
            uid: [0x42; 16],
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting: false,
            spec,
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    const TELEMETRY_PROVIDER_REF: &str = "Provider/observability-otel";

    fn service_row() -> StoredDesiredResource {
        row(
            "dev",
            "telemetry.d2bus.org.TelemetryService/ingest",
            serde_json::json!({
                "providerRef": TELEMETRY_PROVIDER_REF,
                "serviceRole": "authority",
                "ingestEndpointRefs": ["Endpoint/ingest"],
                "signals": ["metrics"],
                "quota": {},
                "policy": {},
            }),
        )
    }

    fn endpoint_row() -> StoredDesiredResource {
        row(
            "dev",
            "Endpoint/ingest",
            serde_json::json!({
                "providerRef": TELEMETRY_PROVIDER_REF,
                "producerRef": "Process/collector",
                "endpointClass": "service",
            }),
        )
    }

    // -- factory -------------------------------------------------------------

    #[tokio::test]
    async fn factory_registers_only_the_service_type() {
        let factory = TelemetryServiceDriverFactory::new();
        let types = factory
            .resource_types()
            .iter()
            .map(ResourceTypeName::as_str)
            .collect::<Vec<_>>();
        assert_eq!(types, vec![TELEMETRY_SERVICE_TYPE]);
    }

    // -- validate ------------------------------------------------------------

    #[tokio::test]
    async fn validate_rejects_a_malformed_spec() {
        let mut row = service_row();
        row.spec = b"{not-json".to_vec();
        let mut fixture = fixture(row);
        let mut driver = driver(&fixture).await;
        let failure = driver
            .validate(&mut fixture.ctx)
            .await
            .expect_err("malformed spec");
        assert_eq!(failure.op(), DriverOp::Validate);
        assert_eq!(failure.class(), FailureClass::Retryable);
    }

    #[tokio::test]
    async fn validate_accepts_a_provider_declared_spec() {
        let mut fixture = fixture(service_row());
        let mut driver = driver(&fixture).await;
        driver.validate(&mut fixture.ctx).await.expect("valid spec");
    }

    // -- recover -------------------------------------------------------------

    #[tokio::test]
    async fn recover_adopts_a_service_without_a_target_realization() {
        let mut fixture = fixture(service_row());
        let mut driver = driver(&fixture).await;
        assert_eq!(
            driver.recover(&mut fixture.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted,
            "a Service realizes nothing on a target: the ingest rows carry the evidence"
        );
        assert!(fixture.manager.log().is_empty());
    }

    // -- reconcile -----------------------------------------------------------

    #[tokio::test]
    async fn service_pending_until_declared_endpoints_exist_then_fail_closed() {
        let mut fixture = fixture(service_row());
        let mut driver = driver(&fixture).await;

        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let Some(status) = fixture.ctx.status::<TelemetryServiceStatus>() else {
            panic!("service status");
        };
        assert_eq!(status.phase, PHASE_PENDING);
        assert!(status.present_endpoints.is_empty());
        assert_eq!(
            fixture.requeue.scheduled(),
            vec![TELEMETRY_SERVICE_RESYNC],
            "the route is not materialized yet"
        );

        fixture.manager.seed(endpoint_row());
        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let Some(status) = fixture.ctx.status::<TelemetryServiceStatus>() else {
            panic!("service status");
        };
        assert_eq!(status.present_endpoints.len(), 1);
        assert_eq!(status.projection["serviceRole"], "authority");
        // CONTRACT FLAG: the old predicate also required the ingest
        // Endpoint's own `status.phase == "Ready"`, which this surface cannot
        // read; the phase stays fail-closed Pending while the row exists.
        assert_eq!(status.phase, PHASE_PENDING);
        assert_eq!(status.projection["serviceReadiness"], PHASE_PENDING);
        assert_eq!(
            fixture.requeue.scheduled(),
            vec![TELEMETRY_SERVICE_RESYNC],
            "a present endpoint stops rescheduling; readiness is watch-driven"
        );
    }

    #[tokio::test]
    async fn service_projection_role_reports_ready_without_ingest_evidence() {
        let mut fixture = fixture(row(
            "dev",
            "telemetry.d2bus.org.TelemetryService/aggregate",
            serde_json::json!({
                "providerRef": TELEMETRY_PROVIDER_REF,
                "serviceRole": "projection",
                "ingestEndpointRefs": [],
                "signals": ["metrics"],
                "quota": {},
                "policy": {},
            }),
        ));
        let mut driver = driver(&fixture).await;

        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let Some(status) = fixture.ctx.status::<TelemetryServiceStatus>() else {
            panic!("service status");
        };
        assert_eq!(status.phase, PHASE_READY);
        assert_eq!(status.projection["serviceRole"], "projection");
        assert_eq!(status.projection["serviceReadiness"], PHASE_READY);
        assert!(fixture.manager.log().is_empty());
        assert!(fixture.requeue.scheduled().is_empty());
    }

    #[tokio::test]
    async fn service_reconcile_reports_degraded_for_an_unadmitted_role() {
        let mut fixture = fixture(row(
            "dev",
            "telemetry.d2bus.org.TelemetryService/odd",
            serde_json::json!({
                "providerRef": TELEMETRY_PROVIDER_REF,
                "serviceRole": "mirror",
                "ingestEndpointRefs": [],
            }),
        ));
        let mut driver = driver(&fixture).await;

        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let Some(status) = fixture.ctx.status::<TelemetryServiceStatus>() else {
            panic!("service status");
        };
        assert_eq!(status.phase, PHASE_DEGRADED);
        assert_eq!(status.projection, serde_json::json!({}));
    }

    #[tokio::test]
    async fn dependency_watches_are_registered_once_per_target() {
        let mut fixture = fixture(service_row());
        let mut driver = driver(&fixture).await;

        fixture.manager.seed(endpoint_row());
        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let mut targets = fixture
            .manager
            .watch_targets()
            .into_iter()
            .map(|key| format!("{}/{}", key.type_name, key.name))
            .collect::<Vec<_>>();
        let registered = targets.len();
        targets.sort();
        targets.dedup();
        assert_eq!(registered, targets.len(), "no watch is registered twice");
        assert!(targets.contains(&"Endpoint/ingest".to_owned()), "{targets:?}");
    }

    // -- delete --------------------------------------------------------------

    #[tokio::test]
    async fn delete_runs_no_effect_past_the_manager_cascade() {
        let mut fixture = fixture(service_row());
        let mut driver = driver(&fixture).await;

        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let before = fixture.manager.log().len();
        driver.delete(&mut fixture.ctx).await.expect("delete");
        assert_eq!(
            fixture.manager.log().len(),
            before,
            "a Service owns no child and realizes nothing to remove"
        );
    }
}
