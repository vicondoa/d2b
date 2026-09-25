//! The Host resource driver: the v3 `ResourceDriver` conversion of the `Host`
//! handler the shared Core Runner used to execute.
//!
//! `Host` is one of the two bootstrap ResourceTypes `system-core` observes on
//! the local machine: the posture decision plus the bounded
//! capability/platform/proc observations. The type realizes nothing on a
//! target and owns no child rows, so the driver is an observation surface:
//! `recover` adopts without effects, `reconcile` publishes the typed in-memory
//! status (R11), and `delete` converges without effects (the manager already
//! cascaded the row's - empty - owned-child set).
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`host_descriptor`] registration under `Host`.
//! - `validate_spec` -> [`ResourceDriver::validate`]: typed spec decode plus
//!   the `Host.spec.providerRef` fence (`Provider/system-core` is the only
//!   Provider the Host contract admits).
//! - `plan` -> the preserved `observedGeneration` short-circuit: a status
//!   observed at the current generation skips re-observing (the old
//!   `ResourceReconciler::plan` returned a converged plan in that case).
//! - `observe` -> [`ResourceDriver::recover`] (old `ObservationResult` was
//!   converged: nothing to adopt).
//! - `execute_effect` (old `status_candidate`) -> [`ResourceDriver::reconcile`]
//!   over the [`HostDriverEffects`] probe port.
//! - `finalize` -> [`ResourceDriver::delete`] (old `FinalizeResult` was
//!   converged: the family owns no children and carries no finalizer).
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11).
//!
//! Deliberately not carried from the old handler: the durable
//! `status.resource` JSON projection and its `lastReconciledAt` /
//! `observedGeneration` writes (status is runtime-only now, R11), and the
//! `assess_update` / `plan_upgrade` runner path (no driver equivalent; the
//! family never planned an upgrade). The old runner's 5s resync relisted and
//! did nothing whenever the status was **current**, so it is not reproduced as
//! a blanket poll - but "current" is not "ready": an observation that is
//! `Degraded` (the user manager not up yet, a mandatory gate not passed) is the
//! state that resolves on its own, and pinning it for the whole generation
//! would leave the row claiming a realization the host never reached. A
//! non-Ready observation therefore re-probes on [`HOST_REOBSERVE`] and answers
//! [`ReconcileOutcome::RetryScheduled`], never `Satisfied`.
//!
//! U5: the driver's effects are this crate's own implementation
//! ([`crate::effects_service::HostEffectsService`]) built from the
//! daemon-supplied facet set - the construction site holds no externally
//! built port (R2) - and the family's declared effects service
//! (`host.d2bus.org/effects`) rides the declaration, so a zone that cannot
//! host it refuses startup by name (R5).
//!
//! KTD13: the driver has no spawn surface at all. It observes the local host
//! through the family's own probe and owns no Process.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ResourcePhase, ResourceRef, ResourceSpec,
    host::{HOST_PROVIDER_REF, HOST_RESOURCE_TYPE, HostSpec},
};
use d2b_provider_system_core::HostObservationReport;
use d2b_resource_runtime::context::{ResourceContext, SpecDecoder, typed_spec_decoder};
use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};

use crate::effects_service::{HOST_EFFECTS_SERVICE, HostEffectsService};
use crate::facets::HostEffectFacets;
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

/// How soon a Host observation that is not `Ready` is re-probed.
///
/// Echoes the old runner's 5s resync, which is what made a degraded Host
/// recover: an observation is a local probe, and the states it reports as not
/// ready resolve on their own. The re-probe is one probe per interval, not a
/// poll on the ready path, which the generation short-circuit still pins.
pub const HOST_REOBSERVE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostDriverErrorKind {
    /// The durable spec did not decode as the closed Host contract, or names
    /// a Provider the Host contract does not admit.
    SpecInvalid,
    /// The bounded host probe failed transiently.
    HostObservation,
    /// Owned children are still retiring; the delete pass requeues.
    DrainPending,
}

impl HostDriverErrorKind {
    /// The registered failure kind this classification reports.
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::SYSTEM_CORE_SPEC_INVALID,
            Self::HostObservation => FailureKinds::SYSTEM_CORE_HOST_OBSERVATION_FAILED,
            Self::DrainPending => FailureKinds::SYSTEM_CORE_DRAIN_PENDING,
        }
    }
}

/// Typed driver failure; mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`].
#[derive(Debug, Clone)]
pub struct HostDriverError {
    kind: HostDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl HostDriverError {
    const fn new(kind: HostDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op, detail: FailureDetail::new() }
    }

