//! Telemetry Binding reconciliation on the v3 resource runtime (U12
//! conversion; KTD3, KTD4, KTD13).
//!
//! The Provider controller stays the authority for the Binding's semantics:
//! the telemetry Provider declares UID-free child intents
//! (`TelemetryBindingController::child_resources`) and Core materializes them.
//! This module is the KTD3 conversion of the old `TelemetryResourceReconciler`
//! Binding half plus the `telemetry_controller_descriptor` runner lane (spec
//! section 13 mapping):
//!
//! - `describe` -> [`TelemetryBindingDriverFactory`], registered for
//!   `telemetry.d2bus.org.TelemetryBinding` in the plane's provider directory.
//! - `validate_spec` -> [`ResourceDriver::validate`]: the stored spec envelope
//!   must decode. A malformed Service/target relationship is fenced in
//!   reconcile (old `telemetry_binding_owner`), never fatal here.
//! - `observe` -> [`ResourceDriver::recover`]: adoption of the owned child set
//!   from the durable rows (R15).
//! - `plan` + `reconcile` + `execute_effect` -> [`ResourceDriver::reconcile`]:
//!   the collector/forwarder Process and Endpoint children are ensured through
//!   the manager (commit before spawn, F1), owned children the desired set no
//!   longer derives are retired in the preserved endpoint-first / process-last
//!   order, and the provider status projection moves into the driver's
//!   in-memory slot ([`ResourceContext::set_status`], R11: zero persistent
//!   writes).
//! - `prepare_finalize` + `execute_finalize` + `finalize` ->
//!   [`ResourceDriver::delete`]. The old
//!   `d2b.d2bus.org/binding-children` finalizer is gone by construction: the
//!   v3 manager already holds a parent row until its owned children retire
//!   (`ResourceManagerState::remove_internal` cascades the removal to owned
//!   children and `pending_retirement` keeps the parent's durable deleting
//!   mark observable), which is the guarantee that finalizer existed for
//!   (F3, R9-R10).

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_provider::v3::semantic_services::{
    child_resources::{BindingChildIntent, BindingChildSet},
    telemetry::TELEMETRY_BINDING_RESOURCE_TYPE,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, ResourceName, ResourceRef, ResourceSpec,
    ResourceTypeName as ContractResourceTypeName, ZoneId,
};
use d2b_provider_observability_otel::TelemetryBindingController;
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::spec_store::{EnsureOutcome, StoredDesiredResource};
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, ChildCreation, ChildCustody, DriverDescriptor,
    WellKnownType,
};

/// The qualified semantic telemetry Binding type this factory serves.
pub const TELEMETRY_BINDING_TYPE: &str = TELEMETRY_BINDING_RESOURCE_TYPE;

/// The serving Provider this driver owns (old `TELEMETRY_BINDING.2`).
pub const TELEMETRY_PROVIDER_REF: &str = "Provider/observability-otel";

/// The Process Provider the telemetry Provider declares for its collector and
/// forwarder children.
pub const TELEMETRY_BINDING_PROCESS_PROVIDER: &str = "Provider/system-minijail";

/// Preserved resync period (old `ResyncPolicy::new(None, 5_000)`). The actor
/// owns scheduling now (R13), so the driver re-schedules itself while the
/// Binding is not converged instead of polling from a runner.
pub const TELEMETRY_BINDING_RESYNC: Duration = Duration::from_secs(5);

/// Closed lifecycle phase for one telemetry Binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryBindingPhase {
    /// The Binding's child set is not current.
    Pending,
    /// The Binding's route or children are not ready.
    Degraded,
}

impl TelemetryBindingPhase {
    /// The provider phase spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Degraded => "Degraded",
        }
    }
}

/// The collector/forwarder creation the Binding declares.
///
/// The telemetry Provider's child requests place both worker Processes under
/// the fixed `Provider/system-minijail` Provider, host-placed, so the
/// declaration pins exactly that pair.
pub const TELEMETRY_BINDING_COLLECTOR_CREATION: ChildCreation = ChildCreation {
    child: WellKnownType::PROCESS,
    provider_ref: TELEMETRY_BINDING_PROCESS_PROVIDER,
    custody: ChildCustody::DriverOwned,
    order: 1,
};

