//! Telemetry Service/Binding reconciliation on the v3 resource runtime
//! (U12 conversion; KTD3, KTD4, KTD13).
//!
//! Provider controllers remain the authority for their Service and Binding
//! semantics: the telemetry Provider declares UID-free child intents
//! (`TelemetryBindingController::child_resources`) and Core materializes them.
//! This module is the KTD3 conversion of the old `TelemetryResourceReconciler`
//! plus `telemetry_controller_descriptor` runner lane (spec section 13
//! mapping):
//!
//! - `describe` -> [`TelemetryDriverFactory`], registered for
//!   `telemetry.d2bus.org.TelemetryService` and
//!   `telemetry.d2bus.org.TelemetryBinding` in the plane's provider directory.
//! - `validate_spec` -> [`ResourceDriver::validate`]: the stored spec envelope
//!   must decode.
//! - `observe` -> [`ResourceDriver::recover`]: adoption of the owned child set
//!   from the durable rows (R15). A Service realizes nothing on a target, so
//!   it recovers as adopted.
//! - `plan` + `reconcile` + `execute_effect` -> [`ResourceDriver::reconcile`]:
//!   the collector/forwarder Process and Endpoint children are ensured through
//!   the manager (commit before spawn, F1), owned children the desired set no
//!   longer derives are retired in the preserved endpoint-first /
//!   process-last order, and the provider status projection moves into the
//!   driver's in-memory slot ([`ResourceContext::set_status`], R11: zero
//!   persistent writes).
//! - `prepare_finalize` + `execute_finalize` + `finalize` ->
//!   [`ResourceDriver::delete`]. The old
//!   `d2b.d2bus.org/binding-children` finalizer is gone by construction: the
//!   v3 manager already holds a parent row until its owned children retire
//!   (`ResourceManagerState::remove_internal` cascades the removal to owned
//!   children and `pending_retirement` keeps the parent's durable deleting
//!   mark observable), which is the guarantee that finalizer existed for
//!   (F3, R9-R10). Nothing else is realized on a target, so teardown is
//!   complete by the time the driver's delete pass runs.
//!
//! KTD13: the telemetry collector and forwarder are Process resources owned by
//! the Binding, so their launch belongs to the Process controller; this driver
//! never spawns a child process.
//!
//! # Contract flag (KTD3 fit; reported with this unit, not worked around)
//!
//! The preserved provider phase reads *another resource's* observed state: the
//! ingest Endpoints' status for a Service, and the children's status for a
//! Binding. [`ResourceContext`] offers
//! `ensure` / `get` / `delete` / `watch` / `set_status` / `requeue_after` /
//! `children`, and none of them returns a dependency's runtime status:
//! `get`/`children` return desired rows, and `WatchCondition::Ready`
//! satisfaction is delivered to the *actor* (which re-reconciles) rather than
//! to the driver. The readiness term of both phase predicates therefore
//! evaluates fail-closed (see [`DEPENDENCY_READINESS_PROVEN`]), and a
//! dependency-proven `Ready` phase is unreachable until the surface carries
//! observed state. The data already exists manager-side
//! (`ResourceManagerMsg::Get` returns a `ResourceView` with `status`); only the
//! driver-facing `ManagerEndpoint` lacks the read.
#![allow(dead_code)]

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

/// Qualified semantic telemetry Service type (old `TELEMETRY_BINDING.0`).
pub(crate) const TELEMETRY_SERVICE_TYPE: &str = "telemetry.d2bus.org.TelemetryService";

/// Qualified semantic telemetry Binding type (old `TELEMETRY_BINDING.1`).
pub(crate) const TELEMETRY_BINDING_TYPE: &str = TELEMETRY_BINDING_RESOURCE_TYPE;

/// The serving Provider this driver owns (old `TELEMETRY_BINDING.2`).
const TELEMETRY_PROVIDER_REF: &str = "Provider/observability-otel";

/// Preserved resync period (old `ResyncPolicy::new(None, 5_000)`). The actor
/// owns scheduling now (R13), so the driver re-schedules itself while the
/// resource is not converged instead of polling from a runner.
const TELEMETRY_RESYNC: Duration = Duration::from_secs(5);

