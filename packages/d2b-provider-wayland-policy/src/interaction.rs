//! The interaction family's shared engine.
//!
//! Six resource types share one driver shape: `WaylandPolicy` and
//! `WaylandSession` (display-wayland), `AudioService` and `AudioBinding`
//! (audio-pipewire), and `ShellPool` and `ShellSession` (shell-terminal).
//! Each type lives in its own crate and declares its own row, decoder,
//! factory, descriptor, and driver behavior; the reconcile, recover, finalize,
//! and delete verbs, the spec-envelope decode, the manager-child plumbing, and
//! the effect port every type drives live here, in the family's root crate.
//!
//! A per-type crate never re-implements a verb: it implements
//! [`InteractionType`], names its row in [`InteractionType::KIND`], and builds
//! its descriptor from [`InteractionDriverArgs`]. The daemon implements
//! [`InteractionDriverEffects`] with the production effect adapter, so this
//! module holds no host state, no path, and no provider identity of its own.
//!
//! Process work never happens here: display workers, audio workers, and the
//! shell supervisor are child rows ensured through the manager and launched by
//! the Process driver. This engine has no spawn surface.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildIntent;
use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceUid, ZoneId};
use d2b_core_controller::{OwnedChildIntent, materialize_child_create_payload};
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::spec_store::EnsureOutcome;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// Closed Provider handler set served by the family.
///
/// Every effect call carries the handler row it binds, so the daemon's
/// production effect adapter dispatches on this value and never on a resource
/// type name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InteractionKind {
    /// The display-wayland WaylandPolicy row.
    DisplayWaylandPolicy,
    /// The display-wayland WaylandSession row.
    DisplayWaylandSession,
    /// The audio-pipewire AudioService row.
    AudioService,
    /// The audio-pipewire AudioBinding row.
    AudioBinding,
    /// The shell-terminal ShellPool row.
    ShellPool,
    /// The shell-terminal ShellSession row.
    ShellSession,
}

impl InteractionKind {
    /// The stable effect label of one handler row.
    ///
    /// The label is the handler identity effect logs and behavior tests key
    /// on; it is not a wire value.
    pub const fn effect_id(self) -> &'static str {
        match self {
            Self::DisplayWaylandPolicy => "display-wayland-policy",
            Self::DisplayWaylandSession => "display-wayland-session",
            Self::AudioService => "audio-service",
            Self::AudioBinding => "audio-binding",
            Self::ShellPool => "shell-pool",
            Self::ShellSession => "shell-session",
        }
    }
}

/// Result returned by one typed Provider effect adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionEffectPhase {
    /// The Provider realization is current.
    Ready,
    /// The Provider realization is still converging.
    Pending,
}

/// One Provider effect outcome: the phase the effect returned plus the
/// `status. resource` projection the effect publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionEffectOutcome {
    /// The convergence phase of the row.
    pub phase: InteractionEffectPhase,
    /// The optional `status. resource` projection.
    pub resource: Option<Value>,
}

impl InteractionEffectOutcome {
    /// A phase-only outcome with no `status. resource` projection.
    pub const fn phase(phase: InteractionEffectPhase) -> Self {
        Self {
            phase,
            resource: None,
        }
    }

    /// A phase outcome carrying the row's `status. resource` projection.
    pub fn projection(phase: InteractionEffectPhase, resource: Value) -> Self {
        Self {
            phase,
            resource: Some(resource),
        }
    }
}

/// Outcome of one Provider teardown stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionFinalize {
    /// Cleanup finished; the owned children may retire.
    Complete,
    /// Cleanup is progressing or refused by a live dependent: re-enter.
    Pending,
}

/// Closed failure surface for interaction Provider adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionEffectError {
    /// The Provider path is not currently available and should retry.
    Unavailable,
    /// Fresh resource or assignment evidence failed closed.
    InvalidResource,
    /// A wire spec or envelope failed to parse; carries the serde reason.
    InvalidSpec(String),
}

impl core::fmt::Display for InteractionEffectError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "interaction-effect-unavailable",
            Self::InvalidResource | Self::InvalidSpec(_) => "interaction-resource-invalid",
        })
    }
}

impl std::error::Error for InteractionEffectError {}

