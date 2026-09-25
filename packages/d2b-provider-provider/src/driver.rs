//! The Provider resource driver: the v3 `ResourceDriver` conversion of the
//! Core provider handler, and the one core type with behavior beyond
//! convergence.
//!
//! `Provider` is one of the nine ResourceTypes the fixed Core process hosted
//! (the old `CORE_RESOURCE_CONTROLLER_REGISTRATIONS`). It realizes nothing on
//! a target and owns no declared child set: the old handler was Core's
//! baseline reconciler plus the readiness observation Core applies to the
//! Provider's owned controller `Process` rows and state `Volume` rows. The
//! pure policy the observation feeds lives in [`crate::providers`].
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`ProviderDriverFactory`] registration through the type's
//!   descriptor ([`provider_descriptor`]).
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
//!   manager's owned rows; the observation and phase policy stay the shared
//!   pure core ([`crate::providers::provider_observation`],
//!   [`ProviderHandler::plan_observed`], [`ProviderHandler::plan_system_core`],
//!   [`ProviderHandler::fixed_system_core_handlers_ready`]).
//! - `prepare_finalize`/`execute_finalize`/`finalize` -> [`ResourceDriver::finalize`]
//!   (the drain step before delete): owned children are drained first, and the
//!   Provider has a second gate - its controller `Process` children must
//!   retire before its own row may.
//! - `finalize` -> [`ResourceDriver::delete`]: the manager committed the
//!   durable deleting mark (R10) and the type realizes no target-local state.
//! - `health`/`drain` -> actor supervision and shutdown, not driver surface.
//! - `assess_update`/`plan_upgrade`/`execute_upgrade` -> no KTD3 equivalent:
//!   the old handler assessed every row `Current` with preserve-state and
//!   planned a no-op restart; the runner's upgrade path is gone with the
//!   runner (R30).
//! - `UpdateStatus` -> [`ResourceContext::set_status`] (in-memory only, R11).
//! - `DependencySnapshot` -> the manager's owned children plus, for the two
//!   internally hosted providers, the fixed `Host`/`Zone` dependency rows.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
};
use d2b_controller_toolkit::{
    DependencySnapshot, ResourceKey as CoreResourceKey, ResourceSnapshot,
};
use d2b_resource_runtime::context::{ResourceContext, SpecDecoder, typed_spec_decoder};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKinds,
    ResourceError,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName, StoredDesiredResource};
use d2b_resource_runtime::manager::ResourceView;
use d2b_resource_runtime::resource::ResourceStatus;
use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};
use serde_json::{Value, json};

use crate::providers::{
    ProviderHandler, ProviderIntent, ProviderObservation, ProviderPhase,
    fixed_system_core_handlers_ready, provider_observation,
};

/// The one resource type this driver serves.
pub const PROVIDER_TYPE_NAME: &str = "Provider";

/// The canonical `Provider/system-core` reference, exported by the provider
/// crate that realizes it (the fixed dependency set reads it instead of the
/// provider's children).
pub const SYSTEM_CORE_PROVIDER_REF: &str = d2b_provider_system_core::PROVIDER_REF;

/// The canonical `Provider/system-minijail` reference, exported by the
/// provider crate that realizes it.
pub const SYSTEM_MINIJAIL_PROVIDER_REF: &str = d2b_provider_process_minijail::PROVIDER_REF;

/// The canonical Host row the fixed providers observe.
pub const SYSTEM_CORE_HOST_REF: &str = "Host/host-system";

/// Convergence poll for a Provider row that is not yet Ready: its controller
/// `Process` children and their live session evidence both arrive after the
/// row's first pass, and neither carries an edge this driver can watch, so
/// the observation re-runs on this schedule until the phase leaves Pending.
const PROVIDER_CONVERGENCE_POLL: Duration = Duration::from_millis(1_000);

/// The execution domains the `Provider` type can be reconciled in.
///
/// Derived from the placement contract: `Provider` names no placement anchor
/// (`PlacementAnchor::canonical_for` resolves none), so a Provider row never
/// carries the canonical `spec.executionRef` and the plane reconciles it on
/// its own Host domain.
const PROVIDER_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the `Provider` observation reads while reconciling.
///
/// Derived from the driver's row reads: the Provider's owned controller
/// `Process` rows and state `Volume` rows are the ones it observes, and the
/// two internally hosted providers read the fixed `Host` and `Zone`
/// dependency rows instead of their children.
const PROVIDER_READS: &[WellKnownType] = &[
    WellKnownType::HOST,
    WellKnownType::PROCESS,
    WellKnownType::VOLUME,
    WellKnownType::ZONE,
];

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
pub struct ProviderDriverStatus {
    /// The desired generation this observation was taken at (the old
    /// `status.observedGeneration`).
    pub observed_generation: u64,
    /// The phase the pure policy projected.
    pub phase: ProviderPhase,
    /// The observation the pass computed.
    pub observation: ProviderObservation,
    /// The `Volume` references among the Provider's owned rows at the last
    /// published observation.
    pub volume_refs: BTreeSet<String>,
}