/// Provider phase spellings the old reconciler published.
const PHASE_READY: &str = "Ready";
const PHASE_PENDING: &str = "Pending";
const PHASE_DEGRADED: &str = "Degraded";

/// The readiness term of the preserved phase predicates.
///
/// CONTRACT FLAG: the term reads a dependency's observed status (an ingest
/// Endpoint's `status.phase` for a Service, a child's phase for a Binding),
/// which the KTD3 driver surface does not expose. It evaluates fail-closed
/// until the surface carries observed state, so `Ready` is never claimed
/// without evidence (see the module header).
const DEPENDENCY_READINESS_PROVEN: bool = false;

// ---------------------------------------------------------------------------
// Driver error
// ---------------------------------------------------------------------------

/// Stable failures from the telemetry family (old
/// `SemanticBindingRuntimeError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TelemetryDriverErrorKind {
    /// The stored spec envelope did not decode.
    InvalidResource,
    /// The Binding's Service/target relationship was not admitted, or the
    /// provider-declared child set could not be materialized.
    InvalidRelationship,
    /// A manager route (row read, child ensure, child retire) failed.
    Reconcile,
}

impl TelemetryDriverErrorKind {
    const fn as_str(self) -> &'static str {
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
pub(crate) struct TelemetryDriverError {
    kind: TelemetryDriverErrorKind,
    op: DriverOp,
}

impl TelemetryDriverError {
    const fn new(kind: TelemetryDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }
}

impl core::fmt::Display for TelemetryDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl std::error::Error for TelemetryDriverError {}

// ---------------------------------------------------------------------------
// In-memory status (R11)
// ---------------------------------------------------------------------------

/// The provider projection the old reconciler persisted through the Resource
/// API, now in-memory only (R11: runtime status is never persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TelemetryStatus {
    /// TelemetryService projection (old `telemetry_service_status`).
    Service {
        phase: &'static str,
        /// `{serviceRole, serviceReadiness}`; empty when the spec is degraded.
        projection: serde_json::Value,
        /// Declared ingest endpoint refs whose rows exist and are not deleting.
        present_endpoints: Vec<ResourceRef>,
    },
    /// TelemetryBinding projection (old `persist_semantic_binding_status`).
    Binding {
        phase: &'static str,
        /// The relationship is malformed or dangling (`fenced_owner`).
        fenced: bool,
        /// The owned child set is current: this pass made no child mutation.
        converged: bool,
        desired_children: Vec<ResourceRef>,
    },
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one telemetry Service/Binding row (KTD2),
/// exactly as persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TelemetrySpecEnvelope {
    provider_ref: Option<ResourceRef>,
    base: CanonicalJsonObject,
}

impl TelemetrySpecEnvelope {
    pub(crate) fn provider_ref(&self) -> Option<&ResourceRef> {
        self.provider_ref.as_ref()
    }

    /// The type-specific base as a JSON value, as the old reconciler read it.
    fn value(&self) -> Result<serde_json::Value, TelemetryDriverErrorKind> {
        serde_json::from_slice(&self.base.to_canonical_bytes())
            .map_err(|_| TelemetryDriverErrorKind::InvalidResource)
    }
}

/// The manager-wired decode hook for telemetry Service and Binding rows.
pub(crate) fn telemetry_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| TelemetrySpecEnvelope {
            provider_ref: spec.provider_ref().cloned(),
            base: spec.base().clone(),
        })
    })
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// [`ResourceDriverFactory`] for the telemetry Service and Binding types.
/// Construction is infallible by contract (R3).
pub(crate) struct TelemetryDriverFactory {
    types: [ResourceTypeName; 2],
}