// ---------------------------------------------------------------------------
// Driver error classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionDriverErrorKind {
    /// The durable spec did not decode or names a Provider outside the row's
    /// ResourceType: terminal, retrying cannot change the stored spec.
    SpecInvalid,
    /// A manager child mutation failed (retryable: the manager owns retries).
    ChildMutation,
    /// The provider path is temporarily unavailable.
    ProviderUnavailable,
    /// A Provider teardown stage is still progressing.
    DeletePending,
}

impl InteractionDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::SpecInvalid => FailureClass::Terminal,
            Self::ChildMutation | Self::ProviderUnavailable | Self::DeletePending => {
                FailureClass::Retryable
            }
        }
    }

    const fn code(self) -> &'static str {
        match self {
            Self::SpecInvalid => "interaction-spec-invalid",
            Self::ChildMutation => "interaction-child-mutation",
            Self::ProviderUnavailable => "interaction-unavailable",
            Self::DeletePending => "interaction-delete-pending",
        }
    }
}

/// Closed failure surface of the family engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionDriverError {
    kind: InteractionDriverErrorKind,
    op: DriverOp,
}

impl InteractionDriverError {
    const fn new(kind: InteractionDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }
}

impl core::fmt::Display for InteractionDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.kind.code())
    }
}

impl std::error::Error for InteractionDriverError {}

/// Typed in-memory status projection (never persisted). Carries the closed
/// phase the effect published plus the Provider's `status. resource`
/// projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionDriverStatus {
    /// Whether the Provider realization is current.
    pub ready: bool,
    /// The Provider's `status. resource` projection, when it published one.
    pub resource: Option<Value>,
}

// ---------------------------------------------------------------------------
// Spec decode (manager-wired)
// ---------------------------------------------------------------------------

/// Decoded interaction spec envelope: the canonical spec document the
/// Provider handlers read, envelope-aware so API-created rows (persisted as
/// the full envelope minus status) and Nix rows (persisted as the compiled
/// spec document) decode the same way.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractionSpecEnvelope {
    /// The spec document (never the surrounding envelope).
    value: Value,
    /// The Layer 2 base view: the document minus the universal
    /// `providerRef`/`updatePolicy` and the Layer 3 `provider` extension.
    base: Value,
}

impl InteractionSpecEnvelope {
    /// The Layer 2 base view of the row's spec document.
    pub fn base(&self) -> &Value {
        &self.base
    }

    /// The spec's Provider reference (the row selector).
    pub fn provider_ref(&self) -> Option<&str> {
        self.value.get("providerRef").and_then(Value::as_str)
    }

    /// Decode one Layer 2 base spec.
    pub fn base_spec<T: DeserializeOwned>(&self) -> Result<T, InteractionEffectError> {
        serde_json::from_value(self.base.clone())
            .map_err(|error| InteractionEffectError::InvalidSpec(error.to_string()))
    }

    /// Decode one typed spec that carries `providerRef` itself (the audio
    /// Provider's typed specs re-insert the universal field the envelope
    /// split removed).
    pub fn spec_with_provider_ref<T: DeserializeOwned>(
        &self,
    ) -> Result<T, InteractionEffectError> {
        let mut spec = self.base.clone();
        if let Some(provider_ref) = self.provider_ref() {
            let object = spec
                .as_object_mut()
                .ok_or(InteractionEffectError::InvalidResource)?;
            object.insert(
                "providerRef".to_owned(),
                Value::String(provider_ref.to_owned()),
            );
        }
        serde_json::from_value(spec)
            .map_err(|error| InteractionEffectError::InvalidSpec(error.to_string()))
    }
}

/// Closed decode error for an interaction spec envelope.
#[derive(Debug, thiserror::Error)]
#[error("interaction spec must be a JSON object")]
pub struct InteractionSpecDecodeError;

/// The manager-wired decode hook every type of the family shares.
pub fn spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        let parsed = serde_json::from_slice::<Value>(bytes)
            .map_err(|_| InteractionSpecDecodeError)?;
        if !parsed.is_object() {
            return Err(InteractionSpecDecodeError);
        }
        let value = if parsed.get("apiVersion").is_some() && parsed.get("spec").is_some() {
            parsed
                .get("spec")
                .cloned()
                .ok_or(InteractionSpecDecodeError)?
        } else {
            parsed
        };
        let mut base = value.clone();
        if let Some(object) = base.as_object_mut() {
            object.remove("providerRef");
            object.remove("updatePolicy");
            object.remove("provider");
        } else {
            return Err(InteractionSpecDecodeError);
        }
        Ok(InteractionSpecEnvelope { value, base })
    })
}

