//! Core resource driver (U12): the v3 `ResourceDriver` conversion of the
//! `CoreResourceReconciler` the shared Core Runner used to execute.
//!
//! The family owns the nine ResourceTypes the fixed Core process hosted
//! (the old `CORE_RESOURCE_CONTROLLER_REGISTRATIONS`): `Zone`, `ZoneLink`,
//! `Provider`,
//! `Role`, `RoleBinding`, `Quota`, `EmergencyPolicy`, `ResourceExport`, and
//! `ResourceImport`. None realizes anything on a target and none owns a
//! declared child set: the old handler was Core's baseline reconciler for
//! metadata-only convergence, so eight of the nine converge once their
//! desired state is admitted, and `Provider` alone carries behavior - the
//! readiness observation Core applies to the Provider's owned controller
//! `Process` rows and state `Volume` rows.
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`CoreResourceDriverFactory`] registration under the nine
//!   types (`resource_plane_v3`'s provider directory).
//! - `validate_spec` -> [`ResourceDriver::validate`]: the stored spec envelope
//!   must decode as the JSON spec object every core row stores. The old fence
//!   was "the canonical row JSON is non-empty", so a row whose spec is absent
//!   or undecodable is the same corrupt row.
//! - `plan` -> folded into [`ResourceDriver::reconcile`]: the old plan was an
//!   empty effect list (the `!ensure_finalizer` flag only selected the legacy
//!   constructor `CoreResourceReconciler::new`, which production never used).
//! - `observe` -> [`ResourceDriver::recover`]: the old `ObservationResult` was
//!   converged, so recovery adopts without effects.
//! - `reconcile`/`execute_effect` -> [`ResourceDriver::reconcile`] over the
//!   manager's owned rows; the `Provider` observation and phase policy stay
//!   the shared pure core ([`provider_observation`],
//!   [`ProviderHandler::plan_observed`], [`ProviderHandler::plan_system_core`],
//!   [`fixed_system_core_handlers_ready`]).
//! - `prepare_finalize`/`execute_finalize`/`finalize` -> [`ResourceDriver::finalize`]
//!   (the drain step before delete) and [`ResourceDriver::delete`] (see the
//!   finalizer mapping below).
//! - `health`/`drain` -> actor supervision and shutdown, not driver surface.
//! - `assess_update`/`plan_upgrade`/`execute_upgrade` -> no KTD3 equivalent:
//!   the old handler assessed every row `Current` with preserve-state and
//!   planned a no-op restart; the runner's upgrade path is gone with the
//!   runner (R30).
//! - `UpdateStatus` -> [`ResourceContext::set_status`] (in-memory only, R11).
//! - `DependencySnapshot` -> the manager's owned children plus, for the two
//!   internally hosted providers, the fixed `Host`/`Zone` dependency rows.
//!
//! ## Finalizer mapping (the per-type delete window)
//!
//! Seven of the nine types carried a drain finalizer in the old descriptor
//! (`ZoneLink`, `Provider`, `RoleBinding`, `Quota`, `EmergencyPolicy`,
//! `ResourceExport`, `ResourceImport`); `Zone` and `Role` carried none. The
//! old reconciler installed the descriptor finalizer in `reconcile` and
//! removed it in `prepare_finalize`, which is the old plane's delete window:
//! the row was held while a finalizer was present and retired once it was
//! gone.
//!
//! The new plane has no finalizer surface. Deletion is the manager's durable
//! mark-deleting, then [`ResourceDriver::finalize`] (drain), then
//! [`ResourceDriver::delete`], then row retirement. `finalize` is where the
//! old window is re-expressed: owned children are drained first (their own
//! finalize-before-delete pass, F3) and only a `Provider` has a second gate -
//! its controller `Process` children must retire before its row may. Every
//! core type's finalizer was otherwise bookkeeping: the drain predicates that
//! could gate it (`QuotaAuthority::drain_pending`,
//! `EmergencyPolicyAuthority::drain_pending`) have no production consumer, and
//! the old controller removed the finalizer unconditionally on the delete
//! path, so [`ResourceDriver::delete`] converges for every core type once the
//! drain step returns. Those two drain predicates keep their behavior where
//! they live (the pure `d2b-core-controller` handlers); re-expressing one as a
//! driver gate belongs to the type that gains a real drain effect, not to this
//! conversion.
//!
//! ## Slice boundary
//!
//! `resource_plane_v3` registers the factory with its spec decoder and the
//! production effects port, wired from the live controller-session seam the
//! G5 reader bridge already uses.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
};
use d2b_core_controller::providers::{
    ProviderHandler, ProviderIntent, ProviderObservation, ProviderPhase,
};
use d2b_core_controller::{
    DependencySnapshot, ResourceKey as CoreResourceKey, ResourceSnapshot,
    fixed_system_core_handlers_ready, provider_observation,
};
use d2b_resource_runtime::context::{ResourceContext, SpecDecoder, typed_spec_decoder};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds, ResourceError,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::manager::ResourceView;
use d2b_resource_runtime::resource::ResourceStatus;
use serde_json::{Value, json};

/// The nine ResourceTypes the fixed Core process hosts, in the old
/// registration order. Replaces `CORE_RESOURCE_CONTROLLER_REGISTRATIONS`.
pub(crate) const CORE_RESOURCE_TYPES: [&str; 9] = [
    "Zone",
    "ZoneLink",
    "Provider",
    "Role",
    "RoleBinding",
    "Quota",
    "EmergencyPolicy",
    "ResourceExport",
    "ResourceImport",
];

/// The one ResourceType in the family with behavior beyond convergence.
const PROVIDER_TYPE_NAME: &str = "Provider";

/// Convergence poll for a Provider row that is not yet Ready: its controller
/// `Process` children and their live session evidence both arrive after the
/// row's first pass, and neither carries an edge this driver can watch, so
/// the observation re-runs on this schedule until the phase leaves Pending.
const PROVIDER_CONVERGENCE_POLL: std::time::Duration = std::time::Duration::from_millis(1_000);

/// The two internally hosted Providers whose readiness reads the fixed
/// dependency set (`Host` plus the Zone's own row) instead of their children
/// (`SYSTEM_CORE_PROVIDER_REF`/`SYSTEM_MINIJAIL_PROVIDER_REF` in
/// `d2b-core-controller`, which stay private there).
const SYSTEM_CORE_PROVIDER_REF: &str = "Provider/system-core";
const SYSTEM_MINIJAIL_PROVIDER_REF: &str = "Provider/system-minijail";

