//! The User resource driver: the v3 `ResourceDriver` conversion of the `User`
//! handler the shared Core Runner used to execute.
//!
//! `User` is one of the two bootstrap ResourceTypes `system-core` observes on
//! the local machine: local NSS discovery of the declared identity. The type
//! realizes nothing on a target and owns no child rows, so the driver is an
//! observation surface: `recover` adopts without effects, `reconcile`
//! publishes the typed in-memory status (R11), and `delete` converges without
//! effects (the manager already cascaded the row's - empty - owned-child set).
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`user_descriptor`] registration under `User`.
//! - `validate_spec` -> [`ResourceDriver::validate`]: typed spec decode.
//! - `plan` -> the preserved `observedGeneration` short-circuit: a status
//!   observed at the current generation skips re-discovering (the old
//!   `ResourceReconciler::plan` returned a converged plan in that case).
//! - `observe` -> [`ResourceDriver::recover`] (old `ObservationResult` was
//!   converged: nothing to adopt).
//! - `execute_effect` (old `status_candidate`) -> [`ResourceDriver::reconcile`]
//!   over the [`UserDriverEffects`] discovery port.
//! - `finalize` -> [`ResourceDriver::delete`] (old `FinalizeResult` was
//!   converged: the family owns no children and carries no finalizer).
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11).
//!
//! Deliberately not carried from the old handler: the durable
//! `status.resource` JSON projection and its `lastReconciledAt` /
//! `observedGeneration` writes (status is runtime-only now, R11), and the
//! `assess_update` / `plan_upgrade` runner path (no driver equivalent; the
//! family never planned an upgrade). The old runner's 5s resync relisted and
//! did nothing whenever the status was current, so no periodic re-discovery
//! is reproduced for a realized identity; a pass whose own discovery is not
//! yet realized re-checks itself ([`USER_REDISCOVER`]).
//!
//! KTD13: the driver has no spawn surface at all. It discovers the local
//! identity through the family's own probe behind the driver effects seam
//! (U5) - the same implementation value that serves the declared effects
//! service - and owns no Process.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ResourcePhase, ResourceRef, ResourceSpec,
    user::{USER_RESOURCE_TYPE, UserSpec},
};
use d2b_provider_system_core::UserStatusReport;
use d2b_resource_runtime::context::{ResourceContext, SpecDecoder, typed_spec_decoder};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};

use crate::effects_service::{USER_EFFECTS_SERVICE, UserEffectsService};
use crate::facets::UserEffectFacets;
use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};

/// How soon a User discovery that is not `Ready` is re-discovered.
///
/// Echoes the old runner's 5s resync, which is what made an absent or drifted
/// identity catch up: discovery reads the local machine, and the identity can
/// appear or drift with no spec write, so the phases it reports as not ready
/// resolve without a generation bump. The re-check is one discovery per
/// interval, never a poll on the ready path, which the generation
/// short-circuit still pins.
pub const USER_REDISCOVER: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserDriverErrorKind {
    /// The durable spec did not decode as the closed User contract.
    SpecInvalid,
    /// Local NSS discovery failed transiently.
    UserDiscovery,
    /// Owned children are still retiring; the delete pass requeues.
    DrainPending,
}

impl UserDriverErrorKind {
    /// The registered failure kind this classification reports.
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::SYSTEM_CORE_SPEC_INVALID,
            Self::UserDiscovery => FailureKinds::SYSTEM_CORE_USER_DISCOVERY_FAILED,
            Self::DrainPending => FailureKinds::SYSTEM_CORE_DRAIN_PENDING,
        }
    }
}

/// Typed driver failure; mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`].
#[derive(Debug, Clone)]
pub struct UserDriverError {
    kind: UserDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl UserDriverError {
    const fn new(kind: UserDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op, detail: FailureDetail::new() }
    }

    /// Attach the operator-visible detail of this failure.
    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for UserDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            UserDriverErrorKind::SpecInvalid => "system-core-spec-invalid",
            UserDriverErrorKind::UserDiscovery => "system-core-user-discovery-failed",
            UserDriverErrorKind::DrainPending => "system-core-drain-pending",
        })
    }
}