    /// Attach the operator-visible detail of this failure.
    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for HostDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            HostDriverErrorKind::SpecInvalid => "system-core-spec-invalid",
            HostDriverErrorKind::HostObservation => "system-core-host-observation-failed",
            HostDriverErrorKind::DrainPending => "system-core-drain-pending",
        })
    }
}

impl std::error::Error for HostDriverError {}

/// Typed in-memory status projection (R11: never persisted).
///
/// The observation is kept with the generation it was taken at, which is the
/// runtime-only successor of the old durable `status.observedGeneration`
/// plan short-circuit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostDriverStatus {
    observed_generation: u64,
    report: HostObservationReport,
}

impl HostDriverStatus {
    /// The desired generation this observation was taken at.
    pub const fn observed_generation(&self) -> u64 {
        self.observed_generation
    }

    /// The typed Host observation this status publishes.
    pub const fn report(&self) -> &HostObservationReport {
        &self.report
    }
}

// ---------------------------------------------------------------------------
// Spec decode hook
// ---------------------------------------------------------------------------

/// The manager-wired decode hook for Host rows.
///
/// The universal spec layer carries `providerRef` / `updatePolicy`; the base
/// keeps exactly the typed Host contract fields, so the decoder hands the
/// driver the complete desired state (`ResourceSpec::base()` is what the old
/// handler decoded into `HostSpec`).
pub fn host_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| serde_json::from_slice::<ResourceSpec>(bytes))
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The host-observation surface the Host driver needs: the preserved
/// `system-core` Provider behavior (bounded capability/platform/metadata
/// probe with its degraded fallback), behind the erased seam the driver
/// tests script. The production implementation is this crate's own
/// [`crate::effects_service::HostEffectsService`] (U5), built from the
/// daemon-supplied facet set.
#[async_trait]
pub trait HostDriverEffects: Send + Sync + 'static {
    /// Observe one Host and compute its public status, or report why the
    /// observation could not be taken.
    async fn observe_host(
        &self,
        host_ref: &ResourceRef,
        provider_ref: &ResourceRef,
        spec: &HostSpec,
    ) -> Result<HostObservationReport, String>;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// [`ResourceDriverFactory`] for the `Host` resource type. Construction is
/// infallible by contract: the effects carry no fallible setup.
pub struct HostDriverFactory {
    types: [ResourceTypeName; 1],
    effects: Arc<dyn HostDriverEffects>,
}

impl HostDriverFactory {
    /// Build the factory over the family's own effects implementation
    /// (U5), constructed from the daemon-supplied facet set: the
    /// construction site holds no externally built port (R2).
    pub fn new(facets: HostEffectFacets) -> Self {
        Self {
            types: [ResourceTypeName::new(HOST_RESOURCE_TYPE)],
            effects: Arc::new(HostEffectsService::new(facets)),
        }
    }
}

#[async_trait]
impl ResourceDriverFactory for HostDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(HostDriver::new(Arc::clone(&self.effects)))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One Host resource's driver.
pub struct HostDriver {
    effects: Arc<dyn HostDriverEffects>,
}

impl HostDriver {
    /// Build one resource's driver over the host-observation port.
    ///
    /// Crate-private: the only construction site is the factory, which
    /// builds the effects from the daemon-supplied facet set (U5); no
    /// externally built port appears anywhere (R2).
    pub(crate) fn new(effects: Arc<dyn HostDriverEffects>) -> Self {
        Self { effects }
    }

    fn error(&self, kind: HostDriverErrorKind, op: DriverOp) -> HostDriverError {
        HostDriverError::new(kind, op)
    }