/// The canonical Host row the fixed providers observe (`SYSTEM_CORE_HOST_REF`
/// in `d2b-core-controller`).
const SYSTEM_CORE_HOST_REF: &str = "Host/host-system";

// ---------------------------------------------------------------------------
// Driver error
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoreDriverErrorKind {
    /// The stored spec envelope did not decode as the JSON spec object the
    /// core types store, or the row identity could not be re-read as a
    /// contract reference (the old `validate_spec` fence and the old
    /// `CoreReconcileError` raised from row parsing, both terminal).
    SpecInvalid,
    /// A manager read the observation depends on failed. The old dependency
    /// read was a source read: transient, retried with backoff, and never a
    /// handler failure.
    DependencyRead,
    /// The drain the deletion must not cut through has not completed; the
    /// actor requeues another delete pass (`finalize` is idempotent).
    DrainPending,
}

impl CoreDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::SpecInvalid => FailureClass::Terminal,
            Self::DependencyRead | Self::DrainPending => FailureClass::Retryable,
        }
    }

    /// The registered failure kind this classification reports (issue #508).
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::CORE_SPEC_INVALID,
            Self::DependencyRead => FailureKinds::CORE_DEPENDENCY_READ_FAILED,
            Self::DrainPending => FailureKinds::CORE_DRAIN_PENDING,
        }
    }
}

/// Typed driver failure; mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`] (R13, issue
/// #508).
#[derive(Debug, Clone)]
pub(crate) struct CoreDriverError {
    kind: CoreDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl CoreDriverError {
    const fn new(kind: CoreDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op, detail: FailureDetail::new() }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }

    /// Return the closed failure class this error is reported with.
    pub(crate) const fn class(&self) -> FailureClass {
        self.kind.class()
    }

    /// Return the operation that failed.
    pub(crate) const fn op(&self) -> DriverOp {
        self.op
    }
}

impl core::fmt::Display for CoreDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            CoreDriverErrorKind::SpecInvalid => "core-spec-invalid",
            CoreDriverErrorKind::DependencyRead => "core-dependency-read-failed",
            CoreDriverErrorKind::DrainPending => "core-drain-pending",
        })
    }
}

impl std::error::Error for CoreDriverError {}

// ---------------------------------------------------------------------------
// In-memory status
// ---------------------------------------------------------------------------

/// Typed in-memory status projection (R11: never persisted).
///
/// The old plane persisted `phase`, `observedGeneration`, the
/// `status.resource.providerReadiness` projection, and the store-derived
/// `status.resource.owned.refs` list. Nothing durable replaces them: the
/// generation the old `Enable`/`Update` short-circuit read and the projected
/// phase are kept here, the readiness fields stay on the typed
/// [`ProviderObservation`], and the last observed owned `Volume` references
/// are carried because the pure core's `expected_provider_volume_refs` fence
/// compares them with the current dependency list (a declared state Volume
/// that disappeared still fails the provider).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CoreDriverStatus {
    /// The `Provider` observation, at the generation it was taken.
    Provider {
        /// The desired generation this observation was taken at (the old
        /// `status.observedGeneration`).
        observed_generation: u64,
        phase: ProviderPhase,
        observation: ProviderObservation,
        /// The `Volume` references among the Provider's owned rows at the last
        /// published observation.
        volume_refs: BTreeSet<String>,
    },
}

impl CoreDriverStatus {
    /// The desired generation this observation was taken at.
    pub(crate) const fn observed_generation(&self) -> u64 {
        match self {
            Self::Provider {
                observed_generation, ..
            } => *observed_generation,
        }
    }

    /// The projected Provider phase.
    pub(crate) const fn phase(&self) -> ProviderPhase {
        match self {
            Self::Provider { phase, .. } => *phase,
        }
    }

    /// The observation the last pass computed.
    pub(crate) const fn observation(&self) -> ProviderObservation {
        match self {
            Self::Provider { observation, .. } => *observation,
        }
    }

    /// The `Volume` references the last observation carried.
    pub(crate) fn volume_refs(&self) -> &BTreeSet<String> {
        match self {
            Self::Provider { volume_refs, .. } => volume_refs,
        }
    }
}

// ---------------------------------------------------------------------------
// Effects port
// ---------------------------------------------------------------------------

/// The live evidence the Core `Provider` observation needs and the manager
/// cannot serve: a converted row's status is in-memory only (R11), so the
/// manager's view carries the closed [`ResourceStatus`] and never the
/// controller-session evidence the pure core reads as
/// `status.resource.controllerSession`. The production implementation is the
/// same live-session seam the G5 reader bridge uses; every unknown fails
/// closed (`None`), so no caller can synthesize an admitted session.
pub(crate) trait CoreDriverEffects: Send + Sync + 'static {
    /// The live admitted controller session for one controller `Process` row,
    /// or `None` when no session is admitted for this exact row identity and
    /// generation.
    fn controller_session_evidence(
        &self,
        process_ref: &ResourceRef,
        process_uid: &ResourceUid,
        generation: ResourceGeneration,
    ) -> Option<Value>;
}

/// Fail-closed default: no session is ever reported admitted. A Provider with
/// controller children then observes `conformance_valid: false`, exactly as
/// the old handler did for a Process row without session evidence.
pub(crate) struct FailClosedCoreDriverEffects;

impl CoreDriverEffects for FailClosedCoreDriverEffects {
    fn controller_session_evidence(
        &self,
        _process_ref: &ResourceRef,
        _process_uid: &ResourceUid,
        _generation: ResourceGeneration,
    ) -> Option<Value> {
        None
    }
}

// ---------------------------------------------------------------------------
// Spec decode hook
// ---------------------------------------------------------------------------

/// The manager-wired decode hook for the nine core types: the stored spec
/// envelope is the JSON spec object (the core types have no typed core spec -
/// the old handler worked from canonical JSON, and Core's provider handler
/// reads `spec.artifactId` / `spec.config` untyped).
pub(crate) fn core_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| serde_json::from_slice::<Value>(bytes))
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// [`ResourceDriverFactory`] for the nine core ResourceTypes. Construction is
/// infallible by contract (R3): the production effects carry no fallible
/// setup.
pub(crate) struct CoreResourceDriverFactory {
    types: [ResourceTypeName; 9],
    effects: Arc<dyn CoreDriverEffects>,
}

impl CoreResourceDriverFactory {
    /// Construct over the fail-closed effects default (unit fixtures).
    pub(crate) fn new() -> Self {
        Self::with_effects(Arc::new(FailClosedCoreDriverEffects))
    }

