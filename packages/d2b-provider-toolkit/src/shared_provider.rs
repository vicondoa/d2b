//! The shared host-provider driver machinery.
//!
//! A shared host-provider family serves one or more ResourceTypes whose rows
//! are dispatched by the Provider reference their spec carries, and whose
//! realization is a typed Provider effect behind a family-owned port. The
//! conversion the v3 rewrite performed for the whole shared family (U8) is
//! the same for every family, so it lives here and no two families can
//! diverge on it:
//!
//! - `describe` -> one [`ProviderRow`] per registration the family declares;
//! - `validate_spec` -> [`ResourceDriver::validate`]: the spec decodes and
//!   names a Provider this family owns for the row's ResourceType;
//! - `observe` -> [`ResourceDriver::recover`]: owned-child adoption;
//! - finalizer enrollment plus `plan`/`reconcile`/`execute_effect` ->
//!   [`ResourceDriver::reconcile`]: the desired child set the family declares
//!   is ensured through the manager child API (committed before the child
//!   actor exists, F1), owned children the desired set no longer derives are
//!   retired in the family's preserved order, the typed Provider effect runs
//!   behind the family's port, and the in-memory status projection is
//!   published with `ctx.set_status` (R11) plus a self-`requeue_after` while
//!   the family is not converged;
//! - `prepare_finalize`/`execute_finalize`/`finalize` ->
//!   [`ResourceDriver::delete`]: the family's teardown stage runs behind the
//!   port before the owned children retire.
//!
//! Process work never happens in the driver (KTD13): a family's effect port
//! owns the child-row ensures and the phase gates, and any broker work it
//! still needs is a state operation, not a launch. Child Process and Endpoint
//! resources are ensured as manager rows and launched by the Process/Endpoint
//! drivers, which is why the Device families read their declared
//! `Process/swtpm-<device>` / `Process/gpu-<device>` rows instead of spawning
//! them.
//!
//! Cross-resource readiness (a child's or dependency's live phase) is
//! observable from an effect through [`SharedProviderChildSurface::view`],
//! which reads the manager plane's live view for a row of the driving
//! resource; the driver itself only registers
//! `ctx.watch(.., WatchCondition::Ready)` edges so dependency and child
//! readiness wake it, and it never fabricates a readiness it cannot observe.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
};
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName, StoredDesiredResource};
use d2b_resource_runtime::spec_store::EnsureOutcome;
use serde_json::{Value, json};

/// The canonical Host execution target (the old shared Runner's Host ref).
pub const HOST_REF: &str = "Host/host-system";

// ---------------------------------------------------------------------------
// Family declaration
// ---------------------------------------------------------------------------

/// One ResourceType/Provider row a family declares.
///
/// The row pins the identity every effect call binds: the ResourceType it
/// serves, the family component that selects the handler, the controller
/// reference, the Provider reference, and the repair cadence the old
/// descriptor carried. A row is declaration data; the handler behind it lives
/// in the family crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderRow<C> {
    /// The ResourceType this row serves.
    pub resource_type: &'static str,
    /// The family component that selects this row's handler.
    pub component: C,
    /// The controller reference the row's effects bind.
    pub controller_ref: &'static str,
    /// The Provider reference the row's spec must name.
    pub provider_ref: &'static str,
    /// The stable effect id the row's runtime operation id carries.
    pub effect_id: &'static str,
    /// Preserved resync cadence (old `repair_interval_secs`).
    pub resync: Duration,
}

/// The failure surface one family declaration hook may raise.
///
/// The family knows *what* failed; the driver owns the operation it happened
/// in and turns this into a classified [`SharedProviderDriverError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedProviderDeclarationError {
    /// The durable spec did not decode or names a shape this family refuses:
    /// terminal, retrying cannot change the stored spec.
    SpecInvalid,
    /// A manager child mutation failed (retryable: the manager owns retries).
    ChildMutation,
}

/// The family half of a shared host-provider driver.
///
/// The driver owns the preserved flow; the family owns everything that is its
/// own knowledge: the rows it declares, the children it declares, the
/// dependency references it reads, and the typed Provider effect behind its
/// port. `Component` is the family's own closed component vocabulary, so a
/// row's handler is selected by a declaration rather than by a type-name
/// heuristic.
#[async_trait]
pub trait SharedProviderFamily: Send + Sync + 'static {
    /// The family's closed component vocabulary.
    type Component: Copy + core::fmt::Debug + Eq + Send + Sync + 'static;
    /// The per-resource Provider state the family's effects keep.
    ///
    /// One instance per driver instance (one resource, R6); never persisted:
    /// after a restart the family's controllers rehydrate from fresh evidence
    /// exactly as the old in-memory maps did.
    type State: Default + Send + Sync + 'static;

    /// The rows this family declares, in the preserved registration order.
    fn rows(&self) -> &'static [ProviderRow<Self::Component>];

    /// The desired child set of one reconcile pass, as manager child rows.
    ///
    /// `None` marks a component whose children are not this driver's to
    /// declare *or diff*: the Device/Service components own rows another
    /// layer declares - the Zone bundle declares the Device-owned worker rows
    /// (`Process/swtpm-<device>`, `Process/gpu-<device>`, KTD13) with `Nix`
    /// provenance, and the family's Provider effect ensures its own
    /// controller-created rows through the child surface (the TPM state
    /// Volume). Running the obsolete-children diff against an empty desired
    /// set would retire every declared row on the first reconcile pass - the
    /// bundle ingest would look like it never happened. Those components
    /// retire their whole owned subtree on teardown instead.
    async fn desired_children(
        &self,
        ctx: &mut ResourceContext,
        component: Self::Component,
        spec: &Value,
    ) -> Result<Option<Vec<ChildEnsure>>, SharedProviderDeclarationError>;

    /// The dependency references one row declares, in the order the family's
    /// selectors carry them (R12/R17 watch targets).
    fn declared_dependency_refs(
        &self,
        component: Self::Component,
        spec: &Value,
        metadata: &Value,
    ) -> Vec<ResourceRef>;

    /// Run one row's typed Provider effect.
    async fn effect(
        &self,
        component: Self::Component,
        request: &SharedProviderEffectRequest<'_>,
        state: &Self::State,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Advance one row's Provider teardown stage (old `execute_finalize`).
    async fn finalize(
        &self,
        component: Self::Component,
        request: &SharedProviderEffectRequest<'_>,
        state: &Self::State,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError>;
}

// ---------------------------------------------------------------------------
// Typed Provider effect boundary (old `SharedProviderEffectExecutor` rows)
// ---------------------------------------------------------------------------

/// Result returned by one typed Provider effect adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedProviderEffectPhase {
    /// The Provider converged.
    Ready,
    /// The Provider path is still progressing.
    Pending,
}