/// The ingest/forwarder Endpoint creation the Binding declares.
///
/// Each Endpoint is produced by one of the declared worker Processes, so it is
/// created after its producer and retires before it.
pub const TELEMETRY_BINDING_ENDPOINT_CREATION: ChildCreation = ChildCreation {
    child: WellKnownType::ENDPOINT,
    provider_ref: TELEMETRY_PROVIDER_REF,
    custody: ChildCustody::DriverOwned,
    order: 2,
};

/// Every child creation the TelemetryBinding driver declares.
pub const TELEMETRY_BINDING_CREATIONS: &[ChildCreation] = &[
    TELEMETRY_BINDING_COLLECTOR_CREATION,
    TELEMETRY_BINDING_ENDPOINT_CREATION,
];

// ---------------------------------------------------------------------------
// Driver error
// ---------------------------------------------------------------------------

/// Stable failures from the telemetry Binding (old
/// `SemanticBindingRuntimeError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryBindingDriverErrorKind {
    /// The stored spec envelope did not decode.
    InvalidResource,
    /// The Binding's Service/target relationship was not admitted, or the
    /// provider-declared child set could not be materialized.
    InvalidRelationship,
    /// A manager route (row read, child ensure, child retire) failed.
    Reconcile,
}

impl TelemetryBindingDriverErrorKind {
    /// The stable lower-kebab code for this classification.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidResource => "semantic-binding-resource-invalid",
            Self::InvalidRelationship => "semantic-binding-relationship-invalid",
            Self::Reconcile => "semantic-binding-reconcile-failed",
        }
    }
}

/// Typed driver failure; redacted at the erased boundary through
/// [`ResourceDriver::classify_error`] (R13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelemetryBindingDriverError {
    kind: TelemetryBindingDriverErrorKind,
    op: DriverOp,
}

impl TelemetryBindingDriverError {
    const fn new(kind: TelemetryBindingDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }
}

impl core::fmt::Display for TelemetryBindingDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl std::error::Error for TelemetryBindingDriverError {}

// ---------------------------------------------------------------------------
// In-memory status (R11)
// ---------------------------------------------------------------------------

/// The provider projection the old reconciler persisted through the Resource
/// API, now in-memory only (R11: runtime status is never persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryBindingStatus {
    /// The projected provider phase.
    pub phase: TelemetryBindingPhase,
    /// The relationship is malformed or dangling (`fenced_owner`).
    pub fenced: bool,
    /// The owned child set is current: this pass made no child mutation.
    pub converged: bool,
    /// The child refs the Provider's declaration derives for this row.
    pub desired_children: Vec<ResourceRef>,
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one telemetry Binding row (KTD2), exactly as
/// persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryBindingSpecEnvelope {
    provider_ref: Option<ResourceRef>,
    base: CanonicalJsonObject,
}

impl TelemetryBindingSpecEnvelope {
    /// The Provider the row names, when the envelope carried one.
    pub fn provider_ref(&self) -> Option<&ResourceRef> {
        self.provider_ref.as_ref()
    }

    /// The type-specific base as a JSON value, as the old reconciler read it.
    fn value(&self) -> Result<serde_json::Value, TelemetryBindingDriverErrorKind> {
        serde_json::from_slice(&self.base.to_canonical_bytes())
            .map_err(|_| TelemetryBindingDriverErrorKind::InvalidResource)
    }
}

/// The manager-wired decode hook for telemetry Binding rows.
pub fn telemetry_binding_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| TelemetryBindingSpecEnvelope {
            provider_ref: spec.provider_ref().cloned(),
            base: spec.base().clone(),
        })
    })
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// [`ResourceDriverFactory`] for the telemetry Binding type. Construction is
/// infallible by contract (R3).
#[derive(Debug)]
pub struct TelemetryBindingDriverFactory {
    types: [ResourceTypeName; 1],
}

impl TelemetryBindingDriverFactory {
    /// Construct the Binding factory.
    pub fn new() -> Self {
        Self {
            types: [ResourceTypeName::new(TELEMETRY_BINDING_TYPE)],
        }
    }
}

impl Default for TelemetryBindingDriverFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for TelemetryBindingDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(TelemetryBindingDriver::new(key))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One telemetry Binding row's driver.
pub struct TelemetryBindingDriver {
    key: ResourceKey,
    /// Targets this driver already registered an internal watch on (R12).
    /// Runtime-only (R13, R6). `WatchCondition::Ready` is one-shot and the
    /// driver cannot observe satisfaction, so one registration per target
    /// keeps the dependency edge (which outlives satisfaction and wakes the
    /// actor on dependency death) without leaking manager watch entries.
    watched: Vec<ResourceKey>,
}