    /// Construct over an injected port. The plane composition wires the live
    /// controller-session seam here.
    pub(crate) fn with_effects(effects: Arc<dyn CoreDriverEffects>) -> Self {
        Self {
            types: CORE_RESOURCE_TYPES.map(ResourceTypeName::new),
            effects,
        }
    }
}

#[async_trait]
impl ResourceDriverFactory for CoreResourceDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(CoreResourceDriver {
            effects: Arc::clone(&self.effects),
        })
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One core resource's driver.
pub(crate) struct CoreResourceDriver {
    effects: Arc<dyn CoreDriverEffects>,
}

impl CoreResourceDriver {
    fn error(&self, kind: CoreDriverErrorKind, op: DriverOp) -> CoreDriverError {
        CoreDriverError::new(kind, op)
    }

    fn spec(&self, ctx: &ResourceContext, op: DriverOp) -> Result<Value, CoreDriverError> {
        let spec = ctx.spec::<Value>().map_err(|error| {
            self.error(CoreDriverErrorKind::SpecInvalid, op).with_detail(
                FailureDetail::at("spec/decode").with_note(error.to_string()),
            )
        })?;
        if !spec.is_object() {
            let shape = match &spec {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            return Err(self.error(CoreDriverErrorKind::SpecInvalid, op).with_detail(
                FailureDetail::at("spec/shape").comparison(FailureComparison::new(
                    "spec.shape",
                    "object",
                    shape,
                )),
            ));
        }
        Ok(spec.clone())
    }

    fn resource_ref(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<ResourceRef, CoreDriverError> {
        ResourceRef::parse(&format!("{}/{}", ctx.key().type_name, ctx.key().name)).map_err(
            |error| {
                self.error(CoreDriverErrorKind::SpecInvalid, op).with_detail(
                    FailureDetail::at("spec/ref").with_note(error.to_string()),
                )
            },
        )
    }

    /// One resource view as the Core dependency snapshot the pure observation
    /// reads: identity and ownership from the manager row, `phase` and
    /// `observedGeneration` from the generation-filtered live status, and the
    /// live session evidence for a controller `Process` row.
    fn dependency_snapshot(
        &self,
        view: &ResourceView,
        provider_ref: &str,
        provider_uid: &ResourceUid,
        provider_generation: ResourceGeneration,
    ) -> Option<DependencySnapshot> {
        let resource_ref =
            ResourceRef::parse(&format!("{}/{}", view.key.type_name, view.key.name)).ok()?;
        let zone = ZoneId::parse(view.key.zone.clone()).ok()?;
        let uid = resource_uid(&view.uid)?;
        let generation = ResourceGeneration::new(view.generation).ok()?;
        let mut metadata: Value = serde_json::from_slice(&view.metadata).ok()?;
        let spec: Value = serde_json::from_slice(&view.spec).ok()?;
        let mut status = observed_status(view);
        if resource_ref.resource_type().as_str() == "Process"
            && let Some(evidence) =
                self.effects
                    .controller_session_evidence(&resource_ref, &uid, generation)
        {
            status["resource"] = json!({ "controllerSession": evidence });
        }
        // The manager's ownership is authoritative (R8): a row listed as this
        // Provider's child carries the Provider reference in the synthesized
        // payload, because the pure core reads ownership from
        // `metadata.ownerRef`, not from the manager.
        metadata
            .as_object_mut()?
            .insert("ownerRef".to_owned(), Value::String(provider_ref.to_owned()));
        let canonical = serde_json::to_vec(&json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": view.key.type_name,
            "metadata": metadata,
            "spec": spec,
            "status": status,
        }))
        .ok()?;
        Some(DependencySnapshot::new(
            ResourceSnapshot::new(
                CoreResourceKey::new(zone, resource_ref, uid),
                ZoneRevision::new(view.generation),
                generation,
                canonical,
                view.deleting,
            )
            .with_owner_identity(Some(provider_uid.clone()), Some(provider_generation)),
        ))
    }
}

/// Whether one owned row is a controller `Process`: the Provider's controller
/// component, whose retirement the Process controller owns and which must
/// complete before the Provider's own row may retire (KTD13, F3). The manager
/// lists ownership by uid (R8), so the row being present in the owned-child
/// read is the ownership proof; the spec decides the class.
fn is_controller_process(row: &d2b_resource_runtime::identity::StoredDesiredResource) -> bool {
    row.key.type_name == "Process"
        && serde_json::from_slice::<Value>(&row.spec).is_ok_and(|spec| {
            spec.get("processClass").and_then(Value::as_str) == Some("controller")
        })
}

/// The status projection for one manager row. `observed_status()` is the only
/// status the manager vouches for: a status published for an older row
/// generation is not observed state of the current row and is therefore never
/// reported as `Ready`.
fn observed_status(view: &ResourceView) -> Value {
    match view.observed_status() {
        Some(ResourceStatus::Ready) => json!({
            "phase": "Ready",
            "observedGeneration": view.generation,
        }),
        // A failed driver is the closed classification the old plane reported
        // as a degraded component; it is never `Ready`.
        Some(ResourceStatus::Failed(_)) => json!({ "phase": "Degraded" }),
        Some(_) | None => json!({ "phase": "Pending" }),
    }
}