/// One Provider effect outcome: the phase the old effect returned plus the
/// `status.resource` projection the old status candidate published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedProviderEffectOutcome {
    /// The phase the effect returned.
    pub phase: SharedProviderEffectPhase,
    /// The `status.resource` projection the old status candidate published.
    pub resource_projection: Option<Value>,
}

impl SharedProviderEffectOutcome {
    /// A phase-only outcome.
    pub const fn phase(phase: SharedProviderEffectPhase) -> Self {
        Self {
            phase,
            resource_projection: None,
        }
    }

    /// A phase outcome carrying one `status.resource` projection.
    pub fn projection(phase: SharedProviderEffectPhase, resource_projection: Value) -> Self {
        Self {
            phase,
            resource_projection: Some(resource_projection),
        }
    }
}

/// Outcome of one Provider teardown stage (old `execute_finalize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedProviderFinalize {
    /// Cleanup finished; the owned children may retire.
    Complete,
    /// Cleanup is progressing; the owner is re-entered (old `Pending`).
    Pending,
}

/// Closed failure surface for shared Provider adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedProviderEffectError {
    /// Cleanup is progressing and the owner should be re-entered.
    ///
    /// The converted Device Providers report teardown progress through
    /// [`SharedProviderFinalize::Pending`] instead, so no port constructs
    /// this variant today; the effect-error arm that maps it onto
    /// [`SharedProviderDriverErrorKind::FinalizePending`] stays closed for
    /// the ports still being converted.
    Pending,
    /// The Provider path is not currently available and should retry.
    Unavailable,
    /// Fresh resource or assignment evidence failed closed.
    InvalidResource,
}

impl core::fmt::Display for SharedProviderEffectError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Pending => "shared-provider-effect-pending",
            Self::Unavailable => "shared-provider-effect-unavailable",
            Self::InvalidResource => "shared-provider-resource-invalid",
        })
    }
}

impl std::error::Error for SharedProviderEffectError {}

// ---------------------------------------------------------------------------
// Driver error classification
// ---------------------------------------------------------------------------

/// The closed failure kinds one shared host-provider driver classifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedProviderDriverErrorKind {
    /// The durable spec did not decode or names a Provider outside the row's
    /// ResourceType: terminal, retrying cannot change the stored spec.
    SpecInvalid,
    /// A manager child mutation failed (retryable: the manager owns retries).
    ChildMutation,
    /// The provider path is temporarily unavailable.
    ProviderUnavailable,
    /// A Provider teardown stage is still progressing.
    FinalizePending,
}

impl SharedProviderDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::SpecInvalid => FailureClass::Terminal,
            Self::ChildMutation | Self::ProviderUnavailable | Self::FinalizePending => {
                FailureClass::Retryable
            }
        }
    }

    const fn code(self) -> &'static str {
        match self {
            Self::SpecInvalid => "shared-provider-spec-invalid",
            Self::ChildMutation => "shared-provider-child-mutation",
            Self::ProviderUnavailable => "shared-provider-unavailable",
            Self::FinalizePending => "shared-provider-finalize-pending",
        }
    }
}

/// One classified shared host-provider driver failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedProviderDriverError {
    kind: SharedProviderDriverErrorKind,
    op: DriverOp,
}

impl SharedProviderDriverError {
    const fn new(kind: SharedProviderDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }
}

impl core::fmt::Display for SharedProviderDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.kind.code())
    }
}

impl std::error::Error for SharedProviderDriverError {}

/// Typed in-memory status projection (R11: never persisted). Carries the
/// closed phase the old status candidate published plus the Provider's
/// `status.resource` projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedProviderDriverStatus {
    /// The phase the effect published.
    pub phase: SharedProviderEffectPhase,
    /// The Provider's `status.resource` projection.
    pub resource: Option<Value>,
}

impl SharedProviderDriverStatus {
    /// The closed phase string this status was built from.
    pub const fn phase(&self) -> &'static str {
        match self.phase {
            SharedProviderEffectPhase::Ready => "Ready",
            SharedProviderEffectPhase::Pending => "Pending",
        }
    }
}

// ---------------------------------------------------------------------------
// Spec decode (manager-wired)
// ---------------------------------------------------------------------------

/// Decoded shared-provider spec envelope: the exact stored spec bytes and the
/// canonical spec document the family's Provider handlers read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedProviderSpecEnvelope {
    /// The exact stored spec bytes; never rewritten by this driver.
    raw: Vec<u8>,
    value: Value,
}

impl SharedProviderSpecEnvelope {
    /// The canonical spec document of the row.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The exact stored spec bytes. The family's handlers read [`Self::value`]
    /// and the driver never rewrites the stored envelope.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The spec's Provider reference (the family row selector).
    pub fn provider_ref(&self) -> Option<&str> {
        self.value.get("providerRef").and_then(Value::as_str)
    }
}

/// Closed decode error for a shared-provider spec envelope.
#[derive(Debug, thiserror::Error)]
#[error("shared provider spec must be a JSON object")]
pub struct SharedProviderSpecDecodeError;

/// The manager-wired decode hook for a shared host-provider family's
/// ResourceTypes.
pub fn shared_provider_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        let value =
            serde_json::from_slice::<Value>(bytes).map_err(|_| SharedProviderSpecDecodeError)?;
        if !value.is_object() {
            return Err(SharedProviderSpecDecodeError);
        }
        Ok(SharedProviderSpecEnvelope {
            raw: bytes.to_vec(),
            value,
        })
    })
}