impl TelemetryBindingDriver {
    fn new(key: &ResourceKey) -> Self {
        Self {
            key: key.clone(),
            watched: Vec::new(),
        }
    }

    const fn error(
        &self,
        kind: TelemetryBindingDriverErrorKind,
        op: DriverOp,
    ) -> TelemetryBindingDriverError {
        TelemetryBindingDriverError::new(kind, op)
    }

    /// This row's canonical reference (the child owner reference and the
    /// Binding argument `TelemetryBindingController::child_resources` needs).
    fn row_ref(&self, op: DriverOp) -> Result<ResourceRef, TelemetryBindingDriverError> {
        let resource_type = ContractResourceTypeName::parse(&self.key.type_name)
            .map_err(|_| self.error(TelemetryBindingDriverErrorKind::InvalidResource, op))?;
        let name = ResourceName::parse(&self.key.name)
            .map_err(|_| self.error(TelemetryBindingDriverErrorKind::InvalidResource, op))?;
        Ok(ResourceRef::new(resource_type, name))
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
    ) -> Result<TelemetryBindingSpecEnvelope, TelemetryBindingDriverError> {
        ctx.spec::<TelemetryBindingSpecEnvelope>()
            .cloned()
            .map_err(|_| self.error(TelemetryBindingDriverErrorKind::InvalidResource, op))
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

    /// The provider-declared child set for this Binding, or `None` when the
    /// owner is fenced as the old `telemetry_binding_owner` fenced it: the row
    /// names a different Provider, its Service/target relationship is
    /// malformed, or one of those dependency rows is absent or deleting.
    async fn derived_binding_children(
        &mut self,
        ctx: &mut ResourceContext,
        envelope: &TelemetryBindingSpecEnvelope,
        op: DriverOp,
    ) -> Result<Option<BindingChildSet>, TelemetryBindingDriverError> {
        if envelope
            .provider_ref()
            .map(ResourceRef::to_canonical_string)
            .as_deref()
            != Some(TELEMETRY_PROVIDER_REF)
        {
            return Ok(None);
        }
        let spec = envelope.value().map_err(|kind| self.error(kind, op))?;
        let Some((service_ref, target_ref)) = binding_relationship(&spec) else {
            return Ok(None);
        };
        for dependency in [&service_ref, &target_ref] {
            let key = self.row_key(dependency);
            match ctx.get(&key).await {
                Ok(Some(row)) if !row.deleting => self.watch_once(ctx, key).await,
                Ok(_) => return Ok(None),
                Err(_) => return Err(self.error(TelemetryBindingDriverErrorKind::Reconcile, op)),
            }
        }
        let owner = self.row_ref(op)?;
        TelemetryBindingController::child_resources(&owner, &service_ref, &target_ref)
            .map(Some)
            .map_err(|_| self.error(TelemetryBindingDriverErrorKind::InvalidRelationship, op))
    }

    /// One child ensure built from the provider-declared intent (old Core
    /// `materialize_child_create_payload`: Providers declare intent, Core owns
    /// the child body, and the Process Provider stays Core-chosen).
    fn child_ensure(
        &self,
        intent: &BindingChildIntent,
        zone: &ZoneId,
        owner: &ResourceRef,
        op: DriverOp,
    ) -> Result<ChildEnsure, TelemetryBindingDriverError> {
        let payload = d2b_core_controller::materialize_child_create_payload(intent, zone)
            .map_err(|_| self.error(TelemetryBindingDriverErrorKind::InvalidResource, op))?;
        let value = serde_json::from_slice::<serde_json::Value>(&payload)
            .map_err(|_| self.error(TelemetryBindingDriverErrorKind::InvalidResource, op))?;
        let spec = value
            .get("spec")
            .cloned()
            .ok_or_else(|| self.error(TelemetryBindingDriverErrorKind::InvalidResource, op))?;
        let metadata = serde_json::json!({
            "ownerRef": owner.to_canonical_string(),
            "labels": {},
            "annotations": {},
        });
        Ok(ChildEnsure {
            type_name: ResourceTypeName::new(intent.kind().resource_type()),
            name: intent.resource_ref().name().as_str().to_owned(),
            spec: serde_json::to_vec(&spec)
                .map_err(|_| self.error(TelemetryBindingDriverErrorKind::InvalidResource, op))?,
            metadata: serde_json::to_vec(&metadata)
                .map_err(|_| self.error(TelemetryBindingDriverErrorKind::InvalidResource, op))?,
        })
    }

    /// A fenced Binding: no child mutation and no finalizer change (old
    /// `fenced_owner` + `persist_fenced_binding_status`), provider status
    /// Degraded, and the preserved resync so the fence clears when the
    /// dependency rows appear.
    fn fence_binding(&mut self, ctx: &mut ResourceContext) -> ReconcileOutcome {
        ctx.set_status(TelemetryBindingStatus {
            phase: TelemetryBindingPhase::Degraded,
            fenced: true,
            converged: false,
            desired_children: Vec::new(),
        });
        ctx.requeue_after(TELEMETRY_BINDING_RESYNC);
        ReconcileOutcome::Satisfied
    }

    /// One reconcile pass of the Binding (old `plan` + `reconcile` +
    /// `execute_effect` for the Binding role).
    async fn reconcile_binding(
        &mut self,
        ctx: &mut ResourceContext,
        envelope: &TelemetryBindingSpecEnvelope,
    ) -> Result<ReconcileOutcome, TelemetryBindingDriverError> {
        let op = DriverOp::Reconcile;
        let Some(desired) = self.derived_binding_children(ctx, envelope, op).await? else {
            return Ok(self.fence_binding(ctx));
        };
        let zone = ZoneId::parse(self.key.zone.clone())
            .map_err(|_| self.error(TelemetryBindingDriverErrorKind::InvalidResource, op))?;
        let owner = self.row_ref(op)?;

        // Ensure every desired child; the manager commits each row before its
        // actor exists (F1, AE1). Declaration order puts each Endpoint after
        // its producing Process.
        let mut mutated = false;
        for intent in desired.iter() {
            let child = self.child_ensure(intent, &zone, &owner, op)?;
            match ctx.ensure_child(child).await {
                Ok(EnsureOutcome::Created(_)) | Ok(EnsureOutcome::Updated(_)) => mutated = true,
                Ok(EnsureOutcome::Unchanged(_)) => {}
                Err(_) => return Err(self.error(TelemetryBindingDriverErrorKind::Reconcile, op)),
            }
        }

        // Retire owned children the desired set no longer derives (old Core
        // owner diff), endpoint-first / process-last (old `mutation_order`).
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(TelemetryBindingDriverErrorKind::Reconcile, op))?;
        let desired_refs = desired
            .iter()
            .map(|intent| intent.resource_ref().clone())
            .collect::<Vec<_>>();
        let mut obsolete = owned
            .iter()
            .filter(|row| {
                !row.deleting && !desired_refs.iter().any(|reference| is_child_row(row, reference))
            })
            .collect::<Vec<_>>();
        obsolete.sort_by_key(|row| (teardown_rank(&row.key.type_name), row.key.name.clone()));
        for row in obsolete {
            ctx.delete(&row.key)
                .await
                .map_err(|_| self.error(TelemetryBindingDriverErrorKind::Reconcile, op))?;
            mutated = true;
        }

        // Dependency edges for the desired children that have rows (R12/R17):
        // a child's death or readiness wakes this actor to re-ensure.
        let child_targets = owned
            .iter()
            .filter(|row| desired_refs.iter().any(|reference| is_child_row(row, reference)))
            .map(|row| row.key.clone())
            .collect::<Vec<_>>();
        for target in child_targets {
            self.watch_once(ctx, target).await;
        }

        let converged = !mutated;
        // CONTRACT FLAG: the old phase was `ready ? Ready : Degraded` once
        // converged; the readiness term is unobservable here, so a converged
        // owner reports the fail-closed Degraded projection.
        let phase = if converged {
            TelemetryBindingPhase::Degraded
        } else {
            TelemetryBindingPhase::Pending
        };
        ctx.set_status(TelemetryBindingStatus {
            phase,
            fenced: false,
            converged,
            desired_children: desired_refs,
        });
        if !converged {
            ctx.requeue_after(TELEMETRY_BINDING_RESYNC);
        }
        Ok(ReconcileOutcome::Satisfied)
    }
}