// ---------------------------------------------------------------------------
// Effects port
// ---------------------------------------------------------------------------

/// The live evidence the `Provider` observation needs and the manager cannot
/// serve: a converted row's status is in-memory only (R11), so the manager's
/// view carries the closed [`ResourceStatus`] and never the controller-session
/// evidence the pure core reads as `status.resource.controllerSession`. The
/// production implementation is the daemon's live-session seam; every unknown
/// fails closed (`None`), so no caller can synthesize an admitted session.
pub trait ProviderDriverEffects: Send + Sync + 'static {
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
pub struct FailClosedProviderDriverEffects;

impl ProviderDriverEffects for FailClosedProviderDriverEffects {
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

/// The manager-wired decode hook for `Provider` rows: the stored spec envelope
/// is the JSON spec object every core row stores (the core types have no typed
/// core spec, and the old handler worked from canonical JSON).
pub fn provider_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| serde_json::from_slice::<Value>(bytes))
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Everything the composition must construct to instantiate the Provider
/// driver factory: the live session-evidence port.
pub struct ProviderDriverArgs {
    /// The live controller-session evidence the observation reads.
    pub effects: Arc<dyn ProviderDriverEffects>,
}

/// [`ResourceDriverFactory`] for the `Provider` resource type. Construction is
/// infallible by contract (R3): the production effects carry no fallible
/// setup.
pub struct ProviderDriverFactory {
    types: [ResourceTypeName; 1],
    effects: Arc<dyn ProviderDriverEffects>,
}

impl ProviderDriverFactory {
    /// Construct over an injected port. The plane composition wires the live
    /// controller-session seam here.
    pub fn with_effects(effects: Arc<dyn ProviderDriverEffects>) -> Self {
        Self {
            types: [ResourceTypeName::new(PROVIDER_TYPE_NAME)],
            effects,
        }
    }
}

#[async_trait]
impl ResourceDriverFactory for ProviderDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(ProviderDriver {
            effects: Arc::clone(&self.effects),
        })
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One `Provider` resource's driver.
pub struct ProviderDriver {
    effects: Arc<dyn ProviderDriverEffects>,
}

#[async_trait]
impl ResourceDriver for ProviderDriver {
    type Error = DriverFailure;

    fn classify_error(&self, error: &DriverFailure) -> DriverFailure {
        error.clone()
    }