/// Map a manager row's 16-byte deterministic uid onto the contracts crate's
/// UUIDv4-shaped `ResourceUid` (the same mapping the converted drivers use).
fn resource_uid(bytes: &[u8; 16]) -> Option<ResourceUid> {
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

#[async_trait]
impl ResourceDriver for CoreResourceDriver {
    type Error = CoreDriverError;

    fn classify_error(&self, error: &CoreDriverError) -> DriverFailure {
        let failure = match error.kind {
            CoreDriverErrorKind::SpecInvalid => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            CoreDriverErrorKind::DrainPending => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            CoreDriverErrorKind::DependencyRead => {
                DriverFailure::error(error.op, error.kind.failure_kind(), FailureClass::Retryable)
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// The stored spec object fence (old `validate_spec`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let _spec = self.spec(ctx, DriverOp::Validate)?;
        Ok(())
    }

    /// The old `observe` was converged with nothing to adopt: the family
    /// realizes no target-local state, and its old finalizer bookkeeping has
    /// no recovery surface on the new plane.
    async fn recover(
        &mut self,
        _ctx: &mut ResourceContext,
    ) -> Result<RecoveryOutcome, Self::Error> {
        Ok(RecoveryOutcome::Adopted)
    }

    /// One reconcile pass. Eight of the nine types converged in the old
    /// handler as soon as their finalizer bookkeeping was current, which the
    /// manager's durable deleting mark now owns; `Provider` re-observes its
    /// dependent rows and republishes its phase in memory (R11).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        if ctx.key().type_name != PROVIDER_TYPE_NAME {
            return Ok(ReconcileOutcome::Satisfied);
        }
        self.reconcile_provider(ctx).await?;
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Drain step before [`ResourceDriver::delete`] (R10, F3). Per type:
    ///
    /// - `Zone` and `Role` never carried a Core finalizer: nothing to drain.
    /// - `ZoneLink`, `RoleBinding`, `Quota`, `EmergencyPolicy`,
    ///   `ResourceExport`, `ResourceImport` carried a descriptor finalizer the
    ///   old controller released unconditionally on the delete path, and no
    ///   production drain state exists for them (`QuotaAuthority::drain_pending`
    ///   and `EmergencyPolicyAuthority::drain_pending` are pure core logic
    ///   without a live reservation source): immediate convergence preserves
    ///   the old window.
    /// - `Provider` is the one real drain: its controller `Process` children
    ///   must retire before its row may, so a controller Process row still
    ///   present among the owned children keeps the actor requeueing.
    ///
    /// Order is children first, then this type's drain: every owned child is
    /// nudged through its own finalize-before-delete pass, and the row
    /// requeues while any child row is still live. The erased boundary also
    /// calls [`ResourceContext::finalize_owned_resources`] as a backstop; the
    /// explicit call here keeps the ordering part of this driver's contract.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        match ctx.finalize_owned_resources().await {
            Ok(()) => {}
            Err(ResourceError::ChildrenDraining { .. }) => {
                return Err(self.error(CoreDriverErrorKind::DrainPending, DriverOp::Delete));
            }
            Err(error) => {
                return Err(self
                    .error(CoreDriverErrorKind::DependencyRead, DriverOp::Delete)
                    .with_detail(
                        FailureDetail::at("finalize/children")
                            .comparison(FailureComparison::new(
                                "owned.children",
                                "finalize answered",
                                "read failed",
                            ))
                            .with_note(error.to_string()),
                    ));
            }
        }
        self.finalize_pass(ctx).await
    }

    /// Teardown (see the module header's finalizer mapping): the durable
    /// deleting mark is already committed when this runs (R10), the family
    /// owns no children to retire first, and the drain predicates that carry
    /// real behavior stay in the pure core handlers.
    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl CoreResourceDriver {
    /// The drain gate behind [`ResourceDriver::finalize`]: only the Provider
    /// has dependents whose retirement must precede its own row's, and the
    /// manager's owned-child read is where they are visible. Idempotent under
    /// retry - it reads state and returns, releasing nothing.
    async fn finalize_pass(&self, ctx: &mut ResourceContext) -> Result<(), CoreDriverError> {
        if ctx.key().type_name != PROVIDER_TYPE_NAME {
            return Ok(());
        }
        let children = ctx.children().await.map_err(|error| {
            self.error(CoreDriverErrorKind::DependencyRead, DriverOp::Delete)
                .with_detail(
                    FailureDetail::at("finalize/children")
                        .comparison(FailureComparison::new(
                            "owned.children",
                            "read answered",
                            "read failed",
                        ))
                        .with_note(error.to_string()),
                )
        })?;
        if children.iter().any(is_controller_process) {
            return Err(self
                .error(CoreDriverErrorKind::DrainPending, DriverOp::Delete)
                .with_detail(
                    FailureDetail::at("finalize/drain").comparison(FailureComparison::new(
                        "owned.controllerProcess",
                        "retired",
                        "live",
                    )),
                ));
        }
        Ok(())
    }

    /// The preserved `Provider` pass: observe the owned controller `Process`
    /// and state `Volume` rows (plus the fixed `Host`/`Zone` dependency rows
    /// for the two internally hosted providers) and publish the phase the
    /// shared pure policy projects.
    async fn reconcile_provider(&self, ctx: &mut ResourceContext) -> Result<(), CoreDriverError> {
        let spec = self.spec(ctx, DriverOp::Reconcile)?;
        let provider_ref = self.resource_ref(ctx, DriverOp::Reconcile)?;
        let provider_uid = resource_uid(ctx.uid())
            .ok_or_else(|| self.error(CoreDriverErrorKind::SpecInvalid, DriverOp::Reconcile))?;
        let generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(CoreDriverErrorKind::SpecInvalid, DriverOp::Reconcile))?;
        let zone = ZoneId::parse(ctx.key().zone.clone())
            .map_err(|_| self.error(CoreDriverErrorKind::SpecInvalid, DriverOp::Reconcile))?;

        // The old `status.observedGeneration` short-circuit selected the
        // `Enable` intent; the in-memory status is its runtime-only successor.
        let previous = ctx.status::<CoreDriverStatus>().cloned();
        let intent = match previous.as_ref() {
            Some(status) if status.observed_generation() == ctx.generation() => {
                ProviderIntent::Enable
            }
            _ => ProviderIntent::Update,
        };

        let dependencies = self
            .dependencies(ctx, &provider_ref, &provider_uid, generation)
            .await?;

        // The provider row as the pure observation reads it: spec from the
        // stored envelope, metadata as authored, and the previous
        // observation's owned Volume references standing in for the durable
        // `status.resource.owned.refs` projection the store used to derive.
        let metadata: Value = serde_json::from_slice(ctx.metadata()).map_err(|_| {
            self.error(CoreDriverErrorKind::SpecInvalid, DriverOp::Reconcile)
        })?;
        let status = match previous.as_ref() {
            Some(previous) => json!({
                "observedGeneration": previous.observed_generation(),
                "resource": {
                    "owned": { "refs": previous.volume_refs().iter().collect::<Vec<_>>() },
                },
            }),
            None => json!({}),
        };
        let canonical = serde_json::to_vec(&json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": PROVIDER_TYPE_NAME,
            "metadata": metadata,
            "spec": spec,
            "status": status,
        }))
        .map_err(|_| self.error(CoreDriverErrorKind::SpecInvalid, DriverOp::Reconcile))?;
        let provider = ResourceSnapshot::new(
            CoreResourceKey::new(zone, provider_ref.clone(), provider_uid.clone()),
            ZoneRevision::new(ctx.generation()),
            generation,
            canonical,
            false,
        );
        let observation = provider_observation(&provider, &dependencies)
            .map_err(|_| self.error(CoreDriverErrorKind::SpecInvalid, DriverOp::Reconcile))?;

        let phase = if provider_ref.to_canonical_string() == SYSTEM_CORE_PROVIDER_REF {
            // The system-core exception: its readiness is the Zone's
            // mandatory-handler projection, not its own children.
            ProviderHandler::plan_system_core(fixed_system_core_handlers_ready(&dependencies))
                .phase()
        } else {
            ProviderHandler::plan_observed(&provider_ref, intent, observation)
                .map(|plan| plan.phase())
                .unwrap_or(ProviderPhase::Pending)
        };

        let volume_refs = dependencies
            .iter()
            .filter(|dependency| {
                dependency
                    .resource()
                    .key()
                    .resource_ref()
                    .resource_type()
                    .as_str()
                    == "Volume"
            })
            .map(|dependency| {
                dependency
                    .resource()
                    .key()
                    .resource_ref()
                    .to_canonical_string()
            })
            .collect();
        ctx.set_status(CoreDriverStatus::Provider {
            observed_generation: ctx.generation(),
            phase,
            observation,
            volume_refs,
        });
        // Convergence owns its own schedule: a Provider row's first pass runs
        // while the bundle ingest is still creating its controller `Process`
        // children, and the row turns Ready only once the controller session
        // evidence is live - both arrive after this pass, and neither carries
        // an edge this driver can watch. Requeue while the phase is still
        // Pending so the observation re-runs; Ready (or a degraded/failed
        // terminal) stops the schedule.
        if phase == ProviderPhase::Pending {
            let _ = ctx.requeue_after(PROVIDER_CONVERGENCE_POLL);
        }
        Ok(())
    }

    /// The Provider's dependency rows, as the old registered-API read selected
    /// them: the rows the Provider owns (`Process` controllers, state
    /// `Volume`s) plus, for the two internally hosted providers, the zone's
    /// `Host` and `Zone` rows.
    async fn dependencies(
        &self,
        ctx: &mut ResourceContext,
        provider_ref: &ResourceRef,
        provider_uid: &ResourceUid,
        provider_generation: ResourceGeneration,
    ) -> Result<Vec<DependencySnapshot>, CoreDriverError> {
        let provider_ref_text = provider_ref.to_canonical_string();
        let mut keys: Vec<ResourceKey> = Vec::new();
        let children = ctx.children().await.map_err(|_| {
            self.error(CoreDriverErrorKind::DependencyRead, DriverOp::Reconcile)
        })?;
        keys.extend(children.into_iter().map(|row| row.key));
        if provider_ref_text == SYSTEM_CORE_PROVIDER_REF
            || provider_ref_text == SYSTEM_MINIJAIL_PROVIDER_REF
        {
            let zone = ctx.key().zone.clone();
            let (host_type, host_name) = SYSTEM_CORE_HOST_REF
                .split_once('/')
                .expect("the canonical Host reference is a contract reference");
            keys.push(ResourceKey::new(zone.as_str(), host_type, host_name));
            keys.push(ResourceKey::new(zone.as_str(), "Zone", zone.as_str()));
        }
        let mut dependencies = Vec::new();
        for key in keys {
            let view = ctx.get_view(&key).await.map_err(|_| {
                self.error(CoreDriverErrorKind::DependencyRead, DriverOp::Reconcile)
            })?;
            let Some(view) = view else {
                continue;
            };
            if let Some(snapshot) = self.dependency_snapshot(
                &view,
                provider_ref_text.as_str(),
                provider_uid,
                provider_generation,
            ) {
                dependencies.push(snapshot);
            }
        }
        Ok(dependencies)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, WatchId,
        WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{DriverOp, FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::resource::ResourceStatus;
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;

    use super::{
        CORE_RESOURCE_TYPES, CoreDriverStatus, CoreResourceDriver, CoreResourceDriverFactory,
        FailClosedCoreDriverEffects, core_spec_decoder,
    };

    /// The provider row uid `[0x11; …]`; its contracts uid is the same
    /// UUIDv4-shaped text the old fixtures used.
    const PROVIDER_UID_BYTES: [u8; 16] = [
        0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x41, 0x11, 0x81, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
        0x11,
    ];
    const PROVIDER_UID_TEXT: &str = "11111111-1111-4111-8111-111111111111";
    const CHILD_UID_BYTES: [u8; 16] = [
        0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x42, 0x22, 0x82, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
        0x22,
    ];

    // -- fakes ---------------------------------------------------------------

    /// Scripted session-evidence port.
    struct RecordingEffects {
        calls: parking_lot::Mutex<Vec<String>>,
        evidence: parking_lot::Mutex<Option<serde_json::Value>>,
    }

    impl RecordingEffects {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                evidence: parking_lot::Mutex::new(None),
            })
        }

        fn call_order(&self) -> Vec<String> {
            self.calls.lock().clone()
        }

        fn set_evidence(&self, evidence: serde_json::Value) {
            *self.evidence.lock() = Some(evidence);
        }
    }

    impl super::CoreDriverEffects for RecordingEffects {
        fn controller_session_evidence(
            &self,
            process_ref: &d2b_contracts_resource::v3::ResourceRef,
            process_uid: &d2b_contracts_resource::v3::ResourceUid,
            generation: d2b_contracts_resource::v3::ResourceGeneration,
        ) -> Option<serde_json::Value> {
            self.calls.lock().push("controller-session".to_owned());
            let mut evidence = self.evidence.lock().clone()?;
            evidence["processRef"] = serde_json::Value::String(process_ref.to_canonical_string());
            evidence["processUid"] = serde_json::Value::String(process_uid.as_str().to_owned());
            evidence["processGeneration"] = serde_json::Value::from(generation.get());
            Some(evidence)
        }
    }

    /// Recording manager over a scripted row set.
    struct RecordingManager {
        calls: parking_lot::Mutex<Vec<String>>,
        rows: parking_lot::Mutex<Vec<StoredDesiredResource>>,
        views: parking_lot::Mutex<Vec<(ResourceKey, ResourceView)>>,
        fail_reads: AtomicBool,
    }

    impl RecordingManager {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                rows: parking_lot::Mutex::new(Vec::new()),
                views: parking_lot::Mutex::new(Vec::new()),
                fail_reads: AtomicBool::new(false),
            })
        }

        fn call_order(&self) -> Vec<String> {
            self.calls.lock().clone()
        }

        fn add_owned(&self, row: StoredDesiredResource, view: ResourceView) {
            self.views.lock().push((view.key.clone(), view));
            self.rows.lock().push(row);
        }

        fn add_view(&self, view: ResourceView) {
            self.views.lock().push((view.key.clone(), view));
        }

        fn drop_row(&self, key: &ResourceKey) {
            self.rows.lock().retain(|row| row.key != *key);
            self.views.lock().retain(|(view_key, _)| view_key != key);
        }

        fn view_of(&self, key: &ResourceKey) -> Option<ResourceView> {
            self.views
                .lock()
                .iter()
                .find(|(view_key, _)| view_key == key)
                .map(|(_, view)| view.clone())
        }

        fn fail_reads(&self) {
            self.fail_reads.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            self.calls.lock().push("ensure-child".to_owned());
            Err(ResourceError::ManagerRpc("unexpected ensure_child".into()))
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            self.calls.lock().push("get".to_owned());
            Ok(None)
        }

        async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
            self.calls
                .lock()
                .push(format!("view:{}/{}", key.type_name, key.name));
            if self.fail_reads.load(Ordering::SeqCst) {
                return Err(ResourceError::ManagerRpc("scripted read failure".into()));
            }
            Ok(self.view_of(key))
        }

        async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
            self.calls.lock().push("delete".to_owned());
            Err(ResourceError::ManagerRpc("unexpected delete".into()))
        }

        async fn list_owned(
            &self,
            owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            self.calls.lock().push("list-owned".to_owned());
            if self.fail_reads.load(Ordering::SeqCst) {
                return Err(ResourceError::ManagerRpc("scripted read failure".into()));
            }
            Ok(self
                .rows
                .lock()
                .iter()
                .filter(|row| row.owner_uid == Some(owner_uid))
                .cloned()
                .collect())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            self.calls.lock().push("register-watch".to_owned());
            Ok(WatchId(1))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            self.calls.lock().push("cancel-watch".to_owned());
            Ok(())
        }
    }

    struct RecordingRequeue;

    impl RequeueScheduler for RecordingRequeue {
        fn schedule(&self, _key: ResourceKey, _after: std::time::Duration) -> RequeueId {
            RequeueId(0)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    // -- fixtures ------------------------------------------------------------

    fn spec_bytes(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).expect("spec bytes")
    }

    fn metadata_bytes() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "generation": 1 })).expect("metadata")
    }

    fn row(type_name: &str, name: &str, spec: serde_json::Value) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", type_name, name),
            uid: PROVIDER_UID_BYTES,
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting: false,
            spec: spec_bytes(spec),
            metadata: metadata_bytes(),
            created_at: 0,
        }
    }

    fn provider_row(name: &str, spec: serde_json::Value) -> StoredDesiredResource {
        row("Provider", name, spec)
    }

    fn context(target: StoredDesiredResource, manager: Arc<RecordingManager>) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            target,
            TargetHandle::Host,
            core_spec_decoder(),
            manager,
            Arc::new(RecordingRequeue),
            effects_tx,
            notify_tx,
        )
    }

    fn view(
        type_name: &str,
        name: &str,
        spec: serde_json::Value,
        status: Option<ResourceStatus>,
        generation: u64,
    ) -> ResourceView {
        ResourceView {
            key: ResourceKey::new("work", type_name, name),
            uid: CHILD_UID_BYTES,
            generation,
            deleting: false,
            provenance: ResourceProvenance::Resource,
            spec: spec_bytes(spec),
            metadata: metadata_bytes(),
            owner_key: Some(ResourceKey::new("work", "Provider", "runtime")),
            status,
            status_generation: Some(generation),
            status_projection: None,
        }
    }

    fn owned_row(
        type_name: &str,
        name: &str,
        spec: serde_json::Value,
        generation: u64,
    ) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", type_name, name),
            uid: CHILD_UID_BYTES,
            generation,
            owner_uid: Some(PROVIDER_UID_BYTES),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: spec_bytes(spec),
            metadata: metadata_bytes(),
            created_at: 0,
        }
    }

    fn controller_process() -> (StoredDesiredResource, ResourceView) {
        let spec = serde_json::json!({
            "processClass": "controller",
            "providerRef": "Provider/system-minijail",
        });
        (
            owned_row("Process", "runtime-controller", spec.clone(), 3),
            view("Process", "runtime-controller", spec, Some(ResourceStatus::Ready), 3),
        )
    }

    fn state_volume() -> (StoredDesiredResource, ResourceView) {
        let spec = serde_json::json!({ "state": true });
        (
            owned_row("Volume", "runtime-state", spec.clone(), 1),
            view("Volume", "runtime-state", spec, Some(ResourceStatus::Ready), 1),
        )
    }

    fn ready_session() -> serde_json::Value {
        serde_json::json!({
            "ready": true,
            "providerRef": "Provider/runtime",
            "providerUid": PROVIDER_UID_TEXT,
            "providerGeneration": 1,
            "controllerGeneration": 7,
            "sessionGeneration": 9,
            "artifactReady": true,
            "descriptorReady": true,
            "registrationReady": true,
        })
    }

    async fn build(
        resource_type: &str,
        name: &str,
        effects: Arc<RecordingEffects>,
    ) -> Box<dyn DynResourceDriver> {
        CoreResourceDriverFactory::with_effects(effects)
            .create(&ResourceKey::new("work", resource_type, name))
            .await
    }

    /// A `Provider/runtime` fixture with one ready controller Process and one
    /// ready state Volume.
    async fn provider_fixture() -> (
        ResourceContext,
        Arc<RecordingEffects>,
        Arc<RecordingManager>,
        Box<dyn DynResourceDriver>,
    ) {
        let effects = RecordingEffects::new();
        let manager = RecordingManager::new();
        let (process_row, process_view) = controller_process();
        let (volume_row, volume_view) = state_volume();
        manager.add_owned(process_row, process_view);
        manager.add_owned(volume_row, volume_view);
        let ctx = context(
            provider_row(
                "runtime",
                serde_json::json!({ "artifactId": "runtime", "config": {} }),
            ),
            Arc::clone(&manager),
        );
        let driver = build("Provider", "runtime", Arc::clone(&effects)).await;
        (ctx, effects, manager, driver)
    }

    fn provider_status(ctx: &ResourceContext) -> CoreDriverStatus {
        ctx.status::<CoreDriverStatus>()
            .cloned()
            .expect("status published")
    }

    // -- factory -------------------------------------------------------------

    #[tokio::test]
    async fn factory_registers_exactly_the_nine_core_types() {
        let factory = CoreResourceDriverFactory::new();
        assert_eq!(factory.resource_types().len(), 9);
        assert_eq!(
            factory
                .resource_types()
                .iter()
                .map(d2b_resource_runtime::identity::ResourceTypeName::as_str)
                .collect::<Vec<_>>(),
            CORE_RESOURCE_TYPES
        );
        for resource_type in CORE_RESOURCE_TYPES {
            factory
                .create(&ResourceKey::new("work", resource_type, "sample"))
                .await;
        }
    }

    // -- validate ------------------------------------------------------------

    #[tokio::test]
    async fn validate_refuses_a_spec_that_is_not_an_object() {
        let manager = RecordingManager::new();
        let mut ctx = context(
            row("Provider", "runtime", serde_json::json!("not-an-object")),
            Arc::clone(&manager),
        );
        let mut driver = build("Provider", "runtime", RecordingEffects::new()).await;
        let failure = driver.validate(&mut ctx).await.expect_err("terminal");
        assert_eq!(
            failure,
            d2b_resource_runtime::error::DriverFailure::refused(
                DriverOp::Validate,
                d2b_resource_runtime::error::FailureKinds::CORE_SPEC_INVALID,
            )
            .with_detail(
                d2b_resource_runtime::error::FailureDetail::at("spec/shape").comparison(
                    d2b_resource_runtime::error::FailureComparison::new(
                        "spec.shape",
                        "object",
                        "string",
                    ),
                ),
            )
        );
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Validate);
        assert!(
            manager.call_order().is_empty(),
            "validate must not touch the manager"
        );
    }

    #[tokio::test]
    async fn validate_admits_the_empty_spec_object() {
        // The bootstrap Zone row stores `{}` as its spec; the old fence was
        // "the row is not empty", not "the spec has fields".
        let manager = RecordingManager::new();
        let mut ctx = context(row("Zone", "work", serde_json::json!({})), Arc::clone(&manager));
        let mut driver = build("Zone", "work", RecordingEffects::new()).await;
        driver
            .validate(&mut ctx)
            .await
            .expect("the empty spec object is admitted");
    }

    // -- recover / delete ----------------------------------------------------

    #[tokio::test]
    async fn recover_adopts_without_effects() {
        let manager = RecordingManager::new();
        let mut ctx = context(
            provider_row("runtime", serde_json::json!({ "artifactId": "runtime" })),
            Arc::clone(&manager),
        );
        let mut driver = build("Provider", "runtime", RecordingEffects::new()).await;
        let outcome = driver.recover(&mut ctx).await.expect("adopted");
        assert_eq!(outcome, RecoveryOutcome::Adopted);
        assert!(
            manager.call_order().is_empty(),
            "recovery realizes nothing on a target and owns no child rows"
        );
    }

    #[tokio::test]
    async fn delete_converges_without_effects() {
        let manager = RecordingManager::new();
        let mut ctx = context(
            row("Quota", "work-quota", serde_json::json!({})),
            Arc::clone(&manager),
        );
        let mut driver = build("Quota", "work-quota", RecordingEffects::new()).await;
        driver.delete(&mut ctx).await.expect("converged");
        assert!(manager.call_order().is_empty());
    }

    // -- metadata-only types -------------------------------------------------

    #[tokio::test]
    async fn metadata_only_types_converge_without_effects() {
        for resource_type in [
            "Zone",
            "ZoneLink",
            "Role",
            "RoleBinding",
            "Quota",
            "EmergencyPolicy",
            "ResourceExport",
            "ResourceImport",
        ] {
            let manager = RecordingManager::new();
            let mut ctx = context(
                row(resource_type, "sample", serde_json::json!({})),
                Arc::clone(&manager),
            );
            let mut driver = build(resource_type, "sample", RecordingEffects::new()).await;
            let outcome = driver.reconcile(&mut ctx).await.expect("converged");
            assert_eq!(
                outcome,
                ReconcileOutcome::Satisfied,
                "{resource_type} must converge as metadata-only"
            );
            assert!(
                manager.call_order().is_empty(),
                "{resource_type} must not read or mutate rows"
            );
            assert!(ctx.status::<CoreDriverStatus>().is_none());
        }
    }

    // -- provider observation ------------------------------------------------

    #[tokio::test]
    async fn provider_reconcile_publishes_the_observed_status() {
        let (mut ctx, effects, manager, mut driver) = provider_fixture().await;
        effects.set_evidence(ready_session());
        let outcome = driver.reconcile(&mut ctx).await.expect("observed");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let status = provider_status(&ctx);
        assert_eq!(status.observed_generation(), 1);
        assert_eq!(status.phase(), d2b_core_controller::providers::ProviderPhase::Ready);
        let observation = status.observation();
        let volume_refs = status.volume_refs();
        assert!(observation.package_present && observation.config_valid);
        assert!(observation.graph_valid && observation.conformance_valid);
        assert!(observation.required_dependencies_ready);
        assert!(observation.required_components_ready);
        assert!(!observation.optional_components_degraded);
        assert_eq!(
            volume_refs.iter().map(String::as_str).collect::<Vec<_>>(),
            ["Volume/runtime-state"]
        );
        let calls = manager.call_order();
        assert!(calls.iter().any(|call| call == "list-owned"));
        assert!(
            calls
                .iter()
                .any(|call| call == "view:Process/runtime-controller")
        );
        assert_eq!(
            effects.call_order(),
            ["controller-session"],
            "session evidence is read exactly once per controller child"
        );
    }

    #[tokio::test]
    async fn provider_reconcile_pends_without_session_evidence() {
        let (mut ctx, _effects, _manager, mut driver) = provider_fixture().await;
        let outcome = driver.reconcile(&mut ctx).await.expect("observed");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let status = provider_status(&ctx);
        assert_eq!(
            status.phase(),
            d2b_core_controller::providers::ProviderPhase::Pending,
            "a controller child without live session evidence is never Ready"
        );
        let observation = status.observation();
        assert!(
            !observation.required_components_ready,
            "a controller component is not ready without live session evidence"
        );
        assert!(
            observation.required_dependencies_ready,
            "the Process row itself is Ready; only the session gate fails"
        );
        assert!(
            !observation.conformance_valid,
            "the absent session is exactly the conformance failure the old handler read"
        );
    }

    #[tokio::test]
    async fn provider_reconcile_fails_a_declared_volume_that_disappeared() {
        let (mut ctx, effects, manager, mut driver) = provider_fixture().await;
        effects.set_evidence(ready_session());
        driver.reconcile(&mut ctx).await.expect("first pass");
        assert_eq!(
            provider_status(&ctx).phase(),
            d2b_core_controller::providers::ProviderPhase::Ready
        );
        // The Volume row disappears (drift): the carried expectation must keep
        // the provider out of Ready, exactly as the old
        // `expected_provider_volume_refs` comparison did.
        manager.drop_row(&ResourceKey::new("work", "Volume", "runtime-state"));
        driver.reconcile(&mut ctx).await.expect("second pass");
        let status = provider_status(&ctx);
        assert_eq!(
            status.phase(),
            d2b_core_controller::providers::ProviderPhase::Pending,
            "a declared state Volume that disappeared must fail the provider"
        );
        let observation = status.observation();
        assert!(!observation.required_dependencies_ready);
    }

    #[tokio::test]
    async fn provider_reconcile_reports_manager_failures_as_retryable() {
        let (mut ctx, _effects, manager, mut driver) = provider_fixture().await;
        manager.fail_reads();
        let failure = driver.reconcile(&mut ctx).await.expect_err("read failure");
        assert_eq!(
            failure.class(),
            FailureClass::Retryable,
            "the old dependency read was a retried source read"
        );
        assert_eq!(failure.op(), DriverOp::Reconcile);
    }

    #[tokio::test]
    async fn system_core_provider_phase_follows_the_zone_projection() {
        // The system-core exception: its readiness is the Zone row's
        // mandatory-handler projection. The durable Zone row never carried
        // one, so the predicate reads false and the provider stays Pending -
        // no children involved either way.
        let effects = RecordingEffects::new();
        let manager = RecordingManager::new();
        manager.add_view(view(
            "Zone",
            "work",
            serde_json::json!({ "providerRef": "Provider/system-core" }),
            Some(ResourceStatus::Ready),
            1,
        ));
        let mut ctx = context(
            provider_row(
                "system-core",
                serde_json::json!({ "artifactId": "system-core", "config": {} }),
            ),
            Arc::clone(&manager),
        );
        let mut driver = build("Provider", "system-core", Arc::clone(&effects)).await;
        driver.reconcile(&mut ctx).await.expect("observed");
        assert_eq!(
            provider_status(&ctx).phase(),
            d2b_core_controller::providers::ProviderPhase::Pending
        );
        assert!(
            manager
                .call_order()
                .iter()
                .any(|call| call == "view:Zone/work"),
            "the fixed dependency set is read from the manager"
        );
    }

    // -- finalize (drain) ----------------------------------------------------

    #[tokio::test]
    async fn finalize_converges_for_a_type_without_drain() {
        let manager = RecordingManager::new();
        let mut ctx = context(
            row("RoleBinding", "work-read", serde_json::json!({})),
            Arc::clone(&manager),
        );
        let mut driver = build("RoleBinding", "work-read", RecordingEffects::new()).await;
        driver.finalize(&mut ctx).await.expect("nothing to drain");
        assert!(
            manager
                .call_order()
                .iter()
                .all(|call| call == "list-owned"),
            "the child pass reads ownership and releases nothing; a row without a \
             finalizer has no other drain step. The erased boundary also runs the \
             child pass as a backstop, so the read repeats - it is idempotent."
        );
    }

    #[tokio::test]
    async fn finalize_blocks_while_an_owned_child_is_live() {
        // The child pass runs for every type, not only the Provider: a Quota
        // with an owned child requeues until the child's own finalize/delete
        // retires it.
        let manager = RecordingManager::new();
        let (process_row, process_view) = controller_process();
        manager.add_owned(process_row, process_view);
        let mut ctx = context(
            row("Quota", "work-quota", serde_json::json!({})),
            Arc::clone(&manager),
        );
        let mut driver = build("Quota", "work-quota", RecordingEffects::new()).await;
        let failure = driver.finalize(&mut ctx).await.expect_err("child draining");
        assert_eq!(
            failure,
            d2b_resource_runtime::error::DriverFailure::not_yet(
                DriverOp::Delete,
                d2b_resource_runtime::error::FailureKinds::CHILDREN_DRAINING,
            ),
            "the actor requeues another delete pass while an owned child is live"
        );
        assert!(
            manager
                .call_order()
                .iter()
                .any(|call| call == "delete"),
            "the child is nudged through its own finalize-before-delete pass"
        );
    }

    #[tokio::test]
    async fn finalize_converges_once_the_owned_children_are_gone() {
        let (mut ctx, _effects, manager, mut driver) = provider_fixture().await;
        driver.finalize(&mut ctx).await.expect_err("children live");
        manager.drop_row(&ResourceKey::new("work", "Process", "runtime-controller"));
        manager.drop_row(&ResourceKey::new("work", "Volume", "runtime-state"));
        driver
            .finalize(&mut ctx)
            .await
            .expect("no owned child and no provider drain gate remains");
    }

    #[tokio::test]
    async fn provider_drain_gate_tracks_the_controller_process_child() {
        // The per-type drain behind the child pass, exercised directly on the
        // concrete driver: a controller Process row blocks, a Volume child
        // does not.
        let (mut ctx, _effects, manager, _driver) = provider_fixture().await;
        let driver = CoreResourceDriver {
            effects: Arc::new(FailClosedCoreDriverEffects),
        };
        driver
            .finalize_pass(&mut ctx)
            .await
            .expect_err("a controller Process child is the provider's drain gate");
        manager.drop_row(&ResourceKey::new("work", "Process", "runtime-controller"));
        driver
            .finalize_pass(&mut ctx)
            .await
            .expect("worker and state children are not the provider's drain gate");
    }

    // -- error classification ------------------------------------------------

    #[test]
    fn error_classes_keep_the_old_terminal_fence_and_a_retryable_read() {
        let terminal = super::CoreDriverError::new(
            super::CoreDriverErrorKind::SpecInvalid,
            DriverOp::Validate,
        );
        let retryable = super::CoreDriverError::new(
            super::CoreDriverErrorKind::DependencyRead,
            DriverOp::Reconcile,
        );
        assert_eq!(terminal.class(), FailureClass::Terminal);
        assert_eq!(retryable.class(), FailureClass::Retryable);
        assert_eq!(terminal.op(), DriverOp::Validate);
        assert_eq!(retryable.op(), DriverOp::Reconcile);
    }
}