// ---------------------------------------------------------------------------
// Effect request / port
// ---------------------------------------------------------------------------

/// One owned child row handed to a typed Provider effect call.
///
/// Durable rows only: the live phase is deliberately not carried here, the
/// production effects read it from the manager view or the durable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionChild {
    /// The child row's resource reference.
    pub resource_ref: ResourceRef,
    /// The child row's generation.
    pub generation: u64,
}

/// Everything one Provider effect call may read from the driver.
pub struct InteractionEffectRequest<'a> {
    /// The row's manager key.
    pub target: ResourceKey,
    /// The row's durable uid (the Provider effects key on it).
    pub uid: ResourceUid,
    /// The row's generation.
    pub generation: u64,
    /// The controller generation every effect call binds.
    pub controller_generation: u64,
    /// The Layer 2 base spec document of the row: the stored spec with the
    /// universal `providerRef`/`updatePolicy` and Layer 3 `provider` fields
    /// stripped, so every type decodes its typed spec without re-deriving the
    /// split.
    pub spec: Value,
    /// The row's `spec. providerRef`.
    pub provider_ref: Option<ResourceRef>,
    /// Owned child rows realizing the current desired child set.
    pub children: &'a [InteractionChild],
}

/// Typed Provider effect boundary owned by the daemon composition root.
///
/// The driver sees only these two closed, typed calls; the production
/// implementation owns the display session admission, the audio controller
/// registry, and the shell reference checks.
#[async_trait]
pub trait InteractionDriverEffects: Send + Sync + 'static {
    /// Reconcile one display/audio/shell resource through its typed handler.
    async fn reconcile(
        &self,
        kind: InteractionKind,
        request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionEffectOutcome, InteractionEffectError>;

    /// Advance one Provider teardown stage.
    async fn finalize(
        &self,
        kind: InteractionKind,
        request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionFinalize, InteractionEffectError>;
}

// ---------------------------------------------------------------------------
// Per-type declaration
// ---------------------------------------------------------------------------

/// The row facts one type's child derivation may read.
///
/// The manager owns the child identity, so a type derives its desired child
/// set from the row's key, uid, and generation plus the driver's Zone and
/// controller generation.
pub struct InteractionChildContext<'a> {
    /// The driver's Zone.
    pub zone: &'a ZoneId,
    /// The row's manager key.
    pub key: &'a ResourceKey,
    /// The row's durable uid.
    pub uid: &'a [u8; 16],
    /// The row's generation.
    pub generation: u64,
    /// The controller generation every effect call binds.
    pub controller_generation: u64,
}