    fn resource_ref(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<ResourceRef, HostDriverError> {
        let type_name = d2b_contracts_resource::v3::ResourceTypeName::parse(
            ctx.key().type_name.clone(),
        )
        .map_err(|_| self.error(HostDriverErrorKind::SpecInvalid, op))?;
        let name = d2b_contracts_resource::v3::ResourceName::parse(ctx.key().name.clone())
            .map_err(|_| self.error(HostDriverErrorKind::SpecInvalid, op))?;
        Ok(ResourceRef::new(type_name, name))
    }

    fn decoded_spec<'a>(
        &self,
        ctx: &'a ResourceContext,
        op: DriverOp,
    ) -> Result<&'a ResourceSpec, HostDriverError> {
        ctx.spec::<ResourceSpec>()
            .map_err(|_| self.error(HostDriverErrorKind::SpecInvalid, op))
    }

    /// The declared `spec.providerRef` fence plus the typed Host base spec.
    fn host_spec(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<(ResourceRef, HostSpec), HostDriverError> {
        let envelope = self.decoded_spec(ctx, op)?;
        let provider_ref = envelope
            .provider_ref()
            .cloned()
            .filter(|provider| provider.to_canonical_string() == HOST_PROVIDER_REF)
            .ok_or_else(|| self.error(HostDriverErrorKind::SpecInvalid, op))?;
        let spec = serde_json::from_slice::<HostSpec>(&envelope.base().to_canonical_bytes())
            .map_err(|_| self.error(HostDriverErrorKind::SpecInvalid, op))?;
        Ok((provider_ref, spec))
    }

    /// The generation this actor's in-memory status was observed at, if the
    /// status belongs to this driver and the same generation.
    fn observed_generation(&self, ctx: &ResourceContext) -> Option<u64> {
        ctx.status::<HostDriverStatus>()
            .map(HostDriverStatus::observed_generation)
    }

    /// Whether the stored observation is the one a converged Host publishes.
    ///
    /// The generation short-circuit answers "one observation per desired
    /// generation", and a degraded observation is stored at the same
    /// generation - so without this the short-circuit would pin a Host that is
    /// not realized for as long as its spec stands still.
    fn observed_ready(&self, ctx: &ResourceContext) -> bool {
        ctx.status::<HostDriverStatus>()
            .is_some_and(|status| status.report().status.phase == ResourcePhase::Ready)
    }
}

#[async_trait]
impl ResourceDriver for HostDriver {
    type Error = HostDriverError;

    fn classify_error(&self, error: &HostDriverError) -> DriverFailure {
        let failure = match error.kind {
            HostDriverErrorKind::SpecInvalid => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            HostDriverErrorKind::DrainPending => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            HostDriverErrorKind::HostObservation => DriverFailure::error(
                error.op,
                error.kind.failure_kind(),
                FailureClass::Retryable,
            ),
        };
        failure.with_detail(error.detail.clone())
    }

    /// Typed spec decode plus the Host Provider fence (old `validate_spec`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        self.host_spec(ctx, DriverOp::Validate)?;
        Ok(())
    }

    /// The old `observe` was converged with nothing to adopt: the family
    /// realizes no target-local state and owns no child rows.
    async fn recover(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<RecoveryOutcome, Self::Error> {
        self.host_spec(ctx, DriverOp::Recover)?;
        Ok(RecoveryOutcome::Adopted)
    }

    /// Observe the local host through the effects port and publish the typed
    /// status (R11). The preserved `observedGeneration` short-circuit keeps
    /// one observation per desired generation; a degraded observation still
    /// converges (the old handler persisted the degraded projection and the
    /// runner classified it converged).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let generation = ctx.generation();
        // One observation per desired generation - while that observation is
        // the Ready one. A generation pinned on a degraded observation must be
        // re-probed, or the Host claims a realization the probe never reached.
        if self.observed_generation(ctx) == Some(generation) && self.observed_ready(ctx) {
            return Ok(ReconcileOutcome::Satisfied);
        }
        let (provider_ref, spec) = self.host_spec(ctx, DriverOp::Reconcile)?;
        let host_ref = self.resource_ref(ctx, DriverOp::Reconcile)?;
        let report = self
            .effects
            .observe_host(&host_ref, &provider_ref, &spec)
            .await
            .map_err(|error| {
                self.error(HostDriverErrorKind::HostObservation, DriverOp::Reconcile)
                    .with_detail(
                        FailureDetail::at("host/probe")
                            .comparison(FailureComparison::new(
                                "host.probe",
                                "completed",
                                "failed",
                            ))
                            .with_note(error),
                    )
            })?;
        let ready = report.status.phase == ResourcePhase::Ready;
        ctx.set_status(HostDriverStatus {
            observed_generation: generation,
            report,
        });
        if ready {
            return Ok(ReconcileOutcome::Satisfied);
        }
        // A degraded observation is not a converged Host. It is also the state
        // that resolves on its own, so the row re-probes rather than pinning
        // the degraded projection for the rest of its generation.
        ctx.requeue_after(HOST_REOBSERVE);
        Ok(ReconcileOutcome::RetryScheduled)
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown. The family owns no child in production, so
    /// this converges immediately; an owned row still live requeues the pass.
    /// Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| self.error(HostDriverErrorKind::DrainPending, DriverOp::Delete))?;
        Ok(())
    }

    /// The old `finalize` was converged: the family owns no children and
    /// carries no finalizer, so teardown is the manager's row removal (R10).
    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The resource verbs the Host type supports.