#[async_trait::async_trait]
impl ResourceDriver for TelemetryBindingDriver {
    type Error = TelemetryBindingDriverError;

    fn classify_error(&self, error: &TelemetryBindingDriverError) -> DriverFailure {
        // The old reconciler classified every failure retryable; the actor
        // owns retry/backoff from the closed class (R13).
        DriverFailure::retryable(error.op)
    }

    /// Structural validation only (old `validate_spec`): the stored envelope
    /// must decode. A malformed Service/target relationship is fenced in
    /// reconcile (old `telemetry_binding_owner`), never fatal here.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let _ = self.envelope(ctx, DriverOp::Validate)?;
        Ok(())
    }

    /// Discovery and adoption on the realization target (F2, R15-R16): the
    /// Binding adopts when its durable owned-child rows are already current.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let envelope = self.envelope(ctx, op)?;
        let Some(desired) = self.derived_binding_children(ctx, &envelope, op).await? else {
            return Ok(RecoveryOutcome::Missing);
        };
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(TelemetryBindingDriverErrorKind::Reconcile, op))?;
        let current = desired.iter().all(|intent| {
            owned
                .iter()
                .any(|row| is_child_row(row, intent.resource_ref()) && !row.deleting)
        });
        Ok(if current {
            RecoveryOutcome::Adopted
        } else {
            RecoveryOutcome::Missing
        })
    }

    /// One reconcile pass (old `plan` + `reconcile` + `execute_effect`).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let envelope = self.envelope(ctx, op)?;
        self.reconcile_binding(ctx, &envelope).await
    }

    /// Teardown (old `prepare_finalize` + `execute_finalize` + `finalize`).
    ///
    /// The durable deleting mark is already committed and the manager has
    /// already cascaded the owned children (`remove_internal`) before this
    /// pass runs; the parent row stays until the last child retires. The
    /// telemetry family realizes nothing else on a target, so there is no
    /// further effect to run here.
    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Spec projection helpers (old `binding_relationship`, and the Core owner
// diff's child matching)
// ---------------------------------------------------------------------------

/// The Binding's admitted Service and producer target.
fn binding_relationship(spec: &serde_json::Value) -> Option<(ResourceRef, ResourceRef)> {
    let service_ref = value_ref(spec.get("serviceRef"))?;
    let target_ref = value_ref(spec.get("producerRef"))?;
    Some((service_ref, target_ref))
}

fn value_ref(value: Option<&serde_json::Value>) -> Option<ResourceRef> {
    ResourceRef::parse(value?.as_str()?).ok()
}

/// Whether one owned child row is the resource reference the Provider
/// declared (the old Core owner diff matched the complete reference, not the
/// name alone).
fn is_child_row(row: &StoredDesiredResource, reference: &ResourceRef) -> bool {
    row.key.type_name == reference.resource_type().as_str()
        && row.key.name == reference.name().as_str()
}

/// Old `BindingChildKind` teardown ranks: endpoints retire before their
/// producing processes.
fn teardown_rank(resource_type: &str) -> u8 {
    match resource_type {
        "Endpoint" => 0,
        "EphemeralProcess" => 1,
        _ => 2,
    }
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The execution domains the TelemetryBinding type can be reconciled in.
///
/// Derived from the placement contract: `TelemetryBinding` names no placement
/// anchor (`PlacementAnchor::canonical_for` resolves none), so a Binding row
/// never carries the canonical `spec.executionRef` and the plane reconciles it
/// on its own Host domain. The Provider declares every child host-placed, so
/// the declared set is realized from that same domain.
const TELEMETRY_BINDING_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the Binding realization reads while reconciling.
///
/// Derived from the driver's row reads: the relationship reads the declared
/// `TelemetryService` row and its producer target (`Zone` or `Guest`), and
/// reconcile re-reads the owned `Process` and `Endpoint` children.
const TELEMETRY_BINDING_READS: &[WellKnownType] = &[
    WellKnownType::TELEMETRY_SERVICE,
    WellKnownType::ZONE,
    WellKnownType::GUEST,
    WellKnownType::PROCESS,
    WellKnownType::ENDPOINT,
];

/// The TelemetryBinding type's driver declaration.
///
/// `TelemetryBinding` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane
/// cannot serve the Zone's telemetry producers without it, so it must be
/// registered before the plane opens. The type is not exportable:
/// `ResourceExport` admits only qualified `*.d2bus.org.*Service` types, so a
/// binding can never be an export subject. The driver serves no broker
/// operations and declares the two child creations it performs - the
/// collector/forwarder Process and its Endpoint.
pub fn telemetry_binding_descriptor() -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::TELEMETRY_BINDING,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: TELEMETRY_BINDING_EXECUTION_DOMAINS,
        exportable: false,
        reads: TELEMETRY_BINDING_READS,
        operations: &[],
        creations: TELEMETRY_BINDING_CREATIONS,
        startup: &[],
        services: &[],
        decoder: telemetry_binding_spec_decoder(),
        factory: Arc::new(TelemetryBindingDriverFactory::new()),
    }
}