impl std::error::Error for UserDriverError {}

/// Typed in-memory status projection (R11: never persisted).
///
/// The discovery is kept with the generation it was taken at, which is the
/// runtime-only successor of the old durable `status.observedGeneration`
/// plan short-circuit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserDriverStatus {
    observed_generation: u64,
    report: UserStatusReport,
}

impl UserDriverStatus {
    /// The desired generation this discovery was taken at.
    pub const fn observed_generation(&self) -> u64 {
        self.observed_generation
    }

    /// The typed User discovery report this status publishes.
    pub const fn report(&self) -> &UserStatusReport {
        &self.report
    }
}

// ---------------------------------------------------------------------------
// Spec decode hook
// ---------------------------------------------------------------------------

/// The manager-wired decode hook for User rows.
///
/// The universal spec layer carries `providerRef` / `updatePolicy`; the base
/// keeps exactly the typed User contract fields, so the decoder hands the
/// driver the complete desired state (`ResourceSpec::base()` is what the old
/// handler decoded into `UserSpec`).
pub fn user_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| serde_json::from_slice::<ResourceSpec>(bytes))
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The discovery surface the User driver needs, served by this crate's own
/// [`crate::effects_service::UserEffectsService`] (U5): bounded local NSS
/// discovery over the facet-carried probe, reconciled through the preserved
/// `system-core` `UserReconciler`. The erased seam is what driver tests
/// script.
#[async_trait]
pub trait UserDriverEffects: Send + Sync + 'static {
    /// Discover one declared User and compute its public status, or report
    /// why discovery could not complete.
    async fn observe_user(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<UserStatusReport, String>;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// [`ResourceDriverFactory`] for the `User` resource type. Construction is
/// infallible by contract: the effects carry no fallible setup.
pub struct UserDriverFactory {
    types: [ResourceTypeName; 1],
    effects: Arc<dyn UserDriverEffects>,
}

impl UserDriverFactory {
    /// Build the factory over the family's own effects implementation
    /// (U5), constructed from the daemon-supplied facet set: the
    /// construction site holds no externally built port (R2).
    pub fn new(facets: UserEffectFacets) -> Self {
        Self {
            types: [ResourceTypeName::new(USER_RESOURCE_TYPE)],
            effects: Arc::new(UserEffectsService::new(facets)),
        }
    }
}

#[async_trait]
impl ResourceDriverFactory for UserDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(UserDriver::new(Arc::clone(&self.effects)))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One User resource's driver.
pub struct UserDriver {
    effects: Arc<dyn UserDriverEffects>,
}

impl UserDriver {
    /// Build one resource's driver over the local-discovery port.
    pub fn new(effects: Arc<dyn UserDriverEffects>) -> Self {
        Self { effects }
    }

    fn error(&self, kind: UserDriverErrorKind, op: DriverOp) -> UserDriverError {
        UserDriverError::new(kind, op)
    }

    fn resource_ref(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<ResourceRef, UserDriverError> {
        let type_name = d2b_contracts_resource::v3::ResourceTypeName::parse(
            ctx.key().type_name.clone(),
        )
        .map_err(|_| self.error(UserDriverErrorKind::SpecInvalid, op))?;
        let name = d2b_contracts_resource::v3::ResourceName::parse(ctx.key().name.clone())
            .map_err(|_| self.error(UserDriverErrorKind::SpecInvalid, op))?;
        Ok(ResourceRef::new(type_name, name))
    }

    /// The typed User base spec, decoded from the stored envelope.
    fn user_spec(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<UserSpec, UserDriverError> {
        let envelope = ctx
            .spec::<ResourceSpec>()
            .map_err(|_| self.error(UserDriverErrorKind::SpecInvalid, op))?;
        serde_json::from_slice::<UserSpec>(&envelope.base().to_canonical_bytes())
            .map_err(|_| self.error(UserDriverErrorKind::SpecInvalid, op))
    }

    /// The generation this actor's in-memory status was observed at, if the
    /// status belongs to this driver and the same generation.
    fn observed_generation(&self, ctx: &ResourceContext) -> Option<u64> {
        ctx.status::<UserDriverStatus>()
            .map(UserDriverStatus::observed_generation)
    }

    /// Whether the stored discovery is the one a converged User publishes.
    ///
    /// The generation short-circuit answers "one discovery per desired
    /// generation", and an unrealized discovery is stored at the same
    /// generation - so without this the short-circuit would pin a `Pending`
    /// (no such identity), `Degraded` (a drifted one), or `Unknown` (an
    /// unverified one) row for as long as its spec stands still.
    fn observed_ready(&self, ctx: &ResourceContext) -> bool {
        ctx.status::<UserDriverStatus>()
            .is_some_and(|status| status.report().phase == ResourcePhase::Ready)
    }
}

#[async_trait]
impl ResourceDriver for UserDriver {
    type Error = UserDriverError;

    fn classify_error(&self, error: &UserDriverError) -> DriverFailure {
        let failure = match error.kind {
            UserDriverErrorKind::SpecInvalid => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            UserDriverErrorKind::DrainPending => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            UserDriverErrorKind::UserDiscovery => DriverFailure::error(
                error.op,
                error.kind.failure_kind(),
                FailureClass::Retryable,
            ),
        };
        failure.with_detail(error.detail.clone())
    }

    /// Typed spec decode (old `validate_spec`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        self.user_spec(ctx, DriverOp::Validate)?;
        Ok(())
    }

    /// The old `observe` was converged with nothing to adopt: the family
    /// realizes no target-local state and owns no child rows.
    async fn recover(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<RecoveryOutcome, Self::Error> {
        self.user_spec(ctx, DriverOp::Recover)?;
        Ok(RecoveryOutcome::Adopted)
    }

    /// Discover the declared identity through the effects port and publish
    /// the typed status (R11). A discovery that is not realized schedules its
    /// own bounded re-check and answers `RetryScheduled`, so the runtime
    /// publishes the honest `Pending` phase rather than a claimed `Ready`.
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let generation = ctx.generation();
        // One discovery per desired generation - while that discovery is the
        // realized one. A generation pinned on an unrealized discovery must be
        // re-discovered, or the User claims an identity the discovery never
        // found.
        if self.observed_generation(ctx) == Some(generation) && self.observed_ready(ctx) {
            return Ok(ReconcileOutcome::Satisfied);
        }
        let spec = self.user_spec(ctx, DriverOp::Reconcile)?;
        let user_ref = self.resource_ref(ctx, DriverOp::Reconcile)?;
        let report = self.effects.observe_user(&user_ref, &spec).await.map_err(|error| {
            self.error(UserDriverErrorKind::UserDiscovery, DriverOp::Reconcile)
                .with_detail(
                    FailureDetail::at("user/discovery")
                        .comparison(FailureComparison::new(
                            "user.discovery",
                            "discovered",
                            "failed",
                        ))
                        .with_note(error),
                )
        })?;
        let ready = report.phase == ResourcePhase::Ready;
        ctx.set_status(UserDriverStatus {
            observed_generation: generation,
            report,
        });
        if ready {
            return Ok(ReconcileOutcome::Satisfied);
        }
        // An unrealized discovery is not a converged User. The identity is
        // local state that appears or drifts with no spec write, so the row
        // re-discovers rather than pinning the absence for the rest of its
        // generation.
        ctx.requeue_after(USER_REDISCOVER);
        Ok(ReconcileOutcome::RetryScheduled)
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown. The family owns no child in production, so
    /// this converges immediately; an owned row still live requeues the pass.
    /// Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| self.error(UserDriverErrorKind::DrainPending, DriverOp::Delete))?;
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

/// The User type's driver declaration.
///
/// `User` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane cannot serve the
/// converted bootstrap rows without it, so it must be registered before the
/// plane opens. The type is not exportable: `ResourceExport` admits only
/// qualified `*.d2bus.org.*Service` types. The driver serves no broker
/// operations, creates no children, and reads no other resource: discovery
/// reaches the local machine through the family's own probe. `User` names
/// no placement anchor, so a User row never carries the canonical
/// `spec.executionRef` and the plane reconciles it on its own Host domain -
/// the machine whose local identity it names.
///
/// U5: the declaration builds the family's own effects implementation from
/// the daemon-supplied facet set - no externally built port appears at any
/// construction site (R2) - and carries the family's declared effects
/// service, so a zone that cannot host it refuses startup by name (R5).
pub fn user_descriptor(facets: UserEffectFacets) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::USER,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: &["host"],
        exportable: false,
        reads: &[],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[USER_EFFECTS_SERVICE],
        decoder: user_spec_decoder(),
        factory: Arc::new(UserDriverFactory::new(facets)),
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a recording effects/manager double (the
// no-spawn and one-discovery-per-generation invariants are asserted through
// the recorded calls).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use d2b_contracts_resource::v3::{
        ResourcePhase, ResourceSpec,
        execution_policy::to_base_object,
        user::{OsUsername, UserSpec},
    };
    use d2b_provider_system_core::UserDiscoveryCondition;
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, WatchId,
        WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::provider::ProviderDirectory;
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;

    use crate::test_support::{RecordingEffects, ScriptedProbe, recording_facets};

    use super::{
        USER_REDISCOVER, UserDriver, UserDriverFactory, UserDriverStatus, user_descriptor,
        user_spec_decoder,
    };
    use crate::UserEffectFacets;

    // -- fakes ---------------------------------------------------------------

/// Recording manager:the family must never mutate children or registers;
    /// any unexpected manager call fails the test loudly through the recorded
    /// call list.
    struct RecordingManager {
        calls: parking_lot::Mutex<Vec<&'static str>>,
        owned: parking_lot::Mutex<Vec<StoredDesiredResource>>,
    }

    impl RecordingManager {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                owned: parking_lot::Mutex::new(Vec::new()),
            })
        }

        /// Seed one owned child row (the finalize gate's input).
        fn seed_owned(&self, key: ResourceKey) {
            self.owned.lock().push(StoredDesiredResource {
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
            self.calls.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            self.calls.lock().push("ensure-child"); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            Err(ResourceError::ManagerRpc("unexpected ensure_child".into()))
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            self.calls.lock().push("get"); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            Ok(None)
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            self.calls.lock().push("view"); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            Err(ResourceError::ManagerRpc("unexpected view".into()))
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.calls.lock().push("delete"); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            let mut owned = self.owned.lock(); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
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
            self.calls.lock().push("list-owned");
            Ok(self.owned.lock().clone())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            self.calls.lock().push("register-watch"); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            Ok(WatchId(1))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            self.calls.lock().push("cancel-watch"); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            Ok(())
        }
    }

    struct RecordingRequeue {
        calls: parking_lot::Mutex<Vec<u64>>,
    }

    impl RecordingRequeue {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
            })
        }

        fn call_count(&self) -> usize {
            self.calls.lock().len()
        }

        /// The scheduled delays in milliseconds, in arrival order.
        fn calls(&self) -> Vec<u64> {
            self.calls.lock().clone()
        }
    }

    impl RequeueScheduler for RecordingRequeue {
        fn schedule(&self, _key: ResourceKey, after: std::time::Duration) -> RequeueId {
            self.calls.lock().push(after.as_millis() as u64);
            RequeueId(0)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    // -- fixtures ------------------------------------------------------------

    fn user_spec_bytes() -> Vec<u8> {
        let spec = UserSpec::minimal(OsUsername::parse("alice").expect("username"));
        let base = to_base_object(&spec).expect("user base");
        ResourceSpec::new(None, None, base, None)
            .expect("admitted resource spec")
            .canonical_bytes()
            .expect("canonical spec bytes")
    }

    fn row(spec: Vec<u8>) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", "User", "alice"),
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
        requeue: Arc<RecordingRequeue>,
    ) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            row,
            TargetHandle::Host,
            user_spec_decoder(),
            manager,
            requeue,
            effects_tx,
            notify_tx,
        )
    }

    async fn build_driver(effects: Arc<RecordingEffects>) -> Box<dyn DynResourceDriver> {
        // The driver's own typed seam, scripted: production builds the same
        // seam from the facets (the factory), tests drive the behavior
        // directly over the recording double.
        Box::new(UserDriver::new(effects))
    }

    /// The facet set the factory and declaration tests build over: the
    /// scripted probe double, exactly as the plane's test inputs build it.
    fn facets() -> UserEffectFacets {
        recording_facets(ScriptedProbe::new())
    }

    async fn user_fixture() -> (
        ResourceContext,
        Arc<RecordingEffects>,
        Arc<RecordingManager>,
        Arc<RecordingRequeue>,
        Box<dyn DynResourceDriver>,
    ) {
        let effects = RecordingEffects::new();
        let manager = RecordingManager::new();
        let requeue = RecordingRequeue::new();
        let ctx = fixture(row(user_spec_bytes()), Arc::clone(&manager), Arc::clone(&requeue));
        let driver = build_driver(Arc::clone(&effects)).await;
        (ctx, effects, manager, requeue, driver)
    }

    // -- factory -------------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn factory_registers_exactly_the_user_resource_type() {
        let factory = UserDriverFactory::new(facets());
        assert_eq!(factory.resource_types().len(), 1);
        assert_eq!(factory.resource_types()[0].as_str(), "User");
        factory
            .create(&ResourceKey::new("work", "User", "alice"))
            .await;
    }

    /// The declaration registers the type and the registry serves the
    /// declared factory, so a User row reaches its driver through the
    /// registry alone; the driver's effects come from the crate's own
    /// implementation over the facet set (U5), so no externally built port
    /// appears at the construction site. The facets carry the scripted
    /// probe, so the reconciled outcome is unconditional: the declared
    /// identity resolves as discovered and the driver publishes Ready on
    /// any host.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_registry_serves_the_declared_factory_for_a_user_row() {
        let probe = ScriptedProbe::new();
        let spec = UserSpec::minimal(OsUsername::parse("alice").expect("username"));
        let base = to_base_object(&spec).expect("user base");
        let envelope =
            ResourceSpec::new(None, None, base, None).expect("admitted resource spec");
        let mut providers = ProviderDirectory::new();
        providers
            .register_driver(&user_descriptor(recording_facets(probe)))
            .expect("the declaration registers");

        let key = ResourceKey::new("work", "User", "alice");
        let mut driver = providers.create_driver(&key).await.expect("the registry serves User");
        let mut ctx = fixture(
            row(envelope.canonical_bytes().expect("canonical spec bytes")),
            RecordingManager::new(),
            RecordingRequeue::new(),
        );
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied,
            "the facet-carried probe resolves the declared identity as discovered"
        );
        let status = ctx.status::<UserDriverStatus>().expect("status published");
        assert_eq!(status.report().phase, ResourcePhase::Ready);
        assert_eq!(status.report().discovery, UserDiscoveryCondition::Discovered);
    }

    // -- validate ------------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_accepts_the_bootstrap_user_row() {
        let (mut ctx, _effects, _manager, _requeue, mut driver) = user_fixture().await;
        driver
            .validate(&mut ctx)
            .await
            .expect("User spec validates without a provider ref (the old exact-fixture shape)");
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_rejects_a_malformed_user_spec() {
        let mut ctx = fixture(
            row(br#"{"nonsense":true}"#.to_vec()),
            RecordingManager::new(),
            RecordingRequeue::new(),
        );
        let mut driver = build_driver(RecordingEffects::new()).await;
        let failure = driver.validate(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.class(), FailureClass::Terminal);
    }

    // -- recover -------------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recover_adopts_without_touching_the_target() {
        let (mut ctx, effects, _manager, _requeue, mut driver) = user_fixture().await;
        assert_eq!(
            driver.recover(&mut ctx).await.expect("recover"),
            RecoveryOutcome::Adopted
        );
        assert!(effects.call_order().is_empty());
        assert!(ctx.status::<UserDriverStatus>().is_none());
    }

    // -- reconcile -----------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_discovers_once_per_desired_generation() {
        let (mut ctx, effects, manager, requeue, mut driver) = user_fixture().await;
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(effects.call_order(), vec!["observe-user".to_owned()]);
        let status = ctx.status::<UserDriverStatus>().expect("status published");
        assert_eq!(status.observed_generation(), 1);
        assert_eq!(status.report().discovery, UserDiscoveryCondition::Discovered);

        // The preserved plan short-circuit: a realized status observed at the
        // current generation does not re-discover (the old runner's 5s relist
        // was a no-op in exactly this state).
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("second reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            effects.call_order(),
            vec!["observe-user".to_owned()],
            "one discovery per desired generation"
        );
        assert_eq!(
            requeue.call_count(),
            0,
            "a realized discovery re-checks nothing"
        );
        assert!(manager.call_order().is_empty());
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_publishes_the_user_discovery_projection() {
        for phase in [
            ResourcePhase::Pending,
            ResourcePhase::Degraded,
            ResourcePhase::Unknown,
        ] {
            let (mut ctx, effects, _manager, requeue, mut driver) = user_fixture().await;
            effects.set_phase(phase);
            assert_eq!(
                driver.reconcile(&mut ctx).await.expect("reconcile"),
                ReconcileOutcome::RetryScheduled,
                "{phase:?} is not the realized identity: the pass must not claim it"
            );
            let status = ctx.status::<UserDriverStatus>().expect("status");
            assert_eq!(status.observed_generation(), 1);
            assert_eq!(status.report().phase, phase);
            assert_eq!(status.report().discovery, UserDiscoveryCondition::Discovered);
            assert_eq!(
                requeue.calls(),
                vec![USER_REDISCOVER.as_millis() as u64],
                "exactly one re-check, on the discovery cadence"
            );
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_rediscovers_a_cached_unrealized_phase() {
        let (mut ctx, effects, _manager, _requeue, mut driver) = user_fixture().await;
        effects.set_phase(ResourcePhase::Pending);
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("first reconcile"),
            ReconcileOutcome::RetryScheduled
        );

        // The cached status is current-generation but not realized, so the
        // next pass re-discovers instead of short-circuiting into a claim.
        effects.set_phase(ResourcePhase::Ready);
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("second reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            effects.call_order(),
            vec!["observe-user".to_owned(), "observe-user".to_owned()],
            "a cached unrealized discovery is re-observed"
        );

        // The realized status now short-circuits the third pass.
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("third reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            effects.call_order().len(),
            2,
            "one discovery per realized generation"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_maps_a_discovery_failure_to_a_retryable_failure() {
        let (mut ctx, effects, _manager, _requeue, mut driver) = user_fixture().await;
        effects.fail.store(true, Ordering::SeqCst);
        let failure = driver.reconcile(&mut ctx).await.expect_err("retryable");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert!(ctx.status::<UserDriverStatus>().is_none());
    }

    // -- finalize: owned children retire before the delete no-op (F3) ---------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn finalize_finalizes_owned_children_before_the_delete_noop() {
        let effects = RecordingEffects::new();
        let manager = RecordingManager::new();
        manager.seed_owned(ResourceKey::new("work", "Process", "system-core-child"));
        let requeue = RecordingRequeue::new();
        let mut ctx = fixture(row(user_spec_bytes()), Arc::clone(&manager), requeue);
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
        let (mut ctx, effects, manager, _requeue, mut driver) = user_fixture().await;
        driver.delete(&mut ctx).await.expect("delete");
        assert!(effects.call_order().is_empty());
        assert!(manager.call_order().is_empty());
    }

    // -- no-spawn surface (KTD13) --------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn driver_operations_stay_off_every_spawn_surface() {
        let (mut ctx, _effects, manager, requeue, mut driver) = user_fixture().await;
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
            requeue.call_count(),
            0,
            "no self-requeue: the observed identity was realized"
        );
    }
}