///
/// Derived from the v3 resource plane's converted-type verb surface: the
/// closed `RoleResourceVerb` set minus the two Credential-scoped credential
/// verbs (`use-credential`, `admin-credential`), which the plane gates to the
/// `Credential` type. Every converted type is served by the same manager
/// verbs, and Role rules and the typed CLI nouns resolve their gating from
/// this declaration.
/// The execution domains the Host type can be reconciled in.
///
/// Derived from the placement contract: `Host` names no placement anchor
/// (`PlacementAnchor::canonical_for` resolves none), so a Host row never
/// carries the canonical `spec.executionRef` and the plane reconciles it on
/// its own Host domain - which is the row's own subject.
const HOST_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The Host type's driver declaration.
///
/// `Host` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane cannot serve the
/// converted bootstrap rows without it, so it must be registered before the
/// plane opens. The type is not exportable: `ResourceExport` admits only
/// qualified `*.d2bus.org.*Service` types. The driver serves no broker
/// operations, creates no children, and reads no other resource: the
/// observation reaches the local machine through the family's own probe.
///
/// U5: the driver's effects are this crate's own implementation
/// ([`crate::effects_service::HostEffectsService`]) built from the
/// daemon-supplied facet set - the construction site holds no externally
/// built port (R2) - and the family's declared effects service
/// ([`HOST_EFFECTS_SERVICE`]) rides the declaration, so a zone that cannot
/// host it refuses startup by name (R5).
pub fn host_descriptor(facets: HostEffectFacets) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::HOST,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: HOST_EXECUTION_DOMAINS,
        exportable: false,
        reads: &[],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[HOST_EFFECTS_SERVICE],
        decoder: host_spec_decoder(),
        factory: Arc::new(HostDriverFactory::new(facets)),
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a recording effects/manager double (the
// no-spawn and one-observation-per-generation invariants are asserted through
// the recorded calls).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use d2b_contracts_resource::v3::{
        ResourcePhase, ResourceRef, ResourceSpec,
        execution_policy::to_base_object,
        host::{HOST_PROVIDER_REF, HostSpec},
    };
    use d2b_provider_system_core::HostCapabilityClass;
    use d2b_provider_toolkit::testing::fakes::RecordingRequeue;
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, ResourceContext, WatchId, WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::provider::ProviderDirectory;
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;

    use crate::test_support::{RecordingEffects, RecordingProbe, scripted_facets};

    use super::{
        HostDriver, HostDriverStatus, host_descriptor, host_spec_decoder,
    };

    // -- fakes ---------------------------------------------------------------