/// Decode one row's metadata envelope (empty metadata is the empty object).
pub fn decode_metadata(raw: &[u8]) -> Result<Value, SharedProviderEffectError> {
    if raw.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice::<Value>(raw).map_err(|_| SharedProviderEffectError::InvalidResource)
}

/// The owner reference the old effects read from `/metadata/ownerRef`.
pub fn owner_ref(metadata: &Value) -> Result<ResourceRef, SharedProviderEffectError> {
    metadata
        .get("ownerRef")
        .and_then(Value::as_str)
        .and_then(|value| ResourceRef::parse(value).ok())
        .ok_or(SharedProviderEffectError::InvalidResource)
}

/// Convert one durable 16-byte uid to its canonical identity (the manager
/// persists the uid as bytes; the Provider effects key on the canonical
/// string).
pub fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, SharedProviderEffectError> {
    ResourceUid::from_bytes(bytes).map_err(|_| SharedProviderEffectError::InvalidResource)
}

/// The resource reference of one manager key.
pub fn key_ref(key: &ResourceKey) -> Result<ResourceRef, SharedProviderEffectError> {
    ResourceRef::parse(&format!("{}/{}", key.type_name, key.name))
        .map_err(|_| SharedProviderEffectError::InvalidResource)
}

// ---------------------------------------------------------------------------
// Manager-routed child surface
// ---------------------------------------------------------------------------

/// Manager-routed child mutations handed to one Provider effect call.
///
/// The driver owns this surface (it is the only holder of the resource's
/// [`ResourceContext`]); the Provider effects use it to ensure and delete the
/// child resources they declare. Every ensure rides
/// [`ResourceContext::ensure_child`], so the manager commits the child row
/// BEFORE the child actor exists (F1, AE1).
#[async_trait]
pub trait SharedProviderChildSurface: Send + Sync {
    /// Create or update one child row through the manager.
    async fn ensure(&self, child: ChildEnsure) -> Result<EnsureOutcome, SharedProviderEffectError>;
    /// Delete one owned child row through the manager (idempotent).
    async fn delete(&self, key: &ResourceKey) -> Result<(), SharedProviderEffectError>;
    /// The live manager view of one child row (absent when no row exists).
    ///
    /// The row-owned launch path gates on this: a Provider effect ensures the
    /// declared Process/EphemeralProcess/Endpoint child and reads its
    /// published phase here instead of holding a raw broker handle.
    async fn view(
        &self,
        key: &ResourceKey,
    ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, SharedProviderEffectError>;
}

/// [`SharedProviderChildSurface`] over the calling resource's context.
///
/// The context is held behind an async mutex because every Provider
/// controller drives its child mutations through `&self` ports while the
/// driver keeps the only `&mut ResourceContext`; the guard is never held
/// across another effect call, so the single-threaded drive order is
/// preserved.
pub struct ContextChildSurface<'a> {
    ctx: tokio::sync::Mutex<&'a mut ResourceContext>,
}

impl<'a> ContextChildSurface<'a> {
    /// Build the surface over one resource context.
    pub fn new(ctx: &'a mut ResourceContext) -> Self {
        Self {
            ctx: tokio::sync::Mutex::new(ctx),
        }
    }
}

#[async_trait]
impl SharedProviderChildSurface for ContextChildSurface<'_> {
    async fn ensure(&self, child: ChildEnsure) -> Result<EnsureOutcome, SharedProviderEffectError> {
        let mut ctx = self.ctx.lock().await;
        ctx.ensure_child(child)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)
    }

    async fn delete(&self, key: &ResourceKey) -> Result<(), SharedProviderEffectError> {
        let mut ctx = self.ctx.lock().await;
        if ctx
            .get(key)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?
            .is_none()
        {
            return Ok(());
        }
        ctx.delete(key)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)
    }

    async fn view(
        &self,
        key: &ResourceKey,
    ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, SharedProviderEffectError> {
        let mut ctx = self.ctx.lock().await;
        ctx.get_view(key)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)
    }
}

// ---------------------------------------------------------------------------
// Effect request
// ---------------------------------------------------------------------------

/// Everything one Provider effect call may read from the driver.
pub struct SharedProviderEffectRequest<'a> {
    /// The zone the row lives in.
    pub zone: ZoneId,
    /// The row's manager key.
    pub target: ResourceKey,
    /// The row's durable uid (the Provider effects key on it).
    pub uid: ResourceUid,
    /// The row's current generation.
    pub generation: ResourceGeneration,
    /// Runtime-only operation id (never persisted).
    pub operation_id: String,
    /// Canonical spec document of the row.
    pub spec: Value,
    /// Decoded metadata envelope of the row (`ownerRef`, ...).
    pub metadata: Value,
    /// The driver's last in-memory status projection, when one was published.
    pub status: Option<Value>,
    /// Manager-routed child mutation surface.
    pub children: &'a dyn SharedProviderChildSurface,
}