impl TelemetryDriverFactory {
    pub(crate) fn new() -> Self {
        Self {
            types: [
                ResourceTypeName::new(TELEMETRY_BINDING_TYPE),
                ResourceTypeName::new(TELEMETRY_SERVICE_TYPE),
            ],
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for TelemetryDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(TelemetryDriver::new(key))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One telemetry Service or Binding row's driver.
pub(crate) struct TelemetryDriver {
    key: ResourceKey,
    binding: bool,
    /// Targets this driver already registered an internal watch on (R12).
    /// Runtime-only (R13, R6). `WatchCondition::Ready` is one-shot and the
    /// driver cannot observe satisfaction, so one registration per target
    /// keeps the dependency edge (which outlives satisfaction and wakes the
    /// actor on dependency death) without leaking manager watch entries.
    watched: Vec<ResourceKey>,
}

impl TelemetryDriver {
    fn new(key: &ResourceKey) -> Self {
        Self {
            key: key.clone(),
            binding: key.type_name == TELEMETRY_BINDING_TYPE,
            watched: Vec::new(),
        }
    }

    const fn error(&self, kind: TelemetryDriverErrorKind, op: DriverOp) -> TelemetryDriverError {
        TelemetryDriverError::new(kind, op)
    }

    /// This row's canonical reference (the child owner reference and the
    /// Binding argument `TelemetryBindingController::child_resources` needs).
    fn row_ref(&self, op: DriverOp) -> Result<ResourceRef, TelemetryDriverError> {
        let resource_type = ContractResourceTypeName::parse(&self.key.type_name)
            .map_err(|_| self.error(TelemetryDriverErrorKind::InvalidResource, op))?;
        let name = ResourceName::parse(&self.key.name)
            .map_err(|_| self.error(TelemetryDriverErrorKind::InvalidResource, op))?;
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
    ) -> Result<TelemetrySpecEnvelope, TelemetryDriverError> {
        ctx.spec::<TelemetrySpecEnvelope>()
            .cloned()
            .map_err(|_| self.error(TelemetryDriverErrorKind::InvalidResource, op))
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
        envelope: &TelemetrySpecEnvelope,
        op: DriverOp,
    ) -> Result<Option<BindingChildSet>, TelemetryDriverError> {
        if envelope
            .provider_ref()
            .map(ResourceRef::to_canonical_string)
            .as_deref()
            != Some(TELEMETRY_PROVIDER_REF)
        {
            return Ok(None);
        }
        let spec = envelope
            .value()
            .map_err(|kind| self.error(kind, op))?;
        let Some((service_ref, target_ref)) = binding_relationship(&spec) else {
            return Ok(None);
        };
        for dependency in [&service_ref, &target_ref] {
            let key = self.row_key(dependency);
            match ctx.get(&key).await {
                Ok(Some(row)) if !row.deleting => self.watch_once(ctx, key).await,
                Ok(_) => return Ok(None),
                Err(_) => return Err(self.error(TelemetryDriverErrorKind::Reconcile, op)),
            }
        }
        let owner = self.row_ref(op)?;
        TelemetryBindingController::child_resources(&owner, &service_ref, &target_ref)
            .map(Some)
            .map_err(|_| self.error(TelemetryDriverErrorKind::InvalidRelationship, op))
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
    ) -> Result<ChildEnsure, TelemetryDriverError> {
        let payload = d2b_core_controller::materialize_child_create_payload(intent, zone)
            .map_err(|_| self.error(TelemetryDriverErrorKind::InvalidResource, op))?;
        let value = serde_json::from_slice::<serde_json::Value>(&payload)
            .map_err(|_| self.error(TelemetryDriverErrorKind::InvalidResource, op))?;
        let spec = value
            .get("spec")
            .cloned()
            .ok_or_else(|| self.error(TelemetryDriverErrorKind::InvalidResource, op))?;
        let metadata = serde_json::json!({
            "ownerRef": owner.to_canonical_string(),
            "labels": {},
            "annotations": {},
        });
        Ok(ChildEnsure {
            type_name: ResourceTypeName::new(intent.kind().resource_type()),
            name: intent.resource_ref().name().as_str().to_owned(),
            spec: serde_json::to_vec(&spec)
                .map_err(|_| self.error(TelemetryDriverErrorKind::InvalidResource, op))?,
            metadata: serde_json::to_vec(&metadata)
                .map_err(|_| self.error(TelemetryDriverErrorKind::InvalidResource, op))?,
        })
    }

    /// A fenced Binding: no child mutation and no finalizer change (old
    /// `fenced_owner` + `persist_fenced_binding_status`), provider status
    /// Degraded, and the preserved resync so the fence clears when the
    /// dependency rows appear.
    fn fence_binding(&mut self, ctx: &mut ResourceContext) -> ReconcileOutcome {
        ctx.set_status(TelemetryStatus::Binding {
            phase: PHASE_DEGRADED,
            fenced: true,
            converged: false,
            desired_children: Vec::new(),
        });
        ctx.requeue_after(TELEMETRY_RESYNC);
        ReconcileOutcome::Satisfied
    }

    async fn reconcile_service(
        &mut self,
        ctx: &mut ResourceContext,
        envelope: &TelemetrySpecEnvelope,
    ) -> Result<ReconcileOutcome, TelemetryDriverError> {
        let op = DriverOp::Reconcile;
        let spec = envelope.value().map_err(|kind| self.error(kind, op))?;
        let role = spec
            .get("serviceRole")
            .and_then(serde_json::Value::as_str)
            .filter(|role| matches!(*role, "authority" | "projection"));
        let Some(role) = role else {
            // Old: a Service whose role is absent or unadmitted reports
            // Degraded with an empty projection and mutates nothing.
            ctx.set_status(TelemetryStatus::Service {
                phase: PHASE_DEGRADED,
                projection: serde_json::json!({}),
                present_endpoints: Vec::new(),
            });
            return Ok(ReconcileOutcome::Satisfied);
        };
        if role == "projection" {
            // Old: a projection Service is Ready without an ingest route.
            ctx.set_status(TelemetryStatus::Service {
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
                Err(_) => return Err(self.error(TelemetryDriverErrorKind::Reconcile, op)),
            }
        }
        let ready = all_present && DEPENDENCY_READINESS_PROVEN;
        let phase = if ready { PHASE_READY } else { PHASE_PENDING };
        ctx.set_status(TelemetryStatus::Service {
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
            ctx.requeue_after(TELEMETRY_RESYNC);
        }
        Ok(ReconcileOutcome::Satisfied)
    }

    async fn reconcile_binding(
        &mut self,
        ctx: &mut ResourceContext,
        envelope: &TelemetrySpecEnvelope,
    ) -> Result<ReconcileOutcome, TelemetryDriverError> {
        let op = DriverOp::Reconcile;
        let Some(desired) = self.derived_binding_children(ctx, envelope, op).await? else {
            return Ok(self.fence_binding(ctx));
        };
        let zone = ZoneId::parse(self.key.zone.clone())
            .map_err(|_| self.error(TelemetryDriverErrorKind::InvalidResource, op))?;
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
                Err(_) => return Err(self.error(TelemetryDriverErrorKind::Reconcile, op)),
            }
        }

        // Retire owned children the desired set no longer derives (old Core
        // owner diff), endpoint-first / process-last (old `mutation_order`).
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(TelemetryDriverErrorKind::Reconcile, op))?;
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
                .map_err(|_| self.error(TelemetryDriverErrorKind::Reconcile, op))?;
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
            PHASE_DEGRADED
        } else {
            PHASE_PENDING
        };
        ctx.set_status(TelemetryStatus::Binding {
            phase,
            fenced: false,
            converged,
            desired_children: desired_refs,
        });
        if !converged {
            ctx.requeue_after(TELEMETRY_RESYNC);
        }
        Ok(ReconcileOutcome::Satisfied)
    }
}

#[async_trait::async_trait]
impl ResourceDriver for TelemetryDriver {
    type Error = TelemetryDriverError;