/// Recording manager:the family must never mutate children or registers;
    /// any unexpected manager call fails the test loudly through the recorded
    /// call list.
    struct RecordingManager {
        calls: tokio::sync::Mutex<Vec<&'static str>>,
        owned: tokio::sync::Mutex<Vec<StoredDesiredResource>>,
    }

    impl RecordingManager {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: tokio::sync::Mutex::new(Vec::new()),
                owned: tokio::sync::Mutex::new(Vec::new()),
            })
        }

        /// Seed one owned child row (the finalize gate's input).
        fn seed_owned(&self, key: ResourceKey) {
            self.owned.try_lock().expect("uncontended test mutex").push(StoredDesiredResource {
                key,
                uid: [0x77; 16],
                generation: 1,
                owner_uid: Some([0x42; 16]),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: Vec::new(),
                metadata: Vec::new(),
                created_at: 0,
            });
        }

        fn call_order(&self) -> Vec<&'static str> {
            self.calls.try_lock().expect("uncontended test mutex").clone()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            self.calls.lock().await.push("ensure-child");
            Err(ResourceError::ManagerRpc("unexpected ensure_child".into()))
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            self.calls.lock().await.push("get");
            Ok(None)
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            self.calls.lock().await.push("view");
            Err(ResourceError::ManagerRpc("unexpected view".into()))
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.calls.lock().await.push("delete");
            let mut owned = self.owned.lock().await;
            if owned.iter().any(|row| row.key == *key) {
                owned.retain(|row| row.key != *key);
                Ok(())
            } else {
                Err(ResourceError::ManagerRpc("unexpected delete".into()))
            }
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            self.calls.lock().await.push("list-owned");
            Ok(self.owned.lock().await.clone())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            self.calls.lock().await.push("register-watch");
            Ok(WatchId(1))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            self.calls.lock().await.push("cancel-watch");
            Ok(())
        }
    }

    // -- fixtures ------------------------------------------------------------

    fn host_spec_bytes(provider_ref: Option<&str>) -> Vec<u8> {
        let base = to_base_object(&HostSpec::system_default()).expect("host base");
        let provider = provider_ref.map(|reference| ResourceRef::parse(reference).expect("ref"));
        ResourceSpec::new(provider, None, base, None)
            .expect("admitted resource spec")
            .canonical_bytes()
            .expect("canonical spec bytes")
    }

    fn row(spec: Vec<u8>) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", "Host", "host-system"),
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

    fn fixture(
        row: StoredDesiredResource,
        manager: Arc<RecordingManager>,
        requeue: RecordingRequeue,
    ) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            row,
            TargetHandle::Host,
            host_spec_decoder(),
            manager,
            Arc::new(requeue),
            effects_tx,
            notify_tx,
        )
    }

    async fn build_driver(effects: Arc<RecordingEffects>) -> Box<dyn DynResourceDriver> {
        // The driver's own typed seam, scripted: production builds the same
        // seam from the facets (the factory), tests drive the behavior
        // directly over the recording double.
        Box::new(HostDriver::new(effects))
    }

    async fn host_fixture() -> (
        ResourceContext,
        Arc<RecordingEffects>,
        Arc<RecordingManager>,
        RecordingRequeue,
        Box<dyn DynResourceDriver>,
    ) {
        let effects = RecordingEffects::new();
        let manager = RecordingManager::new();
        let requeue = RecordingRequeue::default();
        let ctx = fixture(
            row(host_spec_bytes(Some(HOST_PROVIDER_REF))),
            Arc::clone(&manager),
            requeue.clone(),
        );
        let driver = build_driver(Arc::clone(&effects)).await;
        (ctx, effects, manager, requeue, driver)
    }

