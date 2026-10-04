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
use d2b_resource_runtime::ResourceStatus;
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
/// `status.resource` projection the effect publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionEffectOutcome {
    /// The convergence phase of the row.
    pub phase: InteractionEffectPhase,
    /// The optional `status.resource` projection.
    pub resource: Option<Value>,
}

impl InteractionEffectOutcome {
    /// A phase-only outcome with no `status.resource` projection.
    pub const fn phase(phase: InteractionEffectPhase) -> Self {
        Self {
            phase,
            resource: None,
        }
    }

    /// A phase outcome carrying the row's `status.resource` projection.
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
/// phase the effect published plus the Provider's `status.resource`
/// projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionDriverStatus {
    /// Whether the Provider realization is current.
    pub ready: bool,
    /// The Provider's `status.resource` projection, when it published one.
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
    /// The child row's durable uid: the exact consumer identity an admitted
    /// relationship is evaluated against.
    pub uid: ResourceUid,
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
    /// The row's `spec.providerRef`.
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
    /// Whether a row's spec must carry the typed universal `spec.providerRef`.
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

/// The evidence one watched target's watch conditions are evaluated against.
///
/// Exactly the pair a resource actor compares when it publishes a transition:
/// the status for [`WatchCondition::Ready`], the `status.resource` layer for
/// [`WatchCondition::ProjectionChanged`]. A fingerprint that moved is a
/// registration the target has already satisfied and removed; one that stands
/// is a registration still live in the target's mailbox.
#[derive(PartialEq)]
struct WatchEvidence {
    status: Option<ResourceStatus>,
    projection: Option<Value>,
}

/// One armed internal watch: the target, and the evidence its registrations
/// were evaluated against when they went out.
struct ArmedWatch {
    target: ResourceKey,
    evidence: WatchEvidence,
}

/// The evidence one watched target's conditions are evaluated against, read
/// from the manager's in-memory plane.
///
/// A target actor enqueues its transition to the manager in the same handler
/// that notifies a projection-change subscriber, so the pass that wake opened
/// observes at least that transition: unchanged evidence means the live
/// registrations were not spent, and anything else means they were. A read the
/// plane cannot answer is evidence too - absence, not a guess - so a target
/// that appears later moves the fingerprint and re-arms.
async fn target_evidence(ctx: &mut ResourceContext, target: &ResourceKey) -> WatchEvidence {
    let (status, projection) = match ctx.get_view(target).await {
        Ok(Some(view)) => (view.status, view.status_projection),
        _ => (None, None),
    };
    WatchEvidence { status, projection }
}

/// One desired interaction resource.
pub struct InteractionDriver<T: InteractionType> {
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    effects: Arc<dyn InteractionDriverEffects>,
    behavior: T,
    /// Armed dependency and child watches: each target, beside the evidence
    /// its live registrations were armed against (see [`Self::watch_target`]).
    watched: Vec<ArmedWatch>,
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

    /// Arm this row's internal watches on one dependency or child target.
    ///
    /// Two conditions, never one. A readiness phase cannot express a delivery
    /// downgrade: the row that publishes `Undelivered` over a withdrawn
    /// authorization keeps reporting `Ready`, so a `Ready` subscription alone
    /// never wakes this row for the evidence change it has to re-read (R21,
    /// AE18, KTD7).
    ///
    /// The arming is per evidence, not per driver. An internal registration is
    /// satisfied exactly once and then removed by the target actor, so a
    /// per-driver latch leaves this row unsubscribed the instant the world
    /// moves - and a pass that published `Ready` without mutating anything
    /// requeues nothing, so nothing else would ever re-arm it. This reads the
    /// target's live view and compares it against the evidence the live
    /// registrations were evaluated against: equal evidence means both are
    /// still standing and this pass places none, which is what keeps a
    /// requeued pass from stacking duplicates on one target.
    ///
    /// Best-effort by design: a dependency that is not a manager actor yet
    /// cannot be watched, and the resync schedule re-evaluates it.
    async fn watch_target(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if target == *ctx.key() {
            return;
        }
        let evidence = target_evidence(ctx, &target).await;
        if self
            .watched
            .iter()
            .any(|armed| armed.target == target && armed.evidence == evidence)
        {
            return;
        }
        let ready = ctx
            .watch(target.clone(), WatchCondition::Ready)
            .await
            .is_ok();
        let changed = ctx
            .watch(target.clone(), WatchCondition::ProjectionChanged)
            .await
            .is_ok();
        // Both or neither: a half-placed pair would leave one condition
        // unsubscribed until that evidence moved again, which is exactly the
        // gap this method exists to close.
        if ready && changed {
            self.watched.retain(|armed| armed.target != target);
            self.watched.push(ArmedWatch { target, evidence });
        }
    }