    fn classify_error(&self, error: &TelemetryDriverError) -> DriverFailure {
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
    /// Binding adopts when its durable owned-child rows are already current;
    /// the Service realizes nothing on a target (its observed state is the
    /// ingest-endpoint rows reconcile re-reads).
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let envelope = self.envelope(ctx, op)?;
        if !self.binding {
            return Ok(RecoveryOutcome::Adopted);
        }
        let Some(desired) = self.derived_binding_children(ctx, &envelope, op).await? else {
            return Ok(RecoveryOutcome::Missing);
        };
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(TelemetryDriverErrorKind::Reconcile, op))?;
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
        if self.binding {
            self.reconcile_binding(ctx, &envelope).await
        } else {
            self.reconcile_service(ctx, &envelope).await
        }
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown. The call nudges the collector/forwarder
    /// Process and Endpoint children through their own finalize-before-delete
    /// pass and requeues this pass while any child row is still live.
    /// Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| self.error(TelemetryDriverErrorKind::Reconcile, DriverOp::Delete))?;
        Ok(())
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
// Spec projection helpers (old `telemetry_endpoint_refs`,
// `binding_relationship`, and the Core owner diff's child matching)
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
// Tests: driver-level behavior over a recording manager endpoint (the same
// shape U7 used) and a requeue recorder. Effects are observed as the manager
// records them: child ensure order, teardown order, and watch registration.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use d2b_resource_runtime::context::{
        ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, WatchId, WatchRegistration,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::ResourceProvenance;
    use d2b_resource_runtime::target::TargetHandle;
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