/// The declaration registers the type and the registry serves the
    /// declared factory, so a Host row reaches its driver through the
    /// registry alone; the driver's effects come from the crate's own
    /// implementation over the facet set (U5), so no externally built port
    /// appears at the construction site.
    ///
    /// The facet set carries the scripted probe double, so the reconcile is
    /// hermetic: the real probe's local-machine observations stay the plane
    /// binding test's integration point.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_registry_serves_the_declared_factory_for_a_host_row() {
        let mut providers = ProviderDirectory::new();
        providers
            .register_driver(&host_descriptor(scripted_facets(RecordingProbe::new(Vec::new()))))
            .expect("the declaration registers");

        let key = ResourceKey::new("work", "Host", "host-system");
        let mut driver = providers.create_driver(&key).await.expect("the registry serves Host");
        let mut ctx = fixture(
            row(host_spec_bytes(Some(HOST_PROVIDER_REF))),
            RecordingManager::new(),
            RecordingRequeue::default(),
        );
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied,
            "the scripted probe observes the system-only bootstrap Host as Ready"
        );
        let status = ctx.status::<HostDriverStatus>().expect("status published");
        assert_eq!(status.report().status.phase, ResourcePhase::Ready);
    }

    // -- validate ------------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_accepts_the_bootstrap_host_row() {
        let (mut ctx, _effects, _manager, _requeue, mut driver) = host_fixture().await;
        driver
            .validate(&mut ctx)
            .await
            .expect("bootstrap Host validates");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_rejects_a_host_with_a_foreign_provider() {
        let mut ctx = fixture(
            row(host_spec_bytes(Some("Provider/network-local"))),
            RecordingManager::new(),
            RecordingRequeue::default(),
        );
        let mut driver = build_driver(RecordingEffects::new()).await;
        let failure = driver.validate(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_rejects_a_host_without_a_provider_ref() {
        let mut ctx = fixture(
            row(host_spec_bytes(None)),
            RecordingManager::new(),
            RecordingRequeue::default(),
        );
        let mut driver = build_driver(RecordingEffects::new()).await;
        let failure = driver.validate(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_rejects_a_malformed_host_spec() {
        let mut ctx = fixture(
            row(br#"{"nonsense":true}"#.to_vec()),
            RecordingManager::new(),
            RecordingRequeue::default(),
        );
        let mut driver = build_driver(RecordingEffects::new()).await;
        let failure = driver.validate(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    // -- recover -------------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recover_adopts_without_touching_the_target() {
        let (mut ctx, effects, _manager, _requeue, mut driver) = host_fixture().await;
        assert_eq!(
            driver.recover(&mut ctx).await.expect("recover"),
            RecoveryOutcome::Adopted
        );
        assert!(effects.call_order().is_empty());
        assert!(ctx.status::<HostDriverStatus>().is_none());
    }

    // -- reconcile -----------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_observes_once_per_desired_generation() {
        let (mut ctx, effects, manager, _requeue, mut driver) = host_fixture().await;
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(effects.call_order(), vec!["observe-host".to_owned()]);
        let status = ctx.status::<HostDriverStatus>().expect("status published");
        assert_eq!(status.observed_generation(), 1);
        assert_eq!(
            status.report().status.phase,
            ResourcePhase::Ready,
            "the published projection is the typed Host report"
        );

        // The preserved plan short-circuit: a status observed at the current
        // generation does not re-probe (the old runner's 5s relist was a
        // no-op in exactly this state).
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("second reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            effects.call_order(),
            vec!["observe-host".to_owned()],
            "one observation per desired generation"
        );
        assert!(manager.call_order().is_empty());
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_publishes_a_degraded_host_observation() {
        let (mut ctx, effects, _manager, requeue, mut driver) = host_fixture().await;
        effects.set_phase(ResourcePhase::Degraded);
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("reconcile"),
            ReconcileOutcome::RetryScheduled,
            "a degraded observation is not a converged Host: `Satisfied` would publish Ready \
             and wake every watcher on this Host's readiness"
        );
        assert_eq!(
            requeue.scheduled().len(),
            1,
            "the degraded observation re-probes instead of pinning itself for the generation"
        );
        let report = ctx
            .status::<HostDriverStatus>()
            .expect("status")
            .report();
        assert_eq!(report.status.phase, ResourcePhase::Degraded);
        assert_eq!(report.capabilities, vec![HostCapabilityClass::Kvm]);
        assert!(report.minijail_ready);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_maps_a_probe_failure_to_a_retryable_failure() {
        let (mut ctx, effects, _manager, _requeue, mut driver) = host_fixture().await;
        effects.fail.store(true, Ordering::SeqCst);
        let failure = driver.reconcile(&mut ctx).await.expect_err("retryable");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert!(ctx.status::<HostDriverStatus>().is_none());
    }

    // -- finalize: owned children retire before the delete no-op (F3) ---------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn finalize_finalizes_owned_children_before_the_delete_noop() {
        let effects = RecordingEffects::new();
        let manager = RecordingManager::new();
        manager.seed_owned(ResourceKey::new("work", "Process", "system-core-child"));
        let requeue = RecordingRequeue::default();
        let mut ctx = fixture(
            row(host_spec_bytes(Some(HOST_PROVIDER_REF))),
            Arc::clone(&manager),
            requeue,
        );
        let mut d = build_driver(effects).await;

        // A live owned child: the pass requeues instead of converging.
        let failure = d.finalize(&mut ctx).await.expect_err("owned child still live");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert!(
            manager.call_order().contains(&"delete"),
            "the owned child is nudged through its own finalize-before-delete pass"
        );

        // The manager removed the retired child row: the same pass converges.
        d.finalize(&mut ctx).await.expect("converged once the child retired");
    }

    // -- delete --------------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn delete_converges_without_effects_or_child_mutation() {
        let (mut ctx, effects, manager, _requeue, mut driver) = host_fixture().await;
        driver.delete(&mut ctx).await.expect("delete");
        assert!(effects.call_order().is_empty());
        assert!(manager.call_order().is_empty());
    }

    // -- no-spawn surface (KTD13) --------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn driver_operations_stay_off_every_spawn_surface() {
        let (mut ctx, _effects, manager, requeue, mut driver) = host_fixture().await;
        driver.validate(&mut ctx).await.expect("validate");
        driver.recover(&mut ctx).await.expect("recover");
        driver.reconcile(&mut ctx).await.expect("reconcile");
        driver.delete(&mut ctx).await.expect("delete");
        assert!(
            manager.call_order().is_empty(),
            "the family owns no children and mutates nothing through the manager: {:?}",
            manager.call_order()
        );
        assert_eq!(
            requeue.scheduled().len(),
            0,
            "no self-requeue: the old runner's 5s relist never re-observed a current status"
        );
    }
}