/// One interaction type's declaration: its row and its typed spec behavior.
///
/// Each per-type crate implements this once. The engine supplies every verb
/// around it, so the implementation carries vocabulary and checks only - never
/// a reconcile step, a child mutation, or an effect call.
pub trait InteractionType: Clone + Send + Sync + 'static {
    /// The family handler row this type is served by.
    const KIND: InteractionKind;
    /// The type's canonical ResourceType name.
    const RESOURCE_TYPE: &'static str;
    /// The Provider reference the type's rows select.
    const PROVIDER_REF: &'static str;
    /// Whether a row's spec must carry the typed universal `spec. providerRef`.
    const SPEC_PROVIDER_SELECTOR: bool;

    /// The preserved reconcile resync cadence of the type: the Provider's
    /// repair interval, read by the type's own crate.
    fn resync(&self) -> std::time::Duration;

    /// Structural spec checks beyond decode.
    fn validate(&self, envelope: &InteractionSpecEnvelope)
    -> Result<(), InteractionEffectError>;

    /// The dependency references one reconcile pass watches for readiness.
    fn dependencies(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ResourceRef>, InteractionEffectError>;

    /// The desired child set of one reconcile pass, as manager child rows.
    ///
    /// Types whose Provider realizes nothing through resource rows return an
    /// empty set.
    fn desired_children(
        &self,
        children: &InteractionChildContext<'_>,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ChildEnsure>, InteractionEffectError>;
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Construction arguments shared by every driver of the family.
#[derive(Clone)]
pub struct InteractionDriverArgs<T: InteractionType> {
    /// The driver's Zone.
    pub zone: ZoneId,
    /// The controller generation every effect call binds.
    pub controller_generation: ControllerGeneration,
    /// The Provider effect port the daemon implements.
    pub effects: Arc<dyn InteractionDriverEffects>,
    /// The type's declared behavior.
    pub behavior: T,
}

/// Factory for one interaction type.
///
/// Construction is infallible by contract.
pub struct InteractionDriverFactory<T: InteractionType> {
    types: Vec<ResourceTypeName>,
    args: InteractionDriverArgs<T>,
}

impl<T: InteractionType> InteractionDriverFactory<T> {
    /// Build the factory for its declared type.
    pub fn new(args: InteractionDriverArgs<T>) -> Self {
        Self {
            types: vec![ResourceTypeName::new(T::RESOURCE_TYPE)],
            args,
        }
    }
}

impl<T: InteractionType> core::fmt::Debug for InteractionDriverFactory<T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InteractionDriverFactory")
            .field("types", &self.types)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<T: InteractionType> ResourceDriverFactory for InteractionDriverFactory<T> {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(InteractionDriver::new(self.args.clone()))
    }
}

/// Classify one family-engine refusal onto the structured failure surface.
fn classify_interaction_error(error: &InteractionDriverError) -> DriverFailure {
    match error.kind.class() {
        FailureClass::Retryable => DriverFailure::retryable(error.op),
        FailureClass::Terminal => DriverFailure::terminal(error.op),
    }
}

/// One desired interaction resource.
pub struct InteractionDriver<T: InteractionType> {
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    effects: Arc<dyn InteractionDriverEffects>,
    behavior: T,
    /// Dependency/child keys already watched (exactly once per target).
    watched: Vec<ResourceKey>,
}

impl<T: InteractionType> InteractionDriver<T> {
    /// Build the driver for its declared type.
    ///
    /// The zone arrives as a validated [`ZoneId`] from the daemon
    /// construction boundary, so construction is infallible.
    pub fn new(args: InteractionDriverArgs<T>) -> Self {
        Self {
            zone: args.zone,
            controller_generation: args.controller_generation,
            effects: args.effects,
            behavior: args.behavior,
            watched: Vec::new(),
        }
    }

    fn error(&self, kind: InteractionDriverErrorKind, op: DriverOp) -> InteractionDriverError {
        InteractionDriverError::new(kind, op)
    }

    /// The decoded spec envelope of the row being driven.
    fn envelope(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<InteractionSpecEnvelope, InteractionDriverError> {
        ctx.spec::<InteractionSpecEnvelope>()
            .cloned()
            .map_err(|_| self.error(InteractionDriverErrorKind::SpecInvalid, op))
    }

    /// Check the row this driver is serving: the Zone, the declared type, and
    /// the Provider selector the type's rows carry.
    fn check_row(
        &self,
        ctx: &ResourceContext,
        envelope: &InteractionSpecEnvelope,
        op: DriverOp,
    ) -> Result<(), InteractionDriverError> {
        if ctx.key().zone != self.zone.as_str()
            || ctx.key().type_name != T::RESOURCE_TYPE
            || (T::SPEC_PROVIDER_SELECTOR
                && envelope.provider_ref() != Some(T::PROVIDER_REF))
        {
            return Err(self.error(InteractionDriverErrorKind::SpecInvalid, op));
        }
        Ok(())
    }

    /// Register one internal dependency/child watch exactly once per target.
    ///
    /// Best-effort by design: a dependency that is not a manager actor yet
    /// cannot be watched, and the resync schedule re-evaluates it.
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) || target == *ctx.key() {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    fn child_key(&self, target: &ResourceRef) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        )
    }

    /// Register one dependency reference as a readiness watch.
    async fn watch_dependency(
        &mut self,
        ctx: &mut ResourceContext,
        target: &ResourceRef,
    ) {
        self.watch_once(ctx, self.child_key(target)).await;
    }

    /// The row facts one type's child derivation reads.
    fn child_context<'a>(&'a self, ctx: &'a ResourceContext) -> InteractionChildContext<'a> {
        InteractionChildContext {
            zone: &self.zone,
            key: ctx.key(),
            uid: ctx.uid(),
            generation: ctx.generation(),
            controller_generation: self.controller_generation.get(),
        }
    }

    /// Retire owned children the desired set no longer derives, in the
    /// family's preserved order (endpoint-first, then the producer Process).
    async fn retire_obsolete_children(
        &self,
        ctx: &mut ResourceContext,
        desired: &[ChildEnsure],
        op: DriverOp,
    ) -> Result<bool, InteractionDriverError> {
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(InteractionDriverErrorKind::ChildMutation, op))?;
        let mut obsolete = owned
            .iter()
            .filter(|row| {
                !row.deleting
                    && !desired.iter().any(|child| {
                        child.type_name.as_str() == row.key.type_name && child.name == row.key.name
                    })
            })
            .collect::<Vec<_>>();
        obsolete.sort_by_key(|row| (teardown_rank(&row.key.type_name), row.key.name.clone()));
        let mutated = !obsolete.is_empty();
        for row in obsolete {
            ctx.delete(&row.key)
                .await
                .map_err(|_| self.error(InteractionDriverErrorKind::ChildMutation, op))?;
        }
        Ok(mutated)
    }

    /// The owned child rows realizing the desired child set, as the typed
    /// effect request sees them.
    async fn realized_children(
        &self,
        ctx: &mut ResourceContext,
        desired: &[ChildEnsure],
        op: DriverOp,
    ) -> Result<Vec<InteractionChild>, InteractionDriverError> {
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(InteractionDriverErrorKind::ChildMutation, op))?;
        owned
            .iter()
            .filter(|row| {
                !row.deleting
                    && desired.iter().any(|child| {
                        child.type_name.as_str() == row.key.type_name && child.name == row.key.name
                    })
            })
            .map(|row| {
                Ok(InteractionChild {
                    resource_ref: key_ref(&row.key)
                        .map_err(|_| self.error(InteractionDriverErrorKind::SpecInvalid, op))?,
                    generation: row.generation,
                })
            })
            .collect::<Result<Vec<_>, InteractionDriverError>>()
    }

    fn effect_error(&self, error: InteractionEffectError, op: DriverOp) -> InteractionDriverError {
        match error {
            InteractionEffectError::InvalidResource | InteractionEffectError::InvalidSpec(_) => {
                self.error(InteractionDriverErrorKind::SpecInvalid, op)
            }
            InteractionEffectError::Unavailable => {
                self.error(InteractionDriverErrorKind::ProviderUnavailable, op)
            }
        }
    }

    fn request<'a>(
        &self,
        ctx: &ResourceContext,
        envelope: &InteractionSpecEnvelope,
        children: &'a [InteractionChild],
        op: DriverOp,
    ) -> Result<InteractionEffectRequest<'a>, InteractionDriverError> {
        Ok(InteractionEffectRequest {
            target: ctx.key().clone(),
            uid: resource_uid(ctx.uid())
                .ok_or_else(|| self.error(InteractionDriverErrorKind::SpecInvalid, op))?,
            generation: ctx.generation(),
            controller_generation: self.controller_generation.get(),
            spec: envelope.base().clone(),
            provider_ref: envelope
                .provider_ref()
                .and_then(|provider_ref| ResourceRef::parse(provider_ref).ok()),
            children,
        })
    }

    /// The desired child set of one pass.
    ///
    /// A derivation failure is a spec failure: the stored row cannot describe
    /// a child set this type can realize, and retrying cannot change it.
    fn desired_children(
        &self,
        ctx: &ResourceContext,
        envelope: &InteractionSpecEnvelope,
        op: DriverOp,
    ) -> Result<Vec<ChildEnsure>, InteractionDriverError> {
        self.behavior
            .desired_children(&self.child_context(ctx), envelope)
            .map_err(|_| self.error(InteractionDriverErrorKind::SpecInvalid, op))
    }
}