        fn seed(&self, row: StoredDesiredResource) {
            self.rows.lock().expect("rows").push(row);
        }

        fn seed_child(&self, zone: &str, reference: &str, spec: serde_json::Value) {
            let mut row = row(zone, reference, spec);
            row.owner_uid = Some(self.parent_uid);
            row.provenance = ResourceProvenance::Resource;
            self.seed(row);
        }

        fn drop_row(&self, key: &ResourceKey) {
            self.rows
                .lock()
                .expect("rows")
                .retain(|row| row.key != *key);
        }

        fn log(&self) -> Vec<String> {
            self.log.lock().expect("log").clone()
        }

        fn watch_targets(&self) -> Vec<ResourceKey> {
            self.watch_targets.lock().expect("watch targets").clone()
        }

        fn rows_of_type(&self, type_name: &str) -> Vec<StoredDesiredResource> {
            self.rows
                .lock()
                .expect("rows")
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
            let mut rows = self.rows.lock().expect("rows");
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
            self.log.lock().expect("log").push(format!(
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
            telemetry_spec_decoder(),
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
        TelemetryDriverFactory::new().create(fixture.ctx.key()).await
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

    fn target_row() -> StoredDesiredResource {
        row("dev", "Zone/dev", serde_json::json!({}))
    }

    // -- factory -------------------------------------------------------------

    #[tokio::test]
    async fn factory_registers_both_telemetry_types() {
        let factory = TelemetryDriverFactory::new();
        let types = factory
            .resource_types()
            .iter()
            .map(ResourceTypeName::as_str)
            .collect::<Vec<_>>();
        assert_eq!(types.len(), 2);
        assert!(types.contains(&TELEMETRY_SERVICE_TYPE), "{types:?}");
        assert!(types.contains(&TELEMETRY_BINDING_TYPE), "{types:?}");
    }

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

    #[tokio::test]
    async fn validate_accepts_a_provider_declared_spec() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        let mut driver = driver(&fixture).await;
        driver.validate(&mut fixture.ctx).await.expect("valid spec");
    }

    // -- Binding reconcile ---------------------------------------------------

    #[tokio::test]
    async fn binding_reconcile_ensures_provider_declared_children_then_converges() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        fixture.manager.seed(service_row());
        fixture.manager.seed(target_row());
        let mut driver = driver(&fixture).await;

        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let log = fixture.manager.log();
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
        let processes = fixture.manager.rows_of_type("Process");
        assert_eq!(processes.len(), 1);
        let process_spec = fixture.manager.spec_of(&processes[0]);
        assert_eq!(process_spec["template"], "otel-collector");
        assert_eq!(process_spec["providerRef"], "Provider/system-minijail");
        assert_eq!(process_spec["executionRef"], "Host/host-system");
        assert_eq!(process_spec["processClass"], "service");
        assert_eq!(process_spec["domain"], "system");
        let endpoints = fixture.manager.rows_of_type("Endpoint");
        assert_eq!(endpoints.len(), 1);
        let endpoint_spec = fixture.manager.spec_of(&endpoints[0]);
        assert_eq!(endpoint_spec["providerRef"], TELEMETRY_PROVIDER_REF);
        assert_eq!(
            endpoint_spec["producerRef"],
            format!("Process/{}", processes[0].key.name)
        );
        assert_eq!(endpoint_spec["purpose"], "ingest-endpoint");
        assert_eq!(endpoint_spec["lifecyclePolicy"], "recycle-with-producer");

        let Some(TelemetryStatus::Binding {
            phase,
            fenced,
            converged,
            desired_children,
        }) = fixture.ctx.status::<TelemetryStatus>()
        else {
            panic!("binding status");
        };
        assert_eq!(*phase, PHASE_PENDING, "first pass mutated the child set");
        assert!(!fenced);
        assert!(!converged);
        assert_eq!(desired_children.len(), 2);
        assert_eq!(fixture.requeue.scheduled(), vec![TELEMETRY_RESYNC]);

        // Second pass: the owned child set is current; every ensure is a
        // no-op and no resync is scheduled.
        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let log = fixture.manager.log();
        assert_eq!(log.len(), 4, "both desired children re-ensured: {log:?}");
        assert!(
            log[2].ends_with(":unchanged") && log[3].ends_with(":unchanged"),
            "the second pass mutates nothing: {log:?}"
        );
        let Some(TelemetryStatus::Binding {
            phase, converged, ..
        }) = fixture.ctx.status::<TelemetryStatus>()
        else {
            panic!("binding status");
        };
        assert!(converged, "second pass converged");
        // CONTRACT FLAG: the old `ready ? Ready : Degraded` phase reports the
        // fail-closed projection while readiness is unobservable.
        assert_eq!(*phase, PHASE_DEGRADED);
        assert_eq!(
            fixture.requeue.scheduled(),
            vec![TELEMETRY_RESYNC],
            "converged owners stop rescheduling"
        );
    }

    #[tokio::test]
    async fn binding_reconcile_fences_a_dangling_service_dependency() {
        // The Service/target rows are absent: the old owner fenced, mutated
        // no children, and reported Degraded.
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        let mut driver = driver(&fixture).await;

        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        assert!(fixture.manager.log().is_empty(), "fenced owners mutate nothing");
        assert!(fixture.manager.rows_of_type("Process").is_empty());
        let Some(TelemetryStatus::Binding {
            phase,
            fenced,
            converged,
            ..
        }) = fixture.ctx.status::<TelemetryStatus>()
        else {
            panic!("binding status");
        };
        assert!(fenced);
        assert!(!converged);
        assert_eq!(*phase, PHASE_DEGRADED);
        assert_eq!(
            fixture.requeue.scheduled(),
            vec![TELEMETRY_RESYNC],
            "the preserved resync re-evaluates the fence"
        );
    }

    #[tokio::test]
    async fn binding_reconcile_fences_a_foreign_provider() {
        let mut fixture = fixture(binding_row(binding_spec("Provider/other")));
        fixture.manager.seed(service_row());
        fixture.manager.seed(target_row());
        let mut driver = driver(&fixture).await;

        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert!(
            fixture.manager.log().is_empty(),
            "a foreign Provider never derives children"
        );
        assert!(matches!(
            fixture.ctx.status::<TelemetryStatus>(),
            Some(TelemetryStatus::Binding { fenced: true, .. })
        ));
    }

    #[tokio::test]
    async fn binding_reconcile_retires_obsolete_children_endpoint_first() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        fixture.manager.seed(service_row());
        fixture.manager.seed(target_row());
        // Owned children the current target no longer derives (the old Core
        // owner diff retired these).
        fixture.manager.seed_child(
            "dev",
            "Process/telemetry-forwarder-aaaaaaaa",
            serde_json::json!({"providerRef": "Provider/system-minijail"}),
        );
        fixture.manager.seed_child(
            "dev",
            "Endpoint/telemetry-forwarder-endpoint-bbbbbbbb",
            serde_json::json!({"providerRef": TELEMETRY_PROVIDER_REF}),
        );
        let mut driver = driver(&fixture).await;

        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let deletes = fixture
            .manager
            .log()
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

    #[tokio::test]
    async fn dependency_watches_are_registered_once_per_target() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        fixture.manager.seed(service_row());
        fixture.manager.seed(target_row());
        let mut driver = driver(&fixture).await;

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
        assert!(
            targets.contains(&format!(
                "{}/{}",
                TELEMETRY_SERVICE_TYPE, "ingest"
            )),
            "{targets:?}"
        );
    }