    /// Drop the arming of every target this pass no longer watches, so a
    /// retired child leaves nothing behind for this row to keep re-reading.
    fn forget_unwatched(&mut self, targets: &[ResourceKey]) {
        self.watched.retain(|armed| targets.contains(&armed.target));
    }

    fn child_key(&self, target: &ResourceRef) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        )
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

    /// Revoke every obsolete endpoint relationship this pass owns.
    ///
    /// Endpoint access closes before any helper row retires: a session's
    /// endpoint relationships are asked to revoke first, and the pass reports
    /// that it did, so the delete verb returns pending and the helper rows
    /// retire only once the revocation has been observed.
    async fn revoke_endpoint_relationships(
        &self,
        ctx: &mut ResourceContext,
        desired: &[ChildEnsure],
        op: DriverOp,
    ) -> Result<bool, InteractionDriverError> {
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(InteractionDriverErrorKind::ChildMutation, op))?;
        let mut revoking = owned
            .iter()
            .filter(|row| {
                !row.deleting
                    && is_endpoint_relationship(row.key.type_name.as_str())
                    && !desired.iter().any(|child| {
                        child.type_name.as_str() == row.key.type_name && child.name == row.key.name
                    })
            })
            .map(|row| row.key.clone())
            .collect::<Vec<_>>();
        revoking.sort_by_key(|key| {
            (
                teardown_rank(key.type_name.as_str()),
                key.name.clone(),
            )
        });
        let revoked = !revoking.is_empty();
        for key in revoking {
            ctx.delete(&key)
                .await
                .map_err(|_| self.error(InteractionDriverErrorKind::ChildMutation, op))?;
        }
        Ok(revoked)
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
                    uid: resource_uid(&row.uid)
                        .ok_or_else(|| self.error(InteractionDriverErrorKind::SpecInvalid, op))?,
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
        // watched on readiness AND on projection change, so their readiness,
        // their evidence, or their death wakes this actor.
        let dependencies = self
            .behavior
            .dependencies(&envelope)
            .map_err(|_| self.error(InteractionDriverErrorKind::SpecInvalid, op))?;
        let mut watched = Vec::with_capacity(dependencies.len());
        for dependency in dependencies {
            let target = self.child_key(&dependency);
            self.watch_target(ctx, target.clone()).await;
            watched.push(target);
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
            let target = ResourceKey::new(
                self.zone.as_str(),
                child.type_name.as_str(),
                child.name.as_str(),
            );
            self.watch_target(ctx, target.clone()).await;
            watched.push(target);
        }
        self.forget_unwatched(&watched);

        let children = self.realized_children(ctx, &desired, op).await?;
        let request = self.request(ctx, &envelope, &children, op)?;
        let outcome = self
            .effects
            .reconcile(T::KIND, &request)
            .await
            .map_err(|error| self.effect_error(error, op))?;