impl<T: InteractionType> core::fmt::Debug for InteractionDriver<T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InteractionDriver")
            .field("zone", &self.zone)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl<T: InteractionType> ResourceDriver for InteractionDriver<T> {
    type Error = InteractionDriverError;

    fn classify_error(&self, error: &InteractionDriverError) -> DriverFailure {
        classify_interaction_error(error)
    }

    /// Structural validation: the stored spec decodes, names a Provider this
    /// type owns, and satisfies the type's shape checks.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Validate;
        let envelope = self.envelope(ctx, op)?;
        self.check_row(ctx, &envelope, op)?;
        self.behavior
            .validate(&envelope)
            .map_err(|_| self.error(InteractionDriverErrorKind::SpecInvalid, op))
    }

    /// Discovery and adoption on the realization target: child-bearing types
    /// adopt when their complete desired child set is already present and
    /// current; types that realize nothing through resource rows adopt their
    /// Provider-side realization in reconcile.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let envelope = self.envelope(ctx, op)?;
        self.check_row(ctx, &envelope, op)?;
        let desired = self.desired_children(ctx, &envelope, op)?;
        if desired.is_empty() {
            return Ok(RecoveryOutcome::Adopted);
        }
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(InteractionDriverErrorKind::ChildMutation, op))?;
        let current = desired.iter().all(|child| {
            owned.iter().any(|row| {
                !row.deleting
                    && child.type_name.as_str() == row.key.type_name
                    && child.name == row.key.name
            })
        });
        Ok(if current {
            RecoveryOutcome::Adopted
        } else {
            RecoveryOutcome::Missing
        })
    }

    /// One reconcile pass: dependency edges, the desired child set through the
    /// manager child API, the typed Provider effect, and the in-memory status
    /// projection with a self-resync while the row is not converged.
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let envelope = self.envelope(ctx, op)?;
        self.check_row(ctx, &envelope, op)?;

        // Dependency edges: the resources this type's effects read are
        // watched so their readiness or death wakes this actor.
        let dependencies = self
            .behavior
            .dependencies(&envelope)
            .map_err(|_| self.error(InteractionDriverErrorKind::SpecInvalid, op))?;
        for dependency in dependencies {
            self.watch_dependency(ctx, &dependency).await;
        }

        // Desired child set through the manager child API: every row is
        // committed before its actor exists.
        let desired = self.desired_children(ctx, &envelope, op)?;
        let mut mutated = false;
        for child in &desired {
            match ctx.ensure_child(child.clone()).await {
                Ok(EnsureOutcome::Created(_) | EnsureOutcome::Updated(_)) => mutated = true,
                Ok(EnsureOutcome::Unchanged(_)) => {}
                Err(_) => {
                    return Err(self.error(InteractionDriverErrorKind::ChildMutation, op));
                }
            }
        }
        mutated |= self.retire_obsolete_children(ctx, &desired, op).await?;
        for child in &desired {
            self.watch_once(
                ctx,
                ResourceKey::new(self.zone.as_str(), child.type_name.as_str(), child.name.as_str()),
            )
            .await;
        }

        let children = self.realized_children(ctx, &desired, op).await?;
        let request = self.request(ctx, &envelope, &children, op)?;
        let outcome = self
            .effects
            .reconcile(T::KIND, &request)
            .await
            .map_err(|error| self.effect_error(error, op))?;

        ctx.set_status(InteractionDriverStatus {
            ready: outcome.phase == InteractionEffectPhase::Ready,
            resource: outcome.resource,
        });
        if mutated || outcome.phase != InteractionEffectPhase::Ready {
            ctx.requeue_after(self.behavior.resync());
        }
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Drain step: every owned child finalizes before this resource's own
    /// teardown, and before the Provider's teardown stage in
    /// [`ResourceDriver::delete`].
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| self.error(InteractionDriverErrorKind::DeletePending, DriverOp::Delete))?;
        Ok(())
    }

    /// Teardown: the Provider's teardown stage runs first - the audio lease
    /// finalization, and the AudioService/ShellPool refusals that keep an
    /// owner alive while a dependent Binding/Session remains - then the owned
    /// children retire in the family's preserved order. Idempotent under
    /// retry; a malformed spec skips the Provider stage and still drains the
    /// owned children.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        if let Ok(envelope) = self.envelope(ctx, op)
            && self.check_row(ctx, &envelope, op).is_ok()
            && let Ok(request) = self.request(ctx, &envelope, &[], op)
        {
            match self.effects.finalize(T::KIND, &request).await {
                Ok(InteractionFinalize::Complete) => {}
                Ok(InteractionFinalize::Pending) => {
                    return Err(self.error(InteractionDriverErrorKind::DeletePending, op));
                }
                Err(error) => return Err(self.effect_error(error, op)),
            }
        }
        self.retire_obsolete_children(ctx, &[], op).await?;
        Ok(())
    }
}