    /// The stored spec object fence (old `validate_spec`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        spec_object(ctx, DriverOp::Validate)?;
        Ok(())
    }

    /// The old `observe` was converged with nothing to adopt: the family
    /// realizes no target-local state.
    async fn recover(
        &mut self,
        _ctx: &mut ResourceContext,
    ) -> Result<RecoveryOutcome, Self::Error> {
        Ok(RecoveryOutcome::Adopted)
    }

    /// One reconcile pass: observe the owned controller `Process` and state
    /// `Volume` rows (plus the fixed `Host`/`Zone` dependency rows for the two
    /// internally hosted providers) and publish the phase the pure policy
    /// projects. The manager's durable deleting mark owns the finalizer
    /// bookkeeping the old handler converged on.
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        self.reconcile_provider(ctx).await?;
        Ok(ReconcileOutcome::Satisfied)
    }

    /// The drain step before [`ResourceDriver::delete`] (R10, F3). Owned
    /// children are drained first (their own finalize-before-delete pass), and
    /// the Provider is the one core type with a second gate: its controller
    /// `Process` children must retire before its row may, so a controller
    /// Process row still present among the owned children keeps the actor
    /// requeueing.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        drain_owned_children(ctx).await?;
        self.finalize_pass(ctx).await
    }

    /// Teardown: the manager committed the durable deleting mark (R10) and the
    /// family owns no children of its own to retire.
    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl ProviderDriver {
    /// The Provider's drain gate: only the Provider has dependents whose
    /// retirement must precede its own row's, and the manager's owned-child
    /// read is where they are visible. Idempotent under retry - it reads state
    /// and returns, releasing nothing.
    async fn finalize_pass(&self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        let children = ctx.children().await.map_err(|error| {
            DriverFailure::error(
                DriverOp::Delete,
                FailureKinds::CORE_DEPENDENCY_READ_FAILED,
                FailureClass::Retryable,
            )
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
            return Err(
                DriverFailure::not_yet(DriverOp::Delete, FailureKinds::CORE_DRAIN_PENDING)
                    .with_detail(FailureDetail::at("finalize/drain").comparison(
                        FailureComparison::new("owned.controllerProcess", "retired", "live"),
                    )),
            );
        }
        Ok(())
    }

    /// The preserved `Provider` pass: observe the owned controller `Process`
    /// and state `Volume` rows (plus the fixed `Host`/`Zone` dependency rows
    /// for the two internally hosted providers) and publish the phase the
    /// shared pure policy projects.
    async fn reconcile_provider(&self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        let spec = spec_object(ctx, DriverOp::Reconcile)?;
        let provider_ref = resource_ref(ctx)?;
        let provider_uid =
            resource_uid(ctx.uid()).ok_or_else(|| spec_invalid(DriverOp::Reconcile, "spec/uid"))?;
        let generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| spec_invalid(DriverOp::Reconcile, "spec/generation"))?;
        let zone = ZoneId::parse(ctx.key().zone.as_str())
            .map_err(|_| spec_invalid(DriverOp::Reconcile, "spec/zone"))?;

        let dependencies = self
            .dependencies(ctx, &provider_ref, &provider_uid, generation)
            .await?;

        // The old `status.observedGeneration` short-circuit selected the
        // `Enable` intent; the in-memory status is its runtime-only successor.
        let previous = ctx.status::<ProviderDriverStatus>();
        let intent = match previous {
            Some(status) if status.observed_generation == ctx.generation() => {
                ProviderIntent::Enable
            }
            _ => ProviderIntent::Update,
        };

        // The provider row as the pure observation reads it: spec from the
        // stored envelope, metadata as authored, and the previous
        // observation's owned Volume references standing in for the durable
        // `status.resource.owned.refs` projection the store used to derive.
        let metadata: Value = serde_json::from_slice(ctx.metadata())
            .map_err(|_| spec_invalid(DriverOp::Reconcile, "spec/metadata"))?;
        let status = match previous {
            Some(previous) => json!({
                "observedGeneration": previous.observed_generation,
                "resource": {
                    "owned": { "refs": previous.volume_refs.iter().collect::<Vec<_>>() },
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
        .map_err(|_| spec_invalid(DriverOp::Reconcile, "spec/encode"))?;
        let provider = ResourceSnapshot::new(
            CoreResourceKey::new(zone, provider_ref.clone(), provider_uid.clone()),
            ZoneRevision::new(ctx.generation()),
            generation,
            canonical,
            false,
        );
        let observation = provider_observation(&provider, &dependencies)
            .map_err(|_| spec_invalid(DriverOp::Reconcile, "spec/observation"))?;

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
        ctx.set_status(ProviderDriverStatus {
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
    ) -> Result<Vec<DependencySnapshot>, DriverFailure> {
        let provider_ref_text = provider_ref.to_canonical_string();
        let mut keys: Vec<ResourceKey> = Vec::new();
        let children = ctx.children().await.map_err(|_| {
            DriverFailure::error(
                DriverOp::Reconcile,
                FailureKinds::CORE_DEPENDENCY_READ_FAILED,
                FailureClass::Retryable,
            )
        })?;
        keys.extend(children.into_iter().map(|row| row.key));
        if provider_ref_text == SYSTEM_CORE_PROVIDER_REF
            || provider_ref_text == SYSTEM_MINIJAIL_PROVIDER_REF
        {
            let zone = ctx.key().zone.as_str();
            let (host_type, host_name) = SYSTEM_CORE_HOST_REF
                .split_once('/')
                .expect("the canonical Host reference is a contract reference");
            keys.push(ResourceKey::new(zone, host_type, host_name));
            keys.push(ResourceKey::new(zone, "Zone", zone));
        }
        let mut dependencies = Vec::new();
        for key in keys {
            let view = ctx.get_view(&key).await.map_err(|_| {
                DriverFailure::error(
                    DriverOp::Reconcile,
                    FailureKinds::CORE_DEPENDENCY_READ_FAILED,
                    FailureClass::Retryable,
                )
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
        let zone = ZoneId::parse(view.key.zone.as_str()).ok()?;
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
        metadata.as_object_mut()?.insert(
            "ownerRef".to_owned(),
            Value::String(provider_ref.to_owned()),
        );
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

// ---------------------------------------------------------------------------
// Shared core-type fences
// ---------------------------------------------------------------------------

/// A terminal spec refusal at one stage.
fn spec_invalid(op: DriverOp, stage: &'static str) -> DriverFailure {
    DriverFailure::refused(op, FailureKinds::CORE_SPEC_INVALID)
        .with_detail(FailureDetail::at(stage))
}

/// The stored spec object fence every core type shares: the manager's decode
/// hook must have produced a JSON object, which is exactly what the old
/// `validate_spec` required of the canonical row JSON.
fn spec_object(ctx: &ResourceContext, op: DriverOp) -> Result<Value, DriverFailure> {
    let spec = ctx.spec::<Value>().map_err(|error| {
        DriverFailure::refused(op, FailureKinds::CORE_SPEC_INVALID)
            .with_detail(FailureDetail::at("spec/decode").with_note(error.to_string()))
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
        return Err(
            DriverFailure::refused(op, FailureKinds::CORE_SPEC_INVALID).with_detail(
                FailureDetail::at("spec/shape").comparison(FailureComparison::new(
                    "spec.shape",
                    "object",
                    shape,
                )),
            ),
        );
    }
    Ok(spec.clone())
}

/// The row's own contract reference.
fn resource_ref(ctx: &ResourceContext) -> Result<ResourceRef, DriverFailure> {
    ResourceRef::parse(&format!("{}/{}", ctx.key().type_name, ctx.key().name)).map_err(|error| {
        spec_invalid(DriverOp::Reconcile, "spec/ref")
            .with_detail(FailureDetail::at("spec/ref").with_note(error.to_string()))
    })
}

/// The child-first drain: every owned child is nudged through its own
/// finalize-before-delete pass, and the row requeues while any child row is
/// still live. The erased boundary runs the same pass before the driver's own
/// drain, so this call is the ordering guarantee the driver owns rather than
/// the only one.
async fn drain_owned_children(ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
    match ctx.finalize_owned_resources().await {
        Ok(()) => Ok(()),
        Err(ResourceError::ChildrenDraining { .. }) => Err(DriverFailure::not_yet(
            DriverOp::Delete,
            FailureKinds::CORE_DRAIN_PENDING,
        )),
        Err(error) => Err(DriverFailure::error(
            DriverOp::Delete,
            FailureKinds::CORE_DEPENDENCY_READ_FAILED,
            FailureClass::Retryable,
        )
        .with_detail(
            FailureDetail::at("finalize/children")
                .comparison(FailureComparison::new(
                    "owned.children",
                    "finalize answered",
                    "read failed",
                ))
                .with_note(error.to_string()),
        )),
    }
}

/// Whether one owned row is a controller `Process`: the Provider's controller
/// component, whose retirement the Process controller owns and which must
/// complete before the Provider's own row may retire (KTD13, F3). The manager
/// lists ownership by uid (R8), so the row being present in the owned-child
/// read is the ownership proof; the spec decides the class.
fn is_controller_process(row: &StoredDesiredResource) -> bool {
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
    ResourceUid::from_bytes(bytes).ok()
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The `Provider` type's driver declaration.
///
/// The type is `BUILTIN` (no RUNTIME bit): the plane cannot serve the
/// converted core types without it, so it must be registered before the plane
/// opens. The driver serves no broker operations and creates no children
/// through this declaration.
///
/// The type is not exportable: `ResourceExport` admits only qualified
/// `*.d2bus.org.*Service` types, so a Provider row is never an export subject.
pub fn provider_descriptor(args: ProviderDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::PROVIDER,
        allowed_sources: AllowedSources::BUILTIN,
        verbs: CONVERTED_TYPE_VERBS,
        execution: PROVIDER_EXECUTION_DOMAINS,
        exportable: false,
        reads: PROVIDER_READS,
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: provider_spec_decoder(),
        factory: Arc::new(ProviderDriverFactory::new_with(args)),
    }
}

impl ProviderDriverFactory {
    /// Construct over the declaration's driver arguments.
    fn new_with(args: ProviderDriverArgs) -> Self {
        Self::with_effects(args.effects)
    }
}

// ---------------------------------------------------------------------------
// Tests: the preserved Provider observation, over scripted effects and rows.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, WatchId,
        WatchRegistration,
    };
    use d2b_resource_runtime::driver::{DynResourceDriver, ResourceDriverFactory};
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::spec_store::EnsureOutcome;

    use crate::test_support::RecordingEffects;

    use super::{
        PROVIDER_TYPE_NAME, ProviderDriverFactory, ProviderDriverStatus, provider_spec_decoder,
    };

    /// The provider row uid `[0x11; ...]`; its contracts uid is the same
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

/// Recording manager over a scripted row set.
    struct RecordingManager {
        calls: Mutex<Vec<String>>,
        rows: Mutex<Vec<StoredDesiredResource>>,
        views: Mutex<Vec<(ResourceKey, ResourceView)>>,
        fail_reads: AtomicBool,
    }

    impl RecordingManager {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                rows: Mutex::new(Vec::new()),
                views: Mutex::new(Vec::new()),
                fail_reads: AtomicBool::new(false),
            })
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn call_order(&self) -> Vec<String> {
            self.calls.lock().expect("calls").clone()
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn add_owned(&self, row: StoredDesiredResource, view: ResourceView) {
            self.views
                .lock()
                .expect("views")
                .push((view.key.clone(), view));
            self.rows.lock().expect("rows").push(row);
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn add_view(&self, view: ResourceView) {
            self.views
                .lock()
                .expect("views")
                .push((view.key.clone(), view));
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn drop_row(&self, key: &ResourceKey) {
            self.rows
                .lock()
                .expect("rows")
                .retain(|row| row.key != *key);
            self.views
                .lock()
                .expect("views")
                .retain(|(view_key, _)| view_key != key);
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn view_of(&self, key: &ResourceKey) -> Option<ResourceView> {
            self.views
                .lock()
                .expect("views")
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
            Err(ResourceError::ManagerRejected { reason: "unexpected ensure_child".into() })
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(None)
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
            self.calls
                .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                .expect("calls")
                .push(format!("view:{}/{}", key.type_name, key.name));
            if self.fail_reads.load(Ordering::SeqCst) {
                return Err(ResourceError::ManagerRejected { reason: "scripted read failure".into() });
            }
            Ok(self.view_of(key))
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.calls.lock().expect("calls").push("delete".to_owned()); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            self.rows
                .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                .expect("rows")
                .retain(|row| row.key != *key);
            self.views
                .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                .expect("views")
                .retain(|(view_key, _)| view_key != key);
            Ok(())
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        async fn list_owned(
            &self,
            owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            self.calls
                .lock()
                .expect("calls")
                .push("list-owned".to_owned());
            if self.fail_reads.load(Ordering::SeqCst) {
                return Err(ResourceError::ManagerRejected { reason: "scripted read failure".into() });
            }
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
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Ok(WatchId(1))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
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

    fn provider_row(name: &str, spec: serde_json::Value) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", PROVIDER_TYPE_NAME, name),
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

    fn context(target: StoredDesiredResource, manager: Arc<RecordingManager>) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            target,
            provider_spec_decoder(),
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
        status: Option<d2b_resource_runtime::resource::ResourceStatus>,
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
            owner_key: Some(ResourceKey::new("work", PROVIDER_TYPE_NAME, "runtime")),
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
            view(
                "Process",
                "runtime-controller",
                spec,
                Some(d2b_resource_runtime::resource::ResourceStatus::Ready),
                3,
            ),
        )
    }

    fn state_volume() -> (StoredDesiredResource, ResourceView) {
        let spec = serde_json::json!({ "state": true });
        (
            owned_row("Volume", "runtime-state", spec.clone(), 1),
            view(
                "Volume",
                "runtime-state",
                spec,
                Some(d2b_resource_runtime::resource::ResourceStatus::Ready),
                1,
            ),
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

    async fn build(effects: Arc<RecordingEffects>) -> Box<dyn DynResourceDriver> {
        ProviderDriverFactory::with_effects(effects)
            .create(&ResourceKey::new("work", PROVIDER_TYPE_NAME, "runtime"))
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
        let driver = build(Arc::clone(&effects)).await;
        (ctx, effects, manager, driver)
    }

    fn provider_status(ctx: &ResourceContext) -> ProviderDriverStatus {
        ctx.status::<ProviderDriverStatus>()
            .cloned()
            .expect("status published")
    }

    // -- validate / recover / delete -----------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_a_spec_that_is_not_an_object() {
        let manager = RecordingManager::new();
        let mut ctx = context(
            provider_row("runtime", serde_json::json!("not-an-object")),
            Arc::clone(&manager),
        );
        let mut driver = build(RecordingEffects::new()).await;
        let failure = driver.validate(&mut ctx).await.expect_err("terminal");
        assert_eq!(failure.kind().code(), "core-spec-invalid");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(
            failure.op(),
            d2b_resource_runtime::error::DriverOp::Validate
        );
        assert!(
            manager.call_order().is_empty(),
            "validate must not touch the manager"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recover_adopts_without_effects() {
        let manager = RecordingManager::new();
        let mut ctx = context(
            provider_row("runtime", serde_json::json!({ "artifactId": "runtime" })),
            Arc::clone(&manager),
        );
        let mut driver = build(RecordingEffects::new()).await;
        let outcome = driver
            .recover(&mut ctx)
            .await
            .expect("recovery realizes nothing");
        assert_eq!(
            outcome,
            d2b_resource_runtime::driver::RecoveryOutcome::Adopted
        );
        assert!(
            manager.call_order().is_empty(),
            "recovery realizes nothing on a target and owns no child rows"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn delete_converges_without_effects() {
        let manager = RecordingManager::new();
        let mut ctx = context(
            provider_row("runtime", serde_json::json!({})),
            Arc::clone(&manager),
        );
        let mut driver = build(RecordingEffects::new()).await;
        driver.delete(&mut ctx).await.expect("converged");
        assert!(manager.call_order().is_empty());
    }

    // -- provider observation ------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn provider_reconcile_publishes_the_observed_status() {
        let (mut ctx, effects, manager, mut driver) = provider_fixture().await;
        effects.set_evidence(ready_session());
        let outcome = driver.reconcile(&mut ctx).await.expect("observed");
        assert_eq!(
            outcome,
            d2b_resource_runtime::driver::ReconcileOutcome::Satisfied
        );
        let status = provider_status(&ctx);
        assert_eq!(status.observed_generation, 1);
        assert_eq!(status.phase, crate::providers::ProviderPhase::Ready);
        let observation = status.observation;
        let volume_refs = status.volume_refs;
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn provider_reconcile_pends_without_session_evidence() {
        let (mut ctx, _effects, _manager, mut driver) = provider_fixture().await;
        let outcome = driver.reconcile(&mut ctx).await.expect("observed");
        assert_eq!(
            outcome,
            d2b_resource_runtime::driver::ReconcileOutcome::Satisfied
        );
        let status = provider_status(&ctx);
        assert_eq!(
            status.phase,
            crate::providers::ProviderPhase::Pending,
            "a controller child without live session evidence is never Ready"
        );
        let observation = status.observation;
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn provider_reconcile_fails_a_declared_volume_that_disappeared() {
        let (mut ctx, effects, manager, mut driver) = provider_fixture().await;
        effects.set_evidence(ready_session());
        driver.reconcile(&mut ctx).await.expect("first pass");
        assert_eq!(
            provider_status(&ctx).phase,
            crate::providers::ProviderPhase::Ready
        );
        // The Volume row disappears (drift): the carried expectation must keep
        // the provider out of Ready, exactly as the old
        // `expected_provider_volume_refs` comparison did.
        manager.drop_row(&ResourceKey::new("work", "Volume", "runtime-state"));
        driver.reconcile(&mut ctx).await.expect("second pass");
        let status = provider_status(&ctx);
        assert_eq!(
            status.phase,
            crate::providers::ProviderPhase::Pending,
            "a declared state Volume that disappeared must fail the provider"
        );
        assert!(!status.observation.required_dependencies_ready);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
        assert_eq!(
            failure.op(),
            d2b_resource_runtime::error::DriverOp::Reconcile
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
            Some(d2b_resource_runtime::resource::ResourceStatus::Ready),
            1,
        ));
        let mut ctx = context(
            provider_row(
                "system-core",
                serde_json::json!({ "artifactId": "system-core", "config": {} }),
            ),
            Arc::clone(&manager),
        );
        let mut driver = build(Arc::clone(&effects)).await;
        driver.reconcile(&mut ctx).await.expect("observed");
        assert_eq!(
            provider_status(&ctx).phase,
            crate::providers::ProviderPhase::Pending
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn provider_drain_gate_tracks_the_controller_process_child() {
        // The per-type drain behind the child pass, exercised directly on the
        // concrete driver: a controller Process row blocks, a Volume child
        // does not.
        let (mut ctx, _effects, manager, _driver) = provider_fixture().await;
        let driver = super::ProviderDriver {
            effects: Arc::new(super::FailClosedProviderDriverEffects),
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
}