        let InteractionEffectOutcome { phase, resource } = outcome;
        let ready = phase == InteractionEffectPhase::Ready;
        // The Provider's own projection is this row's wire layer as well as
        // its typed status: holding it only in the in-memory slot published an
        // empty `status.resource` on every row, so no reader could see what
        // the pass proved. The projection belongs to this pass - the actor
        // takes it after the pass that set it - so it is published as the
        // effect produced it, never a later or synthesized one.
        if let Some(projection) = resource.clone() {
            ctx.set_status_projection(projection);
        }
        ctx.set_status(InteractionDriverStatus { ready, resource });
        if mutated || !ready {
            ctx.requeue_after(self.behavior.resync());
        }
        // `Satisfied` is the runtime's "the desired state is realized, the
        // actor may publish `Ready`" verdict; `RetryScheduled` is its "not
        // realized, no effect in flight, publish `Pending`". An interaction
        // row owns its whole realization - the children it committed and the
        // relationships those children publish - so the Provider's phase IS
        // the row's readiness. Answering `Satisfied` over a `Pending` phase
        // published `Ready` over children that had not converged, waking
        // every watcher on a claim nothing backed. The requeue above is this
        // pass's own, which is exactly what `RetryScheduled` reports.
        Ok(if ready {
            ReconcileOutcome::Satisfied
        } else {
            ReconcileOutcome::RetryScheduled
        })
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
    /// owner alive while a dependent Binding/Session remains - then every owned
    /// endpoint relationship is revoked, and only the next pass retires the
    /// helper rows once that revocation has been requested. Idempotent under
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
        // Endpoint access is revoked before any helper retires: a pass that
        // asked for a revocation returns pending, so the worker rows that
        // could still use that access are not deleted in the same pass.
        if self.revoke_endpoint_relationships(ctx, &[], op).await? {
            return Err(self.error(InteractionDriverErrorKind::DeletePending, op));
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

/// Teardown ranks: endpoint relationships revoke before their producing
/// processes.
fn teardown_rank(resource_type: &str) -> u8 {
    match resource_type {
        "EndpointBinding" => 0,
        "Endpoint" => 1,
        "EphemeralProcess" => 2,
        "Process" => 3,
        _ => 4,
    }
}

/// Whether one row is an endpoint relationship, whose revocation must be
/// requested before any helper row retires.
fn is_endpoint_relationship(resource_type: &str) -> bool {
    matches!(resource_type, "EndpointBinding" | "Endpoint")
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

// ---------------------------------------------------------------------------
// Tests: the readiness verdict a session publishes, over the real effects
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceUid, ZoneId};
    use d2b_core_controller::OwnedChildIntent;
    use d2b_provider_display_wayland::{
        DisplayIdentity, EndpointSpec, WaylandSessionSpec, session_children,
    };
    use d2b_provider_toolkit::testing::fakes::RecordingRequeue;
    use d2b_resource_runtime::ResourceStatus;
    use d2b_resource_runtime::context::{
        ChildEnsure, EffectCompleted, ManagerEndpoint, RequeueScheduler, ResourceContext,
        WatchCondition, WatchId, WatchRegistration, WatchSatisfied,
    };
    use d2b_resource_runtime::driver::{ReconcileOutcome, ResourceDriver};
    use d2b_resource_runtime::error::ResourceError;
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use serde_json::{Value, json};
    use tokio::sync::Mutex;

    use super::{
        InteractionChildContext, InteractionDriver, InteractionDriverArgs, InteractionDriverStatus,
        InteractionEffectError, InteractionKind, InteractionSpecEnvelope, InteractionType, key_ref,
        owned_child_ensure, resource_uid, spec_decoder,
    };
    use crate::InteractionPlaneRead;
    use crate::effects_service::InteractionEffectsService;
    use crate::test_support::{ScriptedPlane, scripted_facets_over_plane, scripted_identity};

    /// The Zone every fixture row lives in.
    fn zone() -> ZoneId {
        ZoneId::parse("work").expect("fixture Zone")
    }

    /// The canonical `WaylandSession` ResourceType name.
    const SESSION_TYPE: &str = "display-wayland.d2bus.org.WaylandSession";

    /// The session row's durable uid, the exact bytes of the scripted
    /// admission fence's `33333333-3333-4333-8333-333333333333`. The fixture
    /// asserts the two agree, so a drifting script fails here rather than
    /// quietly reading a different identity than production would.
    const SESSION_UID: [u8; 16] = [
        0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x43, 0x33, 0x83, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33,
    ];

    /// The row generation the fixture drives every pass at.
    const SESSION_GENERATION: u64 = 4;

    /// The realization token every `Endpoint` row and every delivered
    /// relationship in the fixture plane names: the exact shape
    /// `RealizationIncarnation::derive` produces (a bounded token, 192 bits of
    /// digest), so the fixture is graded by the production comparison and not
    /// by a value the parser would reject.
    const FIXTURE_INCARNATION: &str =
        "incarnation-0123456789abcdef0123456789abcdef0123456789abcdef";

    fn session_uid() -> ResourceUid {
        assert_eq!(
            resource_uid(&SESSION_UID),
            Some(scripted_identity().wayland_session_uid),
            "the fixture row uid is the one the scripted admission fence answers"
        );
        resource_uid(&SESSION_UID).expect("the fixture row uid is a closed resource uid")
    }

    fn session_ref() -> ResourceRef {
        ResourceRef::parse("display-wayland.d2bus.org.WaylandSession/display-wayland")
            .expect("the session row reference")
    }

    /// The stored session spec document, naming the exact cross-domain rows the
    /// scripted admission fence answers for.
    fn session_spec() -> WaylandSessionSpec {
        WaylandSessionSpec::new(
            ResourceRef::parse("Guest/work").expect("guest"),
            ResourceRef::parse("Host/host-system").expect("host"),
            ResourceRef::parse("User/alice").expect("user"),
            ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/policy").expect("policy"),
            DisplayIdentity::new("display", "#112233", "#223344", "#334455").expect("identity"),
            true,
        )
        .expect("the session spec")
    }

    fn key_of(reference: &ResourceRef) -> ResourceKey {
        ResourceKey::new(
            zone().as_str(),
            reference.resource_type().as_str(),
            reference.name().as_str(),
        )
    }

    fn committed_intents() -> Vec<OwnedChildIntent> {
        session_children::display_owned_child_intents(
            &zone(),
            &session_ref(),
            &session_uid(),
            &session_spec(),
            SESSION_GENERATION,
        )
        .expect("the production child derivation")
    }

    // -- the declaration the driver is served over ---------------------------

    /// The display session's row vocabulary, over the production child
    /// derivation. Nothing here decides readiness: that is the effects
    /// service's aggregate and the engine's verdict, both under test.
    #[derive(Clone)]
    struct DisplaySession;

    impl InteractionType for DisplaySession {
        const KIND: InteractionKind = InteractionKind::DisplayWaylandSession;
        const RESOURCE_TYPE: &'static str = SESSION_TYPE;
        const PROVIDER_REF: &'static str = d2b_provider_display_wayland::PROVIDER_REF;
        const SPEC_PROVIDER_SELECTOR: bool = false;

        fn resync(&self) -> Duration {
            Duration::from_millis(300)
        }

        fn validate(
            &self,
            envelope: &InteractionSpecEnvelope,
        ) -> Result<(), InteractionEffectError> {
            envelope.base_spec::<WaylandSessionSpec>().map(|_| ())
        }

        fn dependencies(
            &self,
            envelope: &InteractionSpecEnvelope,
        ) -> Result<Vec<ResourceRef>, InteractionEffectError> {
            let spec = envelope.base_spec::<WaylandSessionSpec>()?;
            Ok(vec![
                spec.guest_ref().clone(),
                spec.host_ref().clone(),
                spec.user_ref().clone(),
                spec.policy_ref().clone(),
            ])
        }

        fn desired_children(
            &self,
            children: &InteractionChildContext<'_>,
            envelope: &InteractionSpecEnvelope,
        ) -> Result<Vec<ChildEnsure>, InteractionEffectError> {
            let spec = envelope.base_spec::<WaylandSessionSpec>()?;
            let session_ref = key_ref(children.key).map_err(|_| InteractionEffectError::InvalidResource)?;
            let session_uid =
                resource_uid(children.uid).ok_or(InteractionEffectError::InvalidResource)?;
            let intents = session_children::display_owned_child_intents(
                children.zone,
                &session_ref,
                &session_uid,
                &spec,
                children.generation,
            )
            .map_err(|_| InteractionEffectError::InvalidResource)?;
            intents.iter().map(owned_child_ensure).collect()
        }
    }

    // -- the manager boundary -----------------------------------------------

    /// The manager boundary, recording every committed owned child, every
    /// internal watch the driver registers, and serving the scripted runtime
    /// plane. This is the one surface the driver reaches the plane through; it
    /// commits what the child derivation asked for and reads it back as owned
    /// rows.
    struct ChildManager {
        rows: Arc<Mutex<Vec<StoredDesiredResource>>>,
        parent_uid: [u8; 16],
        /// The same scripted plane the effects read: in production the
        /// driver's `get_view` and the effects' plane read are one in-memory
        /// store (KTD12), so the boundary serves the very rows the pass will
        /// be judged against.
        plane: ScriptedPlane,
        /// Every `(target, condition)` registration the driver placed, in
        /// order, so a test can count what one pass armed and what a later
        /// pass stacked on top of it.
        watches: Arc<Mutex<Vec<(ResourceKey, WatchCondition)>>>,
    }

    #[async_trait]
    impl ManagerEndpoint for ChildManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            let key = ResourceKey::new(zone().as_str(), child.type_name.as_str(), child.name.clone());
            let mut rows = self.rows.lock().await;
            match rows.iter_mut().find(|row| row.key == key) {
                Some(row) => {
                    // The real manager answers `Unchanged` for a row the
                    // derivation already committed, so a converged pass can
                    // report that it mutated nothing - which is what stops it
                    // from requeueing itself.
                    if row.spec == child.spec && row.metadata == child.metadata {
                        return Ok(EnsureOutcome::Unchanged(row.clone()));
                    }
                    row.spec = child.spec;
                    row.metadata = child.metadata;
                    Ok(EnsureOutcome::Updated(row.clone()))
                }
                None => {
                    rows.push(StoredDesiredResource {
                        key,
                        uid: [0x77; 16],
                        generation: SESSION_GENERATION,
                        owner_uid: Some(self.parent_uid),
                        provenance: ResourceProvenance::Resource,
                        deleting: false,
                        spec: child.spec,
                        metadata: child.metadata,
                        created_at: 0,
                    });
                    Ok(EnsureOutcome::Created(rows.last().expect("pushed").clone()))
                }
            }
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self.rows.lock().await.iter().find(|row| row.key == *key).cloned())
        }

        async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
            // The runtime plane is the scripted store the effects read their
            // rows from too, so a pass reads one world through both seams.
            self.plane
                .get(key)
                .await
                .map_err(|()| ResourceError::ManagerRejected {
                    reason: "scripted plane".into(),
                })
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            if let Some(row) = self.rows.lock().await.iter_mut().find(|row| row.key == *key) {
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
                .await
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
            let mut watches = self.watches.lock().await;
            let id = WatchId(watches.len() as u64);
            watches.push((registration.target, registration.condition));
            Ok(id)
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    // -- the zone plane the effects read -------------------------------------

    fn plane_view(
        key: ResourceKey,
        generation: u64,
        status: ResourceStatus,
        spec: Vec<u8>,
        projection: Option<Value>,
    ) -> ResourceView {
        ResourceView {
            key,
            uid: [0x99; 16],
            generation,
            deleting: false,
            provenance: ResourceProvenance::Resource,
            spec,
            metadata: Vec::new(),
            owner_key: None,
            status: Some(status),
            status_generation: Some(generation),
            status_projection: projection,
        }
    }

    /// One committed child row's stored spec column: the `spec` member of the
    /// envelope the intent body carries, which is exactly what the manager
    /// commits for the row and what the delivery gate reads back.
    fn intent_spec_bytes(intent: &OwnedChildIntent) -> Vec<u8> {
        let envelope: Value =
            serde_json::from_slice(intent.canonical_resource()).expect("the committed envelope");
        serde_json::to_vec(envelope.get("spec").expect("the envelope spec")).expect("spec bytes")
    }

    /// The zone plane one session pass reads: the four rows the session reads,
    /// its own committed children, and the canonical relationship rows its
    /// `Endpoint` children publish. Every name is derived by the production
    /// derivations, so the fixture cannot drift from the vocabulary the gate
    /// reads.
    fn plane(children_ready: bool, delivered: bool) -> Vec<ResourceView> {
        let spec = session_spec();
        let mut rows = Vec::new();
        for reference in [
            spec.guest_ref().clone(),
            spec.host_ref().clone(),
            spec.user_ref().clone(),
            spec.policy_ref().clone(),
        ] {
            rows.push(plane_view(
                key_of(&reference),
                1,
                ResourceStatus::Ready,
                b"{}".to_vec(),
                None,
            ));
        }
        for intent in committed_intents() {
            let target = intent.target();
            // Every `Endpoint` row publishes the realization token its own
            // current generation stands for, and every relationship granted
            // over it carries that same token (KTD8): the fixture publishes
            // one token for the delivered graph and a different one for a
            // replaced realization, which is the downgrade the gate reads.
            let endpoint = target.resource_type().as_str() == "Endpoint";
            rows.push(plane_view(
                key_of(target),
                SESSION_GENERATION,
                if children_ready {
                    ResourceStatus::Ready
                } else {
                    ResourceStatus::Pending
                },
                intent_spec_bytes(&intent),
                endpoint.then(|| json!({"endpoint": {"incarnation": FIXTURE_INCARNATION}})),
            ));
            if !endpoint {
                continue;
            }
            let endpoint_spec: EndpointSpec =
                serde_json::from_slice(&intent_spec_bytes(&intent)).expect("endpoint spec");
            for relationship in session_children::display_canonical_bindings(
                &zone(),
                target,
                &endpoint_spec,
            )
            .expect("the canonical binding rows") {
                let generation = 1;
                let layer = if delivered {
                    json!({
                        "binding": {
                            "state": "delivered",
                            "generation": generation,
                            "incarnation": FIXTURE_INCARNATION,
                        }
                    })
                } else {
                    json!({
                        "binding": {
                            "state": "undelivered",
                            "reason": "endpoint-access-dispatch-unavailable",
                        }
                    })
                };
                rows.push(plane_view(
                    key_of(&relationship),
                    generation,
                    ResourceStatus::Ready,
                    b"{}".to_vec(),
                    Some(layer),
                ));
            }
        }
        rows
    }

    // -- the fixture ---------------------------------------------------------

    struct Fixture {
        ctx: ResourceContext,
        requeue: Arc<RecordingRequeue>,
        /// The scripted runtime plane both the driver's `get_view` and the
        /// effects read, so a test can move the evidence between two passes of
        /// one driver.
        plane: ScriptedPlane,
        /// Every `(target, condition)` registration the driver placed.
        watches: Arc<Mutex<Vec<(ResourceKey, WatchCondition)>>>,
    }

    impl Fixture {
        /// How many registrations of one condition this driver placed on one
        /// target: `1` while the armed pair is still standing on the target,
        /// and a second one only after that target's evidence moved.
        async fn armed(&self, target: &ResourceKey, condition: &WatchCondition) -> usize {
            self.watches
                .lock()
                .await
                .iter()
                .filter(|(key, placed)| key == target && placed == condition)
                .count()
        }

        /// Every registration this driver placed, so a test can assert the
        /// whole armed set rather than one row of it.
        async fn armed_targets(&self) -> Vec<ResourceKey> {
            let mut targets = self
                .watches
                .lock()
                .await
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            targets.sort_by(|left, right| {
                (&left.type_name, &left.name).cmp(&(&right.type_name, &right.name))
            });
            targets.dedup();
            targets
        }
    }

    /// The real interaction driver over the real effects service, reading the
    /// scripted zone plane: the production path end to end, short only of the
    /// manager actor that would publish the verdict onto the row.
    fn fixture(rows: Vec<ResourceView>) -> (Fixture, InteractionDriver<DisplaySession>) {
        let spec = session_spec();
        let row = StoredDesiredResource {
            key: key_of(&session_ref()),
            uid: SESSION_UID,
            generation: SESSION_GENERATION,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting: false,
            spec: serde_json::to_vec(&spec).expect("session spec bytes"),
            metadata: b"{}".to_vec(),
            created_at: 0,
        };
        let plane = ScriptedPlane::new(rows);
        let watches: Arc<Mutex<Vec<(ResourceKey, WatchCondition)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let manager = Arc::new(ChildManager {
            rows: Arc::new(Mutex::new(Vec::new())),
            parent_uid: row.uid,
            plane: plane.clone(),
            watches: Arc::clone(&watches),
        });
        let requeue = Arc::new(RecordingRequeue::default());
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel::<EffectCompleted>();
        let (watch_tx, _watch_rx) = tokio::sync::mpsc::unbounded_channel::<WatchSatisfied>();
        let ctx = ResourceContext::new(
            row,
            spec_decoder(),
            manager,
            Arc::clone(&requeue) as Arc<dyn RequeueScheduler>,
            effects_tx,
            watch_tx,
        );
        let effects =
            InteractionEffectsService::new(scripted_facets_over_plane(zone(), plane.clone()));
        let driver = InteractionDriver::new(InteractionDriverArgs {
            zone: zone(),
            controller_generation: ControllerGeneration::new(3).expect("controller generation"),
            effects: Arc::new(effects),
            behavior: DisplaySession,
        });
        let fixture = Fixture { ctx, requeue, plane, watches };
        (fixture, driver)
    }

    // -- reconcile -----------------------------------------------------------

    /// A session whose children have not converged must not claim readiness.
    /// `Satisfied` is the runtime's "publish `Ready`" verdict and
    /// `RetryScheduled` its "publish `Pending`" verdict, so answering
    /// `Satisfied` over pending children woke every watcher on a session
    /// nothing stood behind.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_session_with_pending_children_does_not_publish_ready() {
        let (mut fixture, mut driver) = fixture(plane(false, true));
        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");

        assert_eq!(
            outcome,
            ReconcileOutcome::RetryScheduled,
            "a session whose children are not Ready publishes `Pending`, never `Ready`"
        );
        let status = fixture
            .ctx
            .status::<InteractionDriverStatus>()
            .expect("the typed status is published");
        assert!(!status.ready, "the typed status agrees with the verdict");
        assert_eq!(
            fixture.ctx.take_status_projection(),
            None,
            "a pending aggregate publishes no projection: nothing is realized to name"
        );
        assert_eq!(
            fixture.requeue.scheduled().len(),
            1,
            "the pass requeues on the type's cadence, so the row is re-driven"
        );
    }

    /// The second half of the aggregate: children that are all Ready but a
    /// relationship that has not been delivered is not a realized session
    /// either - the workers exist, but no admitted carriage stands behind them.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_session_with_an_undelivered_binding_does_not_publish_ready() {
        let (mut fixture, mut driver) = fixture(plane(true, false));
        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");

        assert_eq!(
            outcome,
            ReconcileOutcome::RetryScheduled,
            "an undelivered canonical binding holds the session at `Pending`"
        );
        let status = fixture
            .ctx
            .status::<InteractionDriverStatus>()
            .expect("the typed status is published");
        assert!(!status.ready);
        assert_eq!(fixture.ctx.take_status_projection(), None);
    }

    /// Readiness is published exactly when the aggregate holds, and the row's
    /// `status.resource` carries the typed projection the pass proved rather
    /// than the empty layer a driver that only held it in memory publishes.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_session_publishes_ready_and_its_projection_once_the_aggregate_holds() {
        let (mut fixture, mut driver) = fixture(plane(true, true));
        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");

        assert_eq!(
            outcome,
            ReconcileOutcome::Satisfied,
            "every child Ready and every canonical binding delivered is the whole aggregate"
        );
        let status = fixture
            .ctx
            .status::<InteractionDriverStatus>()
            .expect("the typed status is published")
            .clone();
        assert!(status.ready);
        let projection = fixture
            .ctx
            .take_status_projection()
            .expect("the row's `status.resource` carries the Provider projection, not `{}`");
        assert_eq!(
            &projection,
            status.resource.as_ref().expect("the typed status carries it too"),
            "the wire layer and the typed status are one projection"
        );

        let intents = committed_intents();
        let processes = intents
            .iter()
            .filter(|intent| intent.target().resource_type().as_str() == "Process")
            .map(|intent| intent.target().to_canonical_string())
            .collect::<Vec<_>>();
        // R23: the session's own Endpoint is the GuestFrontend-produced row,
        // derived from the session's uid by the display Provider's durable
        // vocabulary - NOT the first `Endpoint` child a list yields, which is
        // the host compositor socket, and not the host proxy's carriage. This
        // assertion names that derivation on purpose: picking the row
        // positionally here would ratify whatever the projection happens to
        // select, which is the mistake the assertion exists to catch.
        let wayland = d2b_provider_display_wayland::durable_wayland_endpoint_ref(&session_uid())
            .expect("the durable guest frontend Endpoint row")
            .to_canonical_string();
        let compositor =
            d2b_provider_display_wayland::durable_compositor_endpoint_ref(&session_uid())
                .expect("the durable host compositor Endpoint row")
                .to_canonical_string();
        assert_ne!(wayland, compositor, "two different rows");
        let projected = projection
            .pointer("/waylandEndpointRef")
            .and_then(Value::as_str);
        assert_ne!(
            projected,
            Some(compositor.as_str()),
            "the session's endpoint is never the host compositor socket: selecting the first \
             `Endpoint` child a list yields is what put that row here"
        );
        assert_eq!(
            projection.pointer("/proxyProcessRef").and_then(Value::as_str),
            processes.first().map(String::as_str),
            "the projection names the host proxy worker"
        );
        assert_eq!(
            projection
                .pointer("/guestFrontendProcessRef")
                .and_then(Value::as_str),
            processes.get(1).map(String::as_str),
            "the projection names the guest frontend worker"
        );
        assert_eq!(
            projected,
            Some(wayland.as_str()),
            "the projection names the GuestFrontend-produced Endpoint row, at the durable \
             derivation the display Provider owns"
        );
        assert_eq!(
            projection
                .pointer("/waylandEndpointGeneration")
                .and_then(Value::as_u64),
            Some(SESSION_GENERATION),
            "at the Endpoint row's own committed generation"
        );
    }

    /// The subscription is the only thing that makes a post-ready downgrade
    /// observable (R21, AE18, KTD7).
    ///
    /// A pass that published `Ready` without mutating anything requeues
    /// nothing, and an internal registration is satisfied once and then
    /// removed by the target actor - so a driver that armed each target once
    /// and never again is unsubscribed at exactly the moment it needs to be
    /// woken, and keeps publishing `Ready` over a child that withdrew. This
    /// drives one driver across three passes: it arms both conditions on every
    /// child and every dependency, stacks nothing while that evidence stands,
    /// re-arms the pair on a target whose evidence moved, and leaves `Ready`
    /// over the withdrawn child.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_downgraded_child_is_re_armed_and_leaves_the_session_pending() {
        let spec = session_spec();
        let mut expected = vec![
            key_of(spec.guest_ref()),
            key_of(spec.host_ref()),
            key_of(spec.user_ref()),
            key_of(spec.policy_ref()),
        ];
        expected.extend(
            committed_intents()
                .iter()
                .map(|intent| key_of(intent.target())),
        );
        let (mut fixture, mut driver) = fixture(plane(true, true));

        let first = driver.reconcile(&mut fixture.ctx).await;
        assert_eq!(
            first.expect("the first pass"),
            ReconcileOutcome::Satisfied,
            "the aggregate holds, so this pass publishes `Ready`"
        );
        let creating = fixture.requeue.scheduled().len();

        for target in &expected {
            for condition in [WatchCondition::Ready, WatchCondition::ProjectionChanged] {
                assert_eq!(
                    fixture.armed(target, &condition).await,
                    1,
                    "the first pass arms {condition:?} on {target:?}, so an evidence change \
                     under a phase that never moves still wakes this row"
                );
            }
        }

        // A second pass over evidence that stood: the armed registrations are
        // still live on their targets, so a converged pass adds none, and it
        // schedules nothing either - which is exactly why the subscription is
        // the only thing left that can re-drive this row.
        let second = driver.reconcile(&mut fixture.ctx).await;
        assert_eq!(
            second.expect("the second pass"),
            ReconcileOutcome::Satisfied,
            "an unchanged converged pass still publishes `Ready`"
        );
        assert_eq!(
            fixture.requeue.scheduled().len(),
            creating,
            "only the pass that committed its children requeued; a ready pass that mutated \
             nothing schedules nothing, so only a subscription re-drives this row"
        );
        for target in &expected {
            for condition in [WatchCondition::Ready, WatchCondition::ProjectionChanged] {
                assert_eq!(
                    fixture.armed(target, &condition).await,
                    1,
                    "unchanged evidence keeps one live registration on {target:?}, never a stack"
                );
            }
        }

        // The child withdrew while the session stood `Ready`: the plane moves,
        // the row does not, and the third pass has to see it.
        fixture.plane.publish(plane(false, false)).await;
        let third = driver.reconcile(&mut fixture.ctx).await;
        assert_eq!(
            third.expect("the third pass"),
            ReconcileOutcome::RetryScheduled,
            "a session whose children withdrew publishes `Pending` again"
        );
        let status = fixture
            .ctx
            .status::<InteractionDriverStatus>()
            .expect("the typed status is published")
            .clone();
        assert!(
            !status.ready,
            "aggregate readiness is withdrawn with the child"
        );
        let children = committed_intents()
            .iter()
            .map(|intent| key_of(intent.target()))
            .collect::<Vec<_>>();
        for target in &children {
            for condition in [WatchCondition::Ready, WatchCondition::ProjectionChanged] {
                assert_eq!(
                    fixture.armed(target, &condition).await,
                    2,
                    "the registration the child already satisfied is armed again on {target:?}, \
                     so a second downgrade is observed too"
                );
            }
        }
        for target in expected.iter().filter(|target| !children.contains(target)) {
            for condition in [WatchCondition::Ready, WatchCondition::ProjectionChanged] {
                assert_eq!(
                    fixture.armed(target, &condition).await,
                    1,
                    "a dependency whose evidence stood keeps exactly one registration"
                );
            }
        }
        assert_eq!(
            fixture.armed_targets().await,
            {
                let mut targets = expected.clone();
                targets.sort_by(|left, right| {
                    (&left.type_name, &left.name).cmp(&(&right.type_name, &right.name))
                });
                targets.dedup();
                targets
            },
            "every watched row is a child or a dependency this pass derived, and nothing else"
        );
    }
}