/// The resource reference of one manager key.
///
/// # Errors
///
/// Returns the `SpecInvalid` refusal when the key does not carry a
/// canonical resource reference.
pub fn key_ref(key: &ResourceKey) -> Result<ResourceRef, InteractionDriverError> {
    ResourceRef::parse(&format!("{}/{}", key.type_name, key.name)).map_err(|_| {
        InteractionDriverError::new(InteractionDriverErrorKind::SpecInvalid, DriverOp::Validate)
    })
}

/// Convert one durable 16-byte uid to its canonical identity (the manager
/// persists the uid as bytes; the Provider effects key on the canonical
/// string).
pub fn resource_uid(bytes: &[u8; 16]) -> Option<ResourceUid> {
    ResourceUid::from_bytes(bytes).ok()
}

/// Teardown ranks: endpoints retire before their producing processes.
fn teardown_rank(resource_type: &str) -> u8 {
    match resource_type {
        "Endpoint" => 0,
        "EphemeralProcess" => 1,
        "Process" => 2,
        _ => 3,
    }
}

/// One display-owned child intent as a manager child row.
///
/// The intent body is the full resource envelope the display Provider
/// synthesized; the manager owns the child identity, so only the spec and the
/// authored metadata (ownerRef, labels, annotations - the restart-generation
/// annotation the display status reads) are carried.
pub fn owned_child_ensure(intent: &OwnedChildIntent) -> Result<ChildEnsure, InteractionEffectError> {
    let invalid = || InteractionEffectError::InvalidResource;
    let value: Value = serde_json::from_slice(intent.canonical_resource())
        .map_err(|error| InteractionEffectError::InvalidSpec(error.to_string()))?;
    let spec = value.get("spec").cloned().ok_or_else(invalid)?;
    let metadata = value.get("metadata").cloned().unwrap_or_else(|| json!({}));
    Ok(ChildEnsure {
        type_name: ResourceTypeName::new(intent.target().resource_type().as_str()),
        name: intent.target().name().as_str().to_owned(),
        spec: serde_json::to_vec(&spec)
            .map_err(|error| InteractionEffectError::InvalidSpec(error.to_string()))?,
        metadata: serde_json::to_vec(&json!({
            "ownerRef": metadata.get("ownerRef").cloned().unwrap_or(Value::Null),
            "labels": metadata.get("labels").cloned().unwrap_or_else(|| json!({})),
            "annotations": metadata.get("annotations").cloned().unwrap_or_else(|| json!({})),
        }))
        .map_err(|error| InteractionEffectError::InvalidSpec(error.to_string()))?,
    })
}