    // -- Binding recover -----------------------------------------------------

    #[tokio::test]
    async fn binding_recover_adopts_only_a_current_owned_child_set() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        fixture.manager.seed(service_row());
        fixture.manager.seed(target_row());
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

    // -- Service reconcile ---------------------------------------------------

    #[tokio::test]
    async fn service_pending_until_declared_endpoints_exist_then_fail_closed() {
        let mut fixture = fixture(service_row());
        let mut driver = driver(&fixture).await;

        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        let Some(TelemetryStatus::Service {
            phase,
            present_endpoints,
            ..
        }) = fixture.ctx.status::<TelemetryStatus>()
        else {
            panic!("service status");
        };
        assert_eq!(*phase, PHASE_PENDING);
        assert!(present_endpoints.is_empty());
        assert_eq!(
            fixture.requeue.scheduled(),
            vec![TELEMETRY_RESYNC],
            "the route is not materialized yet"
        );

        fixture.manager.seed(endpoint_row());
        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let Some(TelemetryStatus::Service {
            phase,
            projection,
            present_endpoints,
        }) = fixture.ctx.status::<TelemetryStatus>()
        else {
            panic!("service status");
        };
        assert_eq!(present_endpoints.len(), 1);
        assert_eq!(projection["serviceRole"], "authority");
        // CONTRACT FLAG: the old predicate also required the ingest
        // Endpoint's own `status.phase == "Ready"`, which this surface cannot
        // read; the phase stays fail-closed Pending while the row exists.
        assert_eq!(*phase, PHASE_PENDING);
        assert_eq!(projection["serviceReadiness"], PHASE_PENDING);
        assert_eq!(
            fixture.requeue.scheduled(),
            vec![TELEMETRY_RESYNC],
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
        let Some(TelemetryStatus::Service {
            phase, projection, ..
        }) = fixture.ctx.status::<TelemetryStatus>()
        else {
            panic!("service status");
        };
        assert_eq!(*phase, PHASE_READY);
        assert_eq!(projection["serviceRole"], "projection");
        assert_eq!(projection["serviceReadiness"], PHASE_READY);
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
        assert!(matches!(
            fixture.ctx.status::<TelemetryStatus>(),
            Some(TelemetryStatus::Service {
                phase: PHASE_DEGRADED,
                ..
            })
        ));
    }

    // -- finalize: owned children retire before the telemetry teardown (F3) ---

    #[tokio::test]
    async fn finalize_finalizes_owned_children_before_the_telemetry_teardown() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        fixture
            .manager
            .seed_child("dev", "Process/collector", serde_json::json!({}));
        let mut driver = driver(&fixture).await;

        // A live owned child: the pass requeues.
        let failure = driver.finalize(&mut fixture.ctx).await.expect_err("owned child still live");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert!(
            fixture
                .manager
                .log()
                .iter()
                .any(|entry| entry == "delete:Process/collector"),
            "the owned child is nudged through its own finalize-before-delete pass"
        );

        // The child row retires: the same pass converges.
        fixture
            .manager
            .drop_row(&ResourceKey::new("dev", "Process", "collector"));
        driver.finalize(&mut fixture.ctx).await.expect("converged once the child retired");
    }

    // -- delete --------------------------------------------------------------

    #[tokio::test]
    async fn delete_runs_no_effect_past_the_manager_cascade() {
        let mut fixture = fixture(binding_row(binding_spec(TELEMETRY_PROVIDER_REF)));
        fixture.manager.seed(service_row());
        fixture.manager.seed(target_row());
        let mut driver = driver(&fixture).await;

        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let before = fixture.manager.log().len();
        driver.delete(&mut fixture.ctx).await.expect("delete");
        assert_eq!(
            fixture.manager.log().len(),
            before,
            "the manager cascades owned children before the driver delete pass"
        );
    }
}