impl SharedProviderEffectRequest<'_> {
    /// The owner reference the old effects read from `/metadata/ownerRef`.
    pub fn owner_ref(&self) -> Result<ResourceRef, SharedProviderEffectError> {
        owner_ref(&self.metadata)
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Construction arguments shared by every driver of one shared family.
pub struct SharedProviderDriverArgs<C: 'static, S: 'static> {
    /// The zone the family's rows live in.
    pub zone: ZoneId,
    /// The controller generation every effect call binds (KTD7).
    pub controller_generation: ControllerGeneration,
    /// The family's declarations and typed Provider effect.
    pub family: Arc<dyn SharedProviderFamily<Component = C, State = S>>,
}

impl<C: 'static, S: 'static> Clone for SharedProviderDriverArgs<C, S> {
    fn clone(&self) -> Self {
        Self {
            zone: self.zone.clone(),
            controller_generation: self.controller_generation,
            family: Arc::clone(&self.family),
        }
    }
}

/// Factory for one shared host-provider family's ResourceTypes.
///
/// Construction is infallible by contract (R3).
pub struct SharedProviderDriverFactory<C: 'static, S: 'static> {
    types: Vec<ResourceTypeName>,
    args: SharedProviderDriverArgs<C, S>,
}

impl<C: Copy + core::fmt::Debug + Eq + Send + Sync + 'static, S: Default + Send + Sync + 'static>
    SharedProviderDriverFactory<C, S>
{
    /// Build a factory over one family's declared rows.
    ///
    /// A family declares one row per (ResourceType, Provider) pair, so the
    /// type list is the rows' types in declaration order, deduplicated: the
    /// registry keys one driver per ResourceType, and the Device family's four
    /// rows all serve the one `Device` type.
    pub fn new(args: SharedProviderDriverArgs<C, S>) -> Self {
        let mut types: Vec<ResourceTypeName> = Vec::new();
        for row in args.family.rows() {
            let resource_type = ResourceTypeName::new(row.resource_type);
            if !types.contains(&resource_type) {
                types.push(resource_type);
            }
        }
        Self { types, args }
    }
}

#[async_trait]
impl<C: Copy + core::fmt::Debug + Eq + Send + Sync + 'static, S: Default + Send + Sync + 'static>
    ResourceDriverFactory for SharedProviderDriverFactory<C, S>
{
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(SharedProviderDriver::new(self.args.clone()))
    }
}

/// One desired shared host-provider resource.
pub struct SharedProviderDriver<C: 'static, S: 'static> {
    zone: ZoneId,
    /// The controller generation every effect call binds (KTD7).
    controller_generation: ControllerGeneration,
    family: Arc<dyn SharedProviderFamily<Component = C, State = S>>,
    state: Arc<S>,
    /// Dependency/child keys already watched (R12: exactly once per target).
    watched: Vec<ResourceKey>,
}

impl<C: Copy + core::fmt::Debug + Eq + Send + Sync + 'static, S: Default + Send + Sync + 'static>
    SharedProviderDriver<C, S>
{
    fn new(args: SharedProviderDriverArgs<C, S>) -> Self {
        Self {
            zone: args.zone,
            controller_generation: args.controller_generation,
            family: args.family,
            state: Arc::new(S::default()),
            watched: Vec::new(),
        }
    }

    /// The controller generation every effect call binds (KTD7).
    pub fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }

    fn error(
        &self,
        kind: SharedProviderDriverErrorKind,
        op: DriverOp,
    ) -> SharedProviderDriverError {
        SharedProviderDriverError::new(kind, op)
    }

    /// The decoded spec envelope of the row being driven.
    fn envelope(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<SharedProviderSpecEnvelope, SharedProviderDriverError> {
        ctx.spec::<SharedProviderSpecEnvelope>()
            .cloned()
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))
    }

    /// Decoded metadata envelope of the row being driven.
    fn metadata(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<Value, SharedProviderDriverError> {
        decode_metadata(ctx.metadata())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))
    }

    /// The family row this row is served by: ResourceType plus the spec's
    /// Provider reference (old `from_registration`).
    ///
    /// The old Runner selected the row by its registration tuple; the new
    /// plane knows only the key, so the row's Provider reference - carried by
    /// the spec and fenced by `validate` - selects the Provider handler. A
    /// Provider this family does not own is refused, never guessed.
    fn row(
        &self,
        ctx: &ResourceContext,
        envelope: &SharedProviderSpecEnvelope,
        op: DriverOp,
    ) -> Result<&'static ProviderRow<C>, SharedProviderDriverError> {
        if ctx.key().zone != self.zone.as_str() {
            return Err(self.error(SharedProviderDriverErrorKind::SpecInvalid, op));
        }
        let Some(provider_ref) = envelope.provider_ref() else {
            return Err(self.error(SharedProviderDriverErrorKind::SpecInvalid, op));
        };
        self.family
            .rows()
            .iter()
            .find(|row| {
                row.resource_type == ctx.key().type_name.as_str() && row.provider_ref == provider_ref
            })
            .ok_or_else(|| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))
    }

    /// Register one internal dependency watch exactly once per target (R12).
    ///
    /// Best-effort by design: a dependency that is still an unconverted row
    /// has no actor to watch yet, and the resync schedule re-evaluates it.
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) || target == *ctx.key() {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    /// The runtime-only operation id of one pass (never persisted).
    fn operation_id(&self, ctx: &ResourceContext, row: &ProviderRow<C>) -> String {
        format!(
            "shared-{}-{}-g{}",
            row.effect_id,
            ctx.uid()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            ctx.generation(),
        )
    }

    fn child_key(&self, target: &ResourceRef) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        )
    }

    /// The desired child set of one reconcile pass, as manager child rows.
    ///
    /// Providers declare the children they own; the family's declaration
    /// materializes them into the manager's child shape (Core-owned bodies,
    /// F1).
    async fn desired_children(
        &self,
        ctx: &mut ResourceContext,
        component: C,
        envelope: &SharedProviderSpecEnvelope,
    ) -> Result<Option<Vec<ChildEnsure>>, SharedProviderDriverError> {
        self.family
            .desired_children(ctx, component, envelope.value())
            .await
            .map_err(|kind| {
                self.error(
                    match kind {
                        SharedProviderDeclarationError::SpecInvalid => {
                            SharedProviderDriverErrorKind::SpecInvalid
                        }
                        SharedProviderDeclarationError::ChildMutation => {
                            SharedProviderDriverErrorKind::ChildMutation
                        }
                    },
                    DriverOp::Reconcile,
                )
            })
    }

    /// Retire owned children the desired set no longer derives, in the
    /// family's preserved order (endpoint-first, process-last; R9/F3).
    async fn retire_obsolete_children(
        &self,
        ctx: &mut ResourceContext,
        desired: &[ChildEnsure],
        op: DriverOp,
    ) -> Result<bool, SharedProviderDriverError> {
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(SharedProviderDriverErrorKind::ChildMutation, op))?;
        let mut obsolete = owned
            .iter()
            .filter(|row| {
                !row.deleting
                    && !desired
                        .iter()
                        .any(|child| owned_child_matches_child_ensure(row, child))
            })
            .collect::<Vec<_>>();
        obsolete.sort_by(|a, b| {
            teardown_rank(&a.key.type_name)
                .cmp(&teardown_rank(&b.key.type_name))
                .then_with(|| a.key.name.cmp(&b.key.name))
        });
        let mut mutated = false;
        for row in obsolete {
            ctx.delete(&row.key)
                .await
                .map_err(|_| self.error(SharedProviderDriverErrorKind::ChildMutation, op))?;
            mutated = true;
        }
        Ok(mutated)
    }

    fn effect_error(
        &self,
        error: SharedProviderEffectError,
        op: DriverOp,
    ) -> SharedProviderDriverError {
        match error {
            SharedProviderEffectError::InvalidResource => {
                self.error(SharedProviderDriverErrorKind::SpecInvalid, op)
            }
            SharedProviderEffectError::Pending => {
                self.error(SharedProviderDriverErrorKind::FinalizePending, op)
            }
            SharedProviderEffectError::Unavailable => {
                self.error(SharedProviderDriverErrorKind::ProviderUnavailable, op)
            }
        }
    }
}