/// One Provider-declared Binding child as a manager child row (Providers
/// declare intent, the controller owns the child body, and the Process
/// Provider stays controller-chosen).
pub fn binding_child_ensure(
    intent: &BindingChildIntent,
    zone: &ZoneId,
) -> Result<ChildEnsure, InteractionEffectError> {
    let invalid = || InteractionEffectError::InvalidResource;
    let payload = materialize_child_create_payload(intent, zone).map_err(|_| invalid())?;
    let value = serde_json::from_slice::<Value>(&payload)
        .map_err(|error| InteractionEffectError::InvalidSpec(error.to_string()))?;
    let spec = value.get("spec").cloned().ok_or_else(invalid)?;
    let metadata = json!({
        "ownerRef": intent.owner_ref().to_canonical_string(),
        "labels": {},
        "annotations": {},
    });
    Ok(ChildEnsure {
        type_name: ResourceTypeName::new(intent.kind().resource_type()),
        name: intent.resource_ref().name().as_str().to_owned(),
        spec: serde_json::to_vec(&spec)
            .map_err(|error| InteractionEffectError::InvalidSpec(error.to_string()))?,
        metadata: serde_json::to_vec(&metadata)
            .map_err(|error| InteractionEffectError::InvalidSpec(error.to_string()))?,
    })
}