// ---------------------------------------------------------------------------
// Tests: driver-level behavior over a recording manager endpoint (the same
// shape U7 used) and a requeue recorder. Effects are observed as the manager
// records them: child ensure order, teardown order, and watch registration.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use d2b_provider_toolkit::testing::fakes::{RecordingManagerEndpoint, RecordingRequeue};
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueScheduler, ResourceContext, WatchId,
        WatchRegistration,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::ResourceProvenance;
    use d2b_resource_runtime::spec_store::{EnsureOutcome, StoredDesiredResource};
    use tokio::sync::mpsc;

    use super::*;

    // -- fakes ---------------------------------------------------------------

    /// Manager endpoint double: an owned-row store plus the ordered call log
    /// the assertions read (ensure order, teardown order, watch targets).
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

        async fn seed(&self, row: StoredDesiredResource) {
            self.rows.lock().await.push(row);
        }

        async fn seed_child(&self, zone: &str, reference: &str, spec: serde_json::Value) {
            let mut row = row(zone, reference, spec);
            row.owner_uid = Some(self.parent_uid);
            row.provenance = ResourceProvenance::Resource;
            self.seed(row).await;
        }

        async fn log(&self) -> Vec<String> {
            self.log.lock().await.clone()
        }

        async fn watch_targets(&self) -> Vec<ResourceKey> {
            self.watch_targets.lock().await.clone()
        }

        async fn rows_of_type(&self, type_name: &str) -> Vec<StoredDesiredResource> {
            self.rows
                .lock().await
                .iter()
                .filter(|row| row.key.type_name == type_name)
                .cloned()
                .collect()
        }

        fn spec_of(&self, row: &StoredDesiredResource) -> serde_json::Value {
            serde_json::from_slice(&row.spec).expect("child spec is JSON")
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            let mut rows = self.rows.lock().await;
            let existing = rows.iter_mut().find(|row| {
                row.key.type_name == child.type_name.as_str() && row.key.name == child.name
            });
            let (outcome, label) = match existing {
                Some(row) if row.spec == child.spec && row.metadata == child.metadata => {
                    (EnsureOutcome::Unchanged(row.clone()), "unchanged")
                }
                Some(row) => {
                    row.spec = child.spec.clone();
                    row.metadata = child.metadata.clone();
                    row.generation += 1;
                    (EnsureOutcome::Updated(row.clone()), "updated")
                }
                None => {
                    let row = StoredDesiredResource {
                        key: ResourceKey::new(
                            "dev",
                            child.type_name.as_str(),
                            child.name.clone(),
                        ),
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
                    (EnsureOutcome::Created(row), "created")
                }
            };
            self.log.lock().await.push(format!(
                "ensure:{}/{}:{label}",
                child.type_name.as_str(),
                child.name
            ));
            Ok(outcome)
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self
                .rows
                .lock().await
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
                .lock().await
                .push(format!("delete:{}/{}", key.type_name, key.name));
            if let Some(row) = self
                .rows
                .lock().await
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
                .lock().await
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
            let mut targets = self.watch_targets.lock().await;
            targets.push(registration.target.clone());
            Ok(WatchId(targets.len() as u64))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
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
            telemetry_binding_spec_decoder(),
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
        TelemetryBindingDriverFactory::new()
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

    fn binding_spec(provider_ref: &str) -> serde_json::Value {
        serde_json::json!({
            "providerRef": provider_ref,
            "serviceRef": "telemetry.d2bus.org.TelemetryService/ingest",
            "producerRef": "Zone/dev",
        })
    }

    fn binding_row(spec: serde_json::Value) -> StoredDesiredResource {
        row("dev", "telemetry.d2bus.org.TelemetryBinding/metrics", spec)
    }

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

    fn target_row() -> StoredDesiredResource {
        row("dev", "Zone/dev", serde_json::json!({}))
    }

    // -- factory -------------------------------------------------------------

    // -- validate ------------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn validate_rejects_a_malformed_spec() {
        let mut row = binding_row(binding_spec(TELEMETRY_PROVIDER_REF));
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn validate_accepts_a_provider_declared_spec() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        let mut driver = driver(&fixture).await;
        driver.validate(&mut fixture.ctx).await.expect("valid spec");
    }

    // -- reconcile -----------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn binding_reconcile_ensures_provider_declared_children_then_converges() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        fixture.manager.seed(service_row()).await;
        fixture.manager.seed(target_row()).await;
        let mut driver = driver(&fixture).await;

        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let log = fixture.manager.log().await;
        assert_eq!(log.len(), 2, "collector Process then ingest Endpoint: {log:?}");
        assert!(
            log[0].starts_with("ensure:Process/") && log[0].ends_with(":created"),
            "{log:?}"
        );
        assert!(
            log[1].starts_with("ensure:Endpoint/") && log[1].ends_with(":created"),
            "{log:?}"
        );

        // The collector is a Process resource (KTD13): the Process
        // controller launches it, this driver only ensures the row.
        let processes = fixture.manager.rows_of_type("Process").await;
        assert_eq!(processes.len(), 1);
        let process_spec = fixture.manager.spec_of(&processes[0]);
        assert_eq!(process_spec["template"], "otel-collector");
        assert_eq!(process_spec["providerRef"], "Provider/system-minijail");
        assert_eq!(process_spec["executionRef"], "Host/host-system");
        assert_eq!(process_spec["processClass"], "service");
        assert_eq!(process_spec["domain"], "system");
        let endpoints = fixture.manager.rows_of_type("Endpoint").await;
        assert_eq!(endpoints.len(), 1);
        let endpoint_spec = fixture.manager.spec_of(&endpoints[0]);
        assert_eq!(endpoint_spec["providerRef"], TELEMETRY_PROVIDER_REF);
        assert_eq!(
            endpoint_spec["producerRef"],
            format!("Process/{}", processes[0].key.name)
        );
        assert_eq!(endpoint_spec["purpose"], "ingest-endpoint");
        assert_eq!(endpoint_spec["lifecyclePolicy"], "recycle-with-producer");

        let Some(status) = fixture.ctx.status::<TelemetryBindingStatus>() else {
            panic!("binding status");
        };
        assert_eq!(
            status.phase,
            TelemetryBindingPhase::Pending,
            "first pass mutated the child set"
        );
        assert!(!status.fenced);
        assert!(!status.converged);
        assert_eq!(status.desired_children.len(), 2);
        assert_eq!(fixture.requeue.scheduled(), vec![TELEMETRY_BINDING_RESYNC]);

        // Second pass: the owned child set is current; every ensure is a
        // no-op and no resync is scheduled.
        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let log = fixture.manager.log().await;
        assert_eq!(log.len(), 4, "both desired children re-ensured: {log:?}");
        assert!(
            log[2].ends_with(":unchanged") && log[3].ends_with(":unchanged"),
            "the second pass mutates nothing: {log:?}"
        );
        let Some(status) = fixture.ctx.status::<TelemetryBindingStatus>() else {
            panic!("binding status");
        };
        assert!(status.converged, "second pass converged");
        // CONTRACT FLAG: the old `ready ? Ready : Degraded` phase reports the
        // fail-closed projection while readiness is unobservable.
        assert_eq!(status.phase, TelemetryBindingPhase::Degraded);
        assert_eq!(
            fixture.requeue.scheduled(),
            vec![TELEMETRY_BINDING_RESYNC],
            "converged owners stop rescheduling"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn binding_reconcile_fences_a_dangling_service_dependency() {
        // The Service/target rows are absent: the old owner fenced, mutated
        // no children, and reported Degraded.
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        let mut driver = driver(&fixture).await;

        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        assert!(fixture.manager.log().await.is_empty(), "fenced owners mutate nothing");
        assert!(fixture.manager.rows_of_type("Process").await.is_empty());
        let Some(status) = fixture.ctx.status::<TelemetryBindingStatus>() else {
            panic!("binding status");
        };
        assert!(status.fenced);
        assert!(!status.converged);
        assert_eq!(status.phase, TelemetryBindingPhase::Degraded);
        assert_eq!(
            fixture.requeue.scheduled(),
            vec![TELEMETRY_BINDING_RESYNC],
            "the preserved resync re-evaluates the fence"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn binding_reconcile_fences_a_foreign_provider() {
        let mut fixture = fixture(binding_row(binding_spec("Provider/other")));
        fixture.manager.seed(service_row()).await;
        fixture.manager.seed(target_row()).await;
        let mut driver = driver(&fixture).await;

        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert!(
            fixture.manager.log().await.is_empty(),
            "a foreign Provider never derives children"
        );
        assert!(
            fixture
                .ctx
                .status::<TelemetryBindingStatus>()
                .is_some_and(|status| status.fenced)
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn binding_reconcile_retires_obsolete_children_endpoint_first() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        fixture.manager.seed(service_row()).await;
        fixture.manager.seed(target_row()).await;
        // Owned children the current target no longer derives (the old Core
        // owner diff retired these).
        fixture.manager.seed_child(
            "dev",
            "Process/telemetry-forwarder-aaaaaaaa",
            serde_json::json!({"providerRef": "Provider/system-minijail"}),
        ).await;
        fixture.manager.seed_child(
            "dev",
            "Endpoint/telemetry-forwarder-endpoint-bbbbbbbb",
            serde_json::json!({"providerRef": TELEMETRY_PROVIDER_REF}),
        ).await;
        let mut driver = driver(&fixture).await;

        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let deletes = fixture
            .manager
            .log().await
            .into_iter()
            .filter(|entry| entry.starts_with("delete:"))
            .collect::<Vec<_>>();
        assert_eq!(
            deletes,
            vec![
                "delete:Endpoint/telemetry-forwarder-endpoint-bbbbbbbb".to_owned(),
                "delete:Process/telemetry-forwarder-aaaaaaaa".to_owned(),
            ],
            "endpoint-first / process-last teardown"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn dependency_watches_are_registered_once_per_target() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        fixture.manager.seed(service_row()).await;
        fixture.manager.seed(target_row()).await;
        let mut driver = driver(&fixture).await;

        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let mut targets = fixture
            .manager
            .watch_targets().await
            .into_iter()
            .map(|key| format!("{}/{}", key.type_name, key.name))
            .collect::<Vec<_>>();
        let registered = targets.len();
        targets.sort();
        targets.dedup();
        assert_eq!(registered, targets.len(), "no watch is registered twice");
        assert!(
            targets.contains(&format!("{}/{}", "telemetry.d2bus.org.TelemetryService", "ingest")),
            "{targets:?}"
        );
    }

    // -- recover -------------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn binding_recover_adopts_only_a_current_owned_child_set() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        fixture.manager.seed(service_row()).await;
        fixture.manager.seed(target_row()).await;
        let mut driver = driver(&fixture).await;

        assert_eq!(
            driver.recover(&mut fixture.ctx).await.expect("recover"),
            RecoveryOutcome::Missing,
            "no owned children yet"
        );

        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert_eq!(
            driver.recover(&mut fixture.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted,
            "the durable child rows reconstruct the owned child set (R15)"
        );
    }

    // -- delete --------------------------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn delete_runs_no_effect_past_the_manager_cascade() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        fixture.manager.seed(service_row()).await;
        fixture.manager.seed(target_row()).await;
        let mut driver = driver(&fixture).await;

        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let before = fixture.manager.log().await.len();
        driver.delete(&mut fixture.ctx).await.expect("delete");
        assert_eq!(
            fixture.manager.log().await.len(),
            before,
            "the manager cascades owned children before the driver delete pass"
        );
    }

    // -- manager failure -----------------------------------------------------

    /// A manager read failure during child derivation surfaces as the
    /// `Reconcile`-class retryable error, never as a fence or a silent
    /// partial projection.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_reports_a_manager_read_failure_as_retryable() {
        let row = binding_row(binding_spec(TELEMETRY_PROVIDER_REF));
        let requeue = Arc::new(RecordingRequeue::default());
        let (effects_tx, _effects_rx) = mpsc::unbounded_channel();
        let (watch_tx, _watch_rx) = mpsc::unbounded_channel();
        let fail_reads = RecordingManagerEndpoint::new();
        fail_reads.set_fail_reads(true);
        let mut ctx = ResourceContext::new(
            row,
            telemetry_binding_spec_decoder(),
            Arc::new(fail_reads) as Arc<dyn ManagerEndpoint>,
            Arc::clone(&requeue) as Arc<dyn RequeueScheduler>,
            effects_tx,
            watch_tx,
        );
        let mut driver: Box<dyn DynResourceDriver> =
            TelemetryBindingDriverFactory::new().create(ctx.key()).await;

        let failure = driver.reconcile(&mut ctx).await.expect_err("manager read failure");
        assert_eq!(failure.op(), DriverOp::Reconcile);
        assert_eq!(failure.class(), FailureClass::Retryable);
    }
}