impl<C: 'static, S: 'static> core::fmt::Debug for SharedProviderDriver<C, S> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SharedProviderDriver")
            .field("zone", &self.zone)
            .finish_non_exhaustive()
    }
}

impl<C: 'static, S: 'static> core::fmt::Debug for SharedProviderDriverFactory<C, S> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SharedProviderDriverFactory")
            .field("types", &self.types)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<C: Copy + core::fmt::Debug + Eq + Send + Sync + 'static, S: Default + Send + Sync + 'static>
    ResourceDriver for SharedProviderDriver<C, S>
{
    type Error = SharedProviderDriverError;

    fn classify_error(&self, error: &SharedProviderDriverError) -> DriverFailure {
        match error.kind.class() {
            FailureClass::Retryable => DriverFailure::retryable(error.op),
            FailureClass::Terminal => DriverFailure::terminal(error.op),
        }
    }

    /// Structural validation (old `validate_spec`): the stored spec decodes
    /// and names a Provider this family owns for the row's ResourceType.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let envelope = self.envelope(ctx, DriverOp::Validate)?;
        let _ = self.row(ctx, &envelope, DriverOp::Validate)?;
        Ok(())
    }

    /// Discovery and adoption on the realization target (F2): the family's
    /// child-bearing components adopt when their complete desired child set
    /// is already present and current; components that realize nothing
    /// through resource rows adopt their Provider-side realization in
    /// reconcile.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let envelope = self.envelope(ctx, op)?;
        let row = *self.row(ctx, &envelope, op)?;
        let Some(desired) = self.desired_children(ctx, row.component, &envelope).await? else {
            // A component with no driver-declared children has nothing to
            // adopt here; its Provider-side realization is discovered in the
            // effect.
            return Ok(RecoveryOutcome::Adopted);
        };
        if desired.is_empty() {
            return Ok(RecoveryOutcome::Adopted);
        }
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(SharedProviderDriverErrorKind::ChildMutation, op))?;
        let current = desired.iter().all(|child| {
            owned
                .iter()
                .any(|row| owned_child_matches_child_ensure(row, child) && !row.deleting)
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
        let row = *self.row(ctx, &envelope, op)?;
        let metadata = self.metadata(ctx, op)?;

        // Dependency edges (R12/R17): the resources this family's effects
        // read are watched so their readiness or death wakes this actor.
        for dependency in
            self.family
                .declared_dependency_refs(row.component, envelope.value(), &metadata)
        {
            self.watch_once(ctx, self.child_key(&dependency)).await;
        }

        // Desired child set through the manager child API (F1): every row is
        // committed before its actor exists. Components whose children
        // another layer declares (the bundle's Device worker rows, the
        // effect's controller-created rows) have no desired set here and skip
        // the child machinery entirely - diffing their owned rows against an
        // empty set would retire the declared rows on every pass.
        let desired = self.desired_children(ctx, row.component, &envelope).await?;
        let mut mutated = false;
        if let Some(desired) = &desired {
            for child in desired {
                match ctx.ensure_child(child.clone()).await {
                    Ok(EnsureOutcome::Created(_)) | Ok(EnsureOutcome::Updated(_)) => mutated = true,
                    Ok(EnsureOutcome::Unchanged(_)) => {}
                    Err(_) => {
                        return Err(self.error(SharedProviderDriverErrorKind::ChildMutation, op));
                    }
                }
            }
            mutated |= self.retire_obsolete_children(ctx, desired, op).await?;
            for child in desired {
                self.watch_once(
                    ctx,
                    ResourceKey::new(
                        self.zone.as_str(),
                        child.type_name.as_str(),
                        child.name.as_str(),
                    ),
                )
                .await;
            }
        }

        let operation_id = self.operation_id(ctx, &row);
        let target = ctx.key().clone();
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
        let generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
        let status = ctx
            .status::<SharedProviderDriverStatus>()
            .and_then(|status| status.resource.clone());
        // The surface borrows the context mutably for the effect call; the
        // driver reads nothing else from the context until it is dropped.
        let outcome = {
            let surface = ContextChildSurface::new(ctx);
            let request = SharedProviderEffectRequest {
                zone: self.zone.clone(),
                target,
                uid,
                generation,
                operation_id,
                spec: envelope.value().clone(),
                metadata,
                status,
                children: &surface,
            };
            self.family
                .effect(row.component, &request, &self.state)
                .await
                .map_err(|error| self.effect_error(error, DriverOp::Reconcile))?
        };

        ctx.set_status(SharedProviderDriverStatus {
            phase: outcome.phase,
            resource: outcome.resource_projection.clone(),
        });
        // A pass that did not realize the row must say so. `Satisfied` is the
        // runtime's own "the desired state is realized, the actor may go
        // ready" verdict, and it is what publishes `Ready` on the row; a family
        // that reports `Pending` - a dependency still coming up, a child that
        // has not converged - is not realized, and answering `Satisfied` would
        // wake every watcher on this row's readiness while nothing backs the
        // claim. The requeue is scheduled either way, so the row is always
        // re-driven: a deferral re-checks, a settled row that mutated its
        // children re-converges.
        let deferred = outcome.phase != SharedProviderEffectPhase::Ready;
        if deferred || mutated {
            ctx.requeue_after(row.resync);
        }
        // `Satisfied` is this driver's convergence - the child set is committed
        // and the pass did its work - and it must stay that verdict. The
        // families this driver serves own children whose realization chains
        // through the parent row's readiness, so answering `RetryScheduled`
        // (which publishes `Pending`) while the effect reports `Pending` starves
        // them: nothing converges. The honest not-ready answer is the status
        // projection this pass just published, which is what dependents read.
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown, and before the Provider's teardown stage in
    /// [`ResourceDriver::delete`]. The call nudges each owned child through
    /// its own finalize-before-delete pass and requeues this pass while any
    /// child row is still live. Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources().await.map_err(|_| {
            self.error(
                SharedProviderDriverErrorKind::FinalizePending,
                DriverOp::Delete,
            )
        })?;
        Ok(())
    }

    /// Teardown (old `prepare_finalize` + `execute_finalize` + `finalize`):
    /// the Provider's teardown stage runs first, then the owned children
    /// retire in the family's preserved order. Idempotent under retry (R10).
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        let Ok(envelope) = self.envelope(ctx, op) else {
            return Ok(());
        };
        let row = *self.row(ctx, &envelope, op)?;
        let metadata = self.metadata(ctx, op)?;
        let operation_id = self.operation_id(ctx, &row);
        let target = ctx.key().clone();
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
        let generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
        let status = ctx
            .status::<SharedProviderDriverStatus>()
            .and_then(|status| status.resource.clone());
        let finalized = {
            let surface = ContextChildSurface::new(ctx);
            let request = SharedProviderEffectRequest {
                zone: self.zone.clone(),
                target,
                uid,
                generation,
                operation_id,
                spec: envelope.value().clone(),
                metadata,
                status,
                children: &surface,
            };
            self.family
                .finalize(row.component, &request, &self.state)
                .await
                .map_err(|error| self.effect_error(error, op))?
        };
        if finalized == SharedProviderFinalize::Pending {
            return Err(self.error(SharedProviderDriverErrorKind::FinalizePending, op));
        }
        self.retire_obsolete_children(ctx, &[], op).await?;
        Ok(())
    }
}

/// The child row key a `ChildEnsure` derives (manager identity rules).
fn owned_child_matches_child_ensure(row: &StoredDesiredResource, child: &ChildEnsure) -> bool {
    row.key.type_name == child.type_name.as_str() && row.key.name == child.name
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ZoneId};
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, SpecDecoder,
        WatchId, WatchRegistration,
    };
    use d2b_resource_runtime::driver::{DynResourceDriver, RecoveryOutcome, ResourceDriverFactory};
    use d2b_resource_runtime::error::ResourceError;
    use d2b_resource_runtime::identity::{
        ResourceKey, ResourceProvenance, ResourceTypeName, StoredDesiredResource,
    };
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use serde_json::json;

    use super::{
        ProviderRow, SharedProviderDeclarationError,
        SharedProviderDriverArgs, SharedProviderDriverFactory, SharedProviderDriverStatus,
        SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectPhase,
        SharedProviderEffectRequest, SharedProviderFamily, SharedProviderFinalize,
        shared_provider_spec_decoder,
    };
    use d2b_resource_runtime::driver::ReconcileOutcome;

    /// Ordered log every fake writes to, so ordering is one assertion.
    type Log = Arc<tokio::sync::Mutex<Vec<String>>>;

    /// The fixture's closed component vocabulary.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Component {
        /// A component that declares a child set.
        One,
        /// A component that declares none.
        Two,
    }

    impl Component {
        const fn effect_id(self) -> &'static str {
            match self {
                Self::One => "one",
                Self::Two => "two",
            }
        }
    }

    const ROWS: [ProviderRow<Component>; 2] = [
        ProviderRow {
            resource_type: "FixtureOne",
            component: Component::One,
            controller_ref: "Process/fixture-one-controller",
            provider_ref: "Provider/fixture-one",
            effect_id: "fixture-one",
            resync: Duration::from_secs(30),
        },
        ProviderRow {
            resource_type: "FixtureTwo",
            component: Component::Two,
            controller_ref: "Process/fixture-two-controller",
            provider_ref: "Provider/fixture-two",
            effect_id: "fixture-two",
            resync: Duration::from_secs(30),
        },
    ];

    struct RecordingManager {
        log: Log,
        owned: tokio::sync::Mutex<Vec<StoredDesiredResource>>,
    }

    impl RecordingManager {
        fn new(log: Log) -> Arc<Self> {
            Arc::new(Self {
                log,
                owned: tokio::sync::Mutex::new(Vec::new()),
            })
        }

        fn with_owned(log: Log, owned: Vec<StoredDesiredResource>) -> Arc<Self> {
            Arc::new(Self {
                log,
                owned: tokio::sync::Mutex::new(owned),
            })
        }
    }

    #[async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            self.log
                .lock()
                .await
                .push(format!("ensure:{}/{}", child.type_name.as_str(), child.name));
            Ok(EnsureOutcome::Created(test_row(
                "dev",
                child.type_name.as_str(),
                &child.name,
            )))
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self
                .owned
                .lock()
                .await
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
                .await
                .push(format!("delete:{}/{}", key.type_name, key.name));
            self.owned.lock().await.retain(|row| row.key != *key);
            Ok(())
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self.owned.lock().await.clone())
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

    #[derive(Default)]
    struct RecordingRequeue {
        scheduled: tokio::sync::Mutex<Vec<RequeueId>>,
    }

    impl RequeueScheduler for RecordingRequeue {
        fn schedule(&self, _key: ResourceKey, _after: Duration) -> RequeueId {
            // The scheduler trait is synchronous; the recording side fails
            // closed on the brief write race, which cannot happen in the
            // single-threaded tests this double serves.
            let mut scheduled = self.scheduled.try_lock().expect("scheduler lock");
            let id = RequeueId(scheduled.len() as u64 + 1);
            scheduled.push(id);
            id
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    struct RecordingFamily {
        log: Log,
        phase: tokio::sync::Mutex<SharedProviderEffectPhase>,
        finalize: tokio::sync::Mutex<SharedProviderFinalize>,
        declares_children: bool,
    }

    impl RecordingFamily {
        fn new(
            log: Log,
            phase: SharedProviderEffectPhase,
            finalize: SharedProviderFinalize,
            declares_children: bool,
        ) -> Arc<Self> {
            Arc::new(Self {
                log,
                phase: tokio::sync::Mutex::new(phase),
                finalize: tokio::sync::Mutex::new(finalize),
                declares_children,
            })
        }
    }

    #[async_trait]
    impl SharedProviderFamily for RecordingFamily {
        type Component = Component;
        type State = ();

        fn rows(&self) -> &'static [ProviderRow<Self::Component>] {
            &ROWS
        }

        async fn desired_children(
            &self,
            _ctx: &mut ResourceContext,
            component: Component,
            _spec: &serde_json::Value,
        ) -> Result<Option<Vec<ChildEnsure>>, SharedProviderDeclarationError> {
            if component == Component::Two && !self.declares_children {
                return Ok(None);
            }
            Ok(Some(vec![ChildEnsure {
                type_name: ResourceTypeName::new("Volume"),
                name: "fixture-child".to_owned(),
                spec: b"{}".to_vec(),
                metadata: Vec::new(),
            }]))
        }

        fn declared_dependency_refs(
            &self,
            _component: Component,
            _spec: &serde_json::Value,
            _metadata: &serde_json::Value,
        ) -> Vec<ResourceRef> {
            Vec::new()
        }

        async fn effect(
            &self,
            component: Component,
            _request: &SharedProviderEffectRequest<'_>,
            _state: &(),
        ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
            self.log
                .lock()
                .await
                .push(format!("effect:{}", component.effect_id()));
            Ok(SharedProviderEffectOutcome::phase(*self.phase.lock().await))
        }

        async fn finalize(
            &self,
            component: Component,
            _request: &SharedProviderEffectRequest<'_>,
            _state: &(),
        ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
            self.log
                .lock()
                .await
                .push(format!("finalize:{}", component.effect_id()));
            Ok(*self.finalize.lock().await)
        }
    }

    fn test_row(zone: &str, type_name: &str, name: &str) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new(zone, type_name, name),
            uid: [0x42; 16],
            generation: 3,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: b"spec".to_vec(),
            metadata: Vec::new(),
            created_at: 1_725_000_000,
        }
    }

    fn owned_row(zone: &str, type_name: &str, name: &str) -> StoredDesiredResource {
        StoredDesiredResource {
            owner_uid: Some([0x42; 16]),
            ..test_row(zone, type_name, name)
        }
    }

    fn decoder() -> Arc<dyn SpecDecoder> {
        shared_provider_spec_decoder()
    }

    struct Fixture {
        ctx: ResourceContext,
        family: Arc<RecordingFamily>,
        requeue: Arc<RecordingRequeue>,
        log: Log,
    }

    #[allow(clippy::too_many_arguments)]
    fn fixture(
        type_name: &str,
        name: &str,
        spec: serde_json::Value,
        manager: Arc<RecordingManager>,
        requeue: Arc<RecordingRequeue>,
        log: Log,
        phase: SharedProviderEffectPhase,
        finalize: SharedProviderFinalize,
        declares_children: bool,
    ) -> Fixture {
        let mut row = test_row("dev", type_name, name);
        row.spec = serde_json::to_vec(&spec).expect("spec json");
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let family = RecordingFamily::new(Arc::clone(&log), phase, finalize, declares_children);
        let ctx = ResourceContext::new(
            row,
            decoder(),
            Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            Arc::clone(&requeue) as Arc<dyn RequeueScheduler>,
            effects_tx,
            notify_tx,
        );
        Fixture {
            ctx,
            family,
            requeue,
            log,
        }
    }

    async fn driver(fixture: &Fixture) -> Box<dyn DynResourceDriver> {
        let family: Arc<dyn SharedProviderFamily<Component = Component, State = ()>> =
            fixture.family.clone();
        let factory = SharedProviderDriverFactory::new(SharedProviderDriverArgs {
            zone: ZoneId::parse("dev").expect("valid test zone"),
            controller_generation: ControllerGeneration::new(1).expect("generation"),
            family,
        });
        let key = fixture.ctx.key().clone();
        factory.create(&key).await
    }

    /// The factory serves exactly the family's declared rows.
    #[test]
    fn factory_serves_the_declared_rows() {
        let factory = SharedProviderDriverFactory::new(SharedProviderDriverArgs {
            zone: ZoneId::parse("dev").expect("valid test zone"),
            controller_generation: ControllerGeneration::new(1).expect("generation"),
            family: RecordingFamily::new(
                Arc::new(tokio::sync::Mutex::new(Vec::new())),
                SharedProviderEffectPhase::Pending,
                SharedProviderFinalize::Complete,
                false,
            ),
        });
        let types = factory
            .resource_types()
            .iter()
            .map(|resource_type| resource_type.as_str().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(types, vec!["FixtureOne".to_owned(), "FixtureTwo".to_owned()]);
    }

    /// A row naming a Provider outside the family is terminal: the stored
    /// spec can never be served by this factory (old `from_registration`
    /// refusal).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn validate_rejects_a_provider_outside_the_family() {
        let log: Log = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let mut fixture = fixture(
            "FixtureOne",
            "row-a",
            json!({"providerRef": "Provider/elsewhere"}),
            RecordingManager::new(Arc::clone(&log)),
            Arc::new(RecordingRequeue::default()),
            log,
            SharedProviderEffectPhase::Pending,
            SharedProviderFinalize::Complete,
            true,
        );
        let mut driver = driver(&fixture).await;
        let failure = driver
            .validate(&mut fixture.ctx)
            .await
            .expect_err("must refuse");
        assert_eq!(
            failure,
            d2b_resource_runtime::error::DriverFailure::terminal(
                d2b_resource_runtime::error::DriverOp::Validate
            )
        );
    }

    /// The declared child set is committed before the typed effect runs, and
    /// a not-converged reconcile self-requeues with the row's cadence.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn reconcile_commits_the_child_set_then_runs_the_effect() {
        let log: Log = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let mut fixture = fixture(
            "FixtureOne",
            "row-a",
            json!({"providerRef": "Provider/fixture-one"}),
            RecordingManager::new(Arc::clone(&log)),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Pending,
            SharedProviderFinalize::Complete,
            true,
        );
        let mut driver = driver(&fixture).await;
        assert!(driver.validate(&mut fixture.ctx).await.is_ok());
        let outcome = driver
            .reconcile(&mut fixture.ctx)
            .await
            .expect("reconcile");
        assert_eq!(
            outcome,
            ReconcileOutcome::Satisfied,
            "the pass converged its own work; the family's phase rides the status projection the \
             pass published, which is what dependents read"
        );

        let entries = fixture.log.lock().await.clone();
        let child_at = entries
            .iter()
            .position(|entry| entry == "ensure:Volume/fixture-child")
            .expect("child committed");
        let effect_at = entries
            .iter()
            .position(|entry| entry == "effect:one")
            .expect("effect ran");
        assert!(child_at < effect_at, "{entries:?}");
        assert_eq!(
            fixture.requeue.scheduled.lock().await.len(),
            1,
            "pending reconcile self-resyncs"
        );
        let status = fixture
            .ctx
            .status::<SharedProviderDriverStatus>()
            .expect("status published (R11)");
        assert_eq!(status.phase(), "Pending");
    }

    /// A component that declares no child set is not diffed: the declared
    /// rows other layers own must survive a reconcile pass.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn reconcile_leaves_rows_of_a_child_less_component_alone() {
        let log: Log = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let manager = RecordingManager::with_owned(
            Arc::clone(&log),
            vec![owned_row("dev", "Process", "declared-worker")],
        );
        let mut fixture = fixture(
            "FixtureTwo",
            "row-b",
            json!({"providerRef": "Provider/fixture-two"}),
            Arc::clone(&manager),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Complete,
            false,
        );
        let mut driver = driver(&fixture).await;
        driver
            .reconcile(&mut fixture.ctx)
            .await
            .expect("reconcile");
        let entries = log.lock().await.clone();
        assert!(
            !entries.iter().any(|entry| entry.starts_with("delete:")),
            "the declared rows are not this driver's to retire: {entries:?}"
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.as_str() == "effect:two")
                .count(),
            1
        );
    }

    /// Delete runs the Provider teardown stage first, then retires the owned
    /// children endpoint-first.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn delete_runs_provider_teardown_then_retires_children() {
        let log: Log = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let manager = RecordingManager::with_owned(
            Arc::clone(&log),
            vec![
                owned_row("dev", "Process", "stale-proxy"),
                owned_row("dev", "Endpoint", "stale-endpoint"),
            ],
        );
        let mut fixture = fixture(
            "FixtureOne",
            "row-a",
            json!({"providerRef": "Provider/fixture-one"}),
            Arc::clone(&manager),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Complete,
            true,
        );
        let mut driver = driver(&fixture).await;
        driver.delete(&mut fixture.ctx).await.expect("teardown");
        let entries = log.lock().await.clone();
        let finalize_at = entries
            .iter()
            .position(|entry| entry == "finalize:one")
            .expect("teardown ran");
        let endpoint_at = entries
            .iter()
            .position(|entry| entry == "delete:Endpoint/stale-endpoint")
            .expect("endpoint retired");
        let process_at = entries
            .iter()
            .position(|entry| entry == "delete:Process/stale-proxy")
            .expect("process retired");
        assert!(
            finalize_at < endpoint_at && endpoint_at < process_at,
            "{entries:?}"
        );
    }

    /// A Provider teardown stage that is still progressing is retryable: the
    /// actor re-enters delete (R10) instead of reporting completion.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn delete_is_retryable_while_the_provider_stage_is_pending() {
        let log: Log = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let mut fixture = fixture(
            "FixtureTwo",
            "row-b",
            json!({"providerRef": "Provider/fixture-two"}),
            RecordingManager::new(Arc::clone(&log)),
            Arc::new(RecordingRequeue::default()),
            log,
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Pending,
            false,
        );
        let mut driver = driver(&fixture).await;
        let failure = driver
            .delete(&mut fixture.ctx)
            .await
            .expect_err("still progressing");
        assert_eq!(
            failure,
            d2b_resource_runtime::error::DriverFailure::retryable(
                d2b_resource_runtime::error::DriverOp::Delete
            )
        );
    }

    /// A component that realizes nothing through resource rows adopts
    /// immediately (F2); a child-bearing component adopts only when its
    /// complete declared set is committed.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]

    #[tokio::test]
    async fn recover_adopts_the_committed_child_set() {
        let log: Log = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let manager = RecordingManager::new(Arc::clone(&log));
        let mut fixture = fixture(
            "FixtureOne",
            "row-a",
            json!({"providerRef": "Provider/fixture-one"}),
            Arc::clone(&manager),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Complete,
            true,
        );
        let mut driver = driver(&fixture).await;
        assert_eq!(
            driver.recover(&mut fixture.ctx).await.expect("recover"),
            RecoveryOutcome::Missing
        );

        manager
            .owned
            .lock()
            .await
            .push(owned_row("dev", "Volume", "fixture-child"));
        assert_eq!(
            driver.recover(&mut fixture.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted
        );
        assert!(
            !log.lock().await.iter().any(|entry| entry.starts_with("effect:")),
            "recovery adoption runs no provider effect"
        );
    }
}
