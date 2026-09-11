//! Interaction and shell family drivers (U12 wave 3): the v3 `ResourceDriver`
//! conversion of the U9 display-wayland, audio-pipewire, and shell-terminal
//! shared-Runner family (R3, R4, R30).
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`InteractionDriverFactory`] registration under the
//!   family's six ResourceTypes (the old `U9_SHARED_PROVIDER_RUNNERS` rows
//!   and `Process/*-controller` identities).
//! - `validate_spec` -> [`ResourceDriver::validate`]: the stored spec decodes
//!   and names a Provider this factory owns for the row's ResourceType, plus
//!   the family's structural spec checks (display session cross-domain
//!   trust, audio typed specs, shell reference shapes).
//! - `observe` -> [`ResourceDriver::recover`]: owned-child adoption.
//! - finalizer enrollment + `plan`/`reconcile`/`execute_effect` ->
//!   [`ResourceDriver::reconcile`]: the desired child set is ensured through
//!   the manager child API (committed before the child actor exists, F1),
//!   owned children the desired set no longer derives are retired in the
//!   family's preserved order (endpoint-first / process-last), the typed
//!   Provider effect runs behind [`InteractionDriverEffects`], and the
//!   in-memory status projection is published with `ctx.set_status` (R11)
//!   plus a self-`requeue_after` while the family is not converged.
//! - `prepare_finalize`/`execute_finalize`/`finalize` ->
//!   [`ResourceDriver::delete`]: the family's preserved teardown ordering -
//!   the Provider stage first (audio lease finalization; the service/pool
//!   reference refusals that keep an owner alive while dependents remain),
//!   then the owned children retire and the manager holds the parent row
//!   until the last child is gone (F3, the guarantee the old finalizer
//!   provided).
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11); the old
//!   durable status candidate and its `status.resource` projection stay as
//!   the driver's in-memory projection.
//!
//! Process work never happens here (KTD13): the display host proxy / guest
//! frontend Processes, the audio host effect / guest agent Processes, and the
//! shell supervisor Process are child rows ensured through the manager and
//! launched by the Process driver. This driver has no spawn surface.
//!
//! Cross-resource readiness (a child's or dependency's live phase) is not
//! observable from this driver: `ResourceContext::get`/`children` return the
//! durable row, which carries no status (R11 keeps status actor-local). The
//! driver registers `ctx.watch(.., WatchCondition::Ready)` edges for every
//! dependency and desired child, requeues while its typed effect reports not
//! converged, and never fabricates a readiness it cannot observe; the
//! production effects read the live phase through the manager view (converted
//! rows) or the durable row (unconverted rows).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildIntent;
use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceUid, ZoneId};
use d2b_core_controller::{OwnedChildIntent, materialize_child_create_payload};
use d2b_provider_audio_pipewire::{AudioBindingSpec, AudioServiceSpec};
use d2b_provider_display_wayland::WaylandSessionSpec;
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

/// Preserved reconcile self-resync while a row is not converged (the old
/// shared Runner repair interval per Provider contract).
pub(crate) const DISPLAY_RESYNC: Duration =
    Duration::from_secs(d2b_provider_display_wayland::DISPLAY_REPAIR_INTERVAL_SECS);
pub(crate) const AUDIO_RESYNC: Duration =
    Duration::from_secs(d2b_provider_audio_pipewire::AUDIO_REPAIR_INTERVAL_SECS);
pub(crate) const SHELL_RESYNC: Duration =
    Duration::from_secs(d2b_provider_shell_terminal::SHELL_REPAIR_INTERVAL_SECS);

/// One ResourceType/Provider row of the U9 interaction family.
///
/// The table pins the provider identity every effect call binds: the
/// controller reference, the Provider reference, and the repair cadence the
/// old runner registration carried. It replaces the old runner registration
/// rows; the descriptors themselves are gone with the old Runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InteractionRegistration {
    pub(crate) kind: InteractionKind,
    pub(crate) controller_ref: &'static str,
    pub(crate) provider_ref: &'static str,
    pub(crate) resource_type: &'static str,
    /// Preserved resync cadence (old `repair_interval_ticks`).
    pub(crate) resync: Duration,
    /// Whether the typed spec carries the universal `spec.providerRef` the
    /// old descriptor selected on. Display rows are envelope-only (the old
    /// descriptor kept no exact Provider selector for them); audio and shell
    /// specs carry the field typed.
    pub(crate) spec_provider_selector: bool,
}

/// Closed Provider handler set served by this family (old
/// `SharedProviderResourceKind`, U9 rows only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum InteractionKind {
    DisplayWaylandPolicy,
    DisplayWaylandSession,
    AudioService,
    AudioBinding,
    ShellPool,
    ShellSession,
}

/// The six U9 shared Runner registrations, in the preserved order.
pub(crate) const INTERACTION_REGISTRATIONS: [InteractionRegistration; 6] = [
    InteractionRegistration {
        kind: InteractionKind::DisplayWaylandPolicy,
        controller_ref: "Process/display-wayland-controller",
        provider_ref: "Provider/display-wayland",
        resource_type: "display-wayland.d2bus.org.WaylandPolicy",
        resync: DISPLAY_RESYNC,
        spec_provider_selector: false,
    },
    InteractionRegistration {
        kind: InteractionKind::DisplayWaylandSession,
        controller_ref: "Process/display-wayland-controller",
        provider_ref: "Provider/display-wayland",
        resource_type: "display-wayland.d2bus.org.WaylandSession",
        resync: DISPLAY_RESYNC,
        spec_provider_selector: false,
    },
    InteractionRegistration {
        kind: InteractionKind::AudioService,
        controller_ref: "Process/audio-pipewire-controller",
        provider_ref: "Provider/audio-pipewire",
        resource_type: "audio.d2bus.org.AudioService",
        resync: AUDIO_RESYNC,
        spec_provider_selector: true,
    },
    InteractionRegistration {
        kind: InteractionKind::AudioBinding,
        controller_ref: "Process/audio-pipewire-controller",
        provider_ref: "Provider/audio-pipewire",
        resource_type: "audio.d2bus.org.AudioBinding",
        resync: AUDIO_RESYNC,
        spec_provider_selector: true,
    },
    InteractionRegistration {
        kind: InteractionKind::ShellPool,
        controller_ref: "Process/shell-terminal-controller",
        provider_ref: "Provider/shell-terminal",
        resource_type: "shell-terminal.d2bus.org.ShellPool",
        resync: SHELL_RESYNC,
        spec_provider_selector: true,
    },
    InteractionRegistration {
        kind: InteractionKind::ShellSession,
        controller_ref: "Process/shell-terminal-controller",
        provider_ref: "Provider/shell-terminal",
        resource_type: "shell-terminal.d2bus.org.ShellSession",
        resync: SHELL_RESYNC,
        spec_provider_selector: true,
    },
];

/// Every ResourceType this factory serves (KTD4 Phase A partition).
pub(crate) const INTERACTION_TYPES: [&str; 6] = [
    "display-wayland.d2bus.org.WaylandPolicy",
    "display-wayland.d2bus.org.WaylandSession",
    "audio.d2bus.org.AudioService",
    "audio.d2bus.org.AudioBinding",
    "shell-terminal.d2bus.org.ShellPool",
    "shell-terminal.d2bus.org.ShellSession",
];

/// The interaction Providers whose committed rows select the family (old
/// `U9_PROVIDER_REFS`; the runner lifecycle that consumed it is gone with the
/// conversion, the presence scan in `resource_runtime` still keys on it).
pub(crate) const INTERACTION_PROVIDER_REFS: [&str; 5] = [
    "Provider/display-wayland",
    "Provider/audio-pipewire",
    "Provider/clipboard-wayland",
    "Provider/notification-desktop",
    "Provider/shell-terminal",
];

impl InteractionKind {
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::DisplayWaylandPolicy => 0,
            Self::DisplayWaylandSession => 1,
            Self::AudioService => 2,
            Self::AudioBinding => 3,
            Self::ShellPool => 4,
            Self::ShellSession => 5,
        }
    }

    pub(crate) const fn registration(self) -> InteractionRegistration {
        INTERACTION_REGISTRATIONS[self.index()]
    }

    #[cfg(test)]
    pub(crate) const fn effect_id(self) -> &'static str {
        match self {
            Self::DisplayWaylandPolicy => "display-wayland-policy",
            Self::DisplayWaylandSession => "display-wayland-session",
            Self::AudioService => "audio-service",
            Self::AudioBinding => "audio-binding",
            Self::ShellPool => "shell-pool",
            Self::ShellSession => "shell-session",
        }
    }

    pub(crate) const fn provider_ref(self) -> &'static str {
        self.registration().provider_ref
    }

    #[cfg(test)]
    pub(crate) const fn controller_ref(self) -> &'static str {
        self.registration().controller_ref
    }

    #[cfg(test)]
    pub(crate) const fn resource_type(self) -> &'static str {
        self.registration().resource_type
    }

    pub(crate) const fn resync(self) -> Duration {
        self.registration().resync
    }

    /// The family row for one ResourceType (old `from_registration` keyed on
    /// the runner tuple; the v3 plane knows only the key).
    pub(crate) fn from_resource_type(resource_type: &str) -> Option<Self> {
        INTERACTION_REGISTRATIONS
            .iter()
            .find(|registration| registration.resource_type == resource_type)
            .map(|registration| registration.kind)
    }

    /// Whether the row's spec must carry the typed universal `spec.providerRef`
    /// (old descriptor exact-value selector).
    pub(crate) const fn spec_provider_selector(self) -> bool {
        self.registration().spec_provider_selector
    }
}

/// Result returned by one typed Provider effect adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InteractionEffectPhase {
    Ready,
    Pending,
}

/// One Provider effect outcome: the phase the old effect returned plus the
/// `status.resource` projection the old status candidate published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InteractionEffectOutcome {
    pub(crate) phase: InteractionEffectPhase,
    pub(crate) resource: Option<Value>,
}

impl InteractionEffectOutcome {
    pub(crate) const fn phase(phase: InteractionEffectPhase) -> Self {
        Self {
            phase,
            resource: None,
        }
    }

    pub(crate) fn projection(phase: InteractionEffectPhase, resource: Value) -> Self {
        Self {
            phase,
            resource: Some(resource),
        }
    }
}

/// Outcome of one Provider teardown stage (old `execute_finalize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InteractionFinalize {
    /// Cleanup finished; the owned children may retire.
    Complete,
    /// Cleanup is progressing or refused by a live dependent: re-enter.
    Pending,
}

/// Closed failure surface for interaction Provider adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InteractionEffectError {
    /// The Provider path is not currently available and should retry.
    Unavailable,
    /// Fresh resource or assignment evidence failed closed.
    InvalidResource,
}

impl core::fmt::Display for InteractionEffectError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "interaction-effect-unavailable",
            Self::InvalidResource => "interaction-resource-invalid",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InteractionDriverError {
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

/// Typed in-memory status projection (R11: never persisted). Carries the
/// closed phase the old status candidate published plus the Provider's
/// `status.resource` projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InteractionDriverStatus {
    pub(crate) ready: bool,
    pub(crate) resource: Option<Value>,
}

// ---------------------------------------------------------------------------
// Spec decode (manager-wired)
// ---------------------------------------------------------------------------

/// Decoded interaction spec envelope: the canonical spec document the
/// family's Provider handlers read, envelope-aware so API-created rows
/// (persisted as the full envelope minus status) and Nix rows (persisted as
/// the compiled spec document) decode the same way.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InteractionSpecEnvelope {
    /// The spec document (never the surrounding envelope).
    value: Value,
    /// The Layer 2 base view: the document minus the universal
    /// `providerRef`/`updatePolicy` and the Layer 3 `provider` extension.
    base: Value,
}

impl InteractionSpecEnvelope {
    pub(crate) fn base(&self) -> &Value {
        &self.base
    }

    /// The spec's Provider reference (the family row selector).
    pub(crate) fn provider_ref(&self) -> Option<&str> {
        self.value.get("providerRef").and_then(Value::as_str)
    }

    /// Decode one Layer 2 base spec.
    pub(crate) fn base_spec<T: DeserializeOwned>(&self) -> Result<T, InteractionEffectError> {
        serde_json::from_value(self.base.clone())
            .map_err(|_| InteractionEffectError::InvalidResource)
    }

    /// Decode one typed spec that carries `providerRef` itself (the audio
    /// Provider's typed specs; old `AudioResourceRuntime::decode_spec`).
    pub(crate) fn spec_with_provider_ref<T: DeserializeOwned>(
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
        serde_json::from_value(spec).map_err(|_| InteractionEffectError::InvalidResource)
    }
}

/// Closed decode error for an interaction spec envelope.
#[derive(Debug, thiserror::Error)]
#[error("interaction spec must be a JSON object")]
pub(crate) struct InteractionSpecDecodeError;

/// The manager-wired decode hook for the family's ResourceTypes.
pub(crate) fn interaction_spec_decoder() -> Arc<dyn SpecDecoder> {
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
/// Durable rows only: the live phase is deliberately not carried here (R11),
/// the production effects read it from the manager view or the durable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InteractionChild {
    pub(crate) resource_ref: ResourceRef,
    pub(crate) generation: u64,
}

/// Everything one Provider effect call may read from the driver.
pub(crate) struct InteractionEffectRequest<'a> {
    pub(crate) target: ResourceKey,
    /// The row's durable uid (the Provider effects key on it).
    pub(crate) uid: ResourceUid,
    pub(crate) generation: u64,
    /// The controller generation every effect call binds (KTD7).
    pub(crate) controller_generation: u64,
    /// The Layer 2 base spec document of the row: the stored spec with the
    /// universal `providerRef`/`updatePolicy` and Layer 3 `provider` fields
    /// stripped, so every kind decodes its typed spec without re-deriving
    /// the split.
    pub(crate) spec: Value,
    /// The row's `spec.providerRef` (the family row selector).
    pub(crate) provider_ref: Option<ResourceRef>,
    /// Owned child rows realizing the current desired child set.
    pub(crate) children: &'a [InteractionChild],
}

/// Typed Provider effect boundary owned by the d2bd composition root.
///
/// This is the U9 family's dyn-erased port: the driver sees only these two
/// closed, typed calls, and the production implementation owns the display
/// session admission, the audio controller registry, and the shell
/// reference checks.
#[async_trait]
pub(crate) trait InteractionDriverEffects: Send + Sync + 'static {
    /// Reconcile one display/audio/shell resource through its typed handler.
    async fn reconcile(
        &self,
        kind: InteractionKind,
        request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionEffectOutcome, InteractionEffectError>;

    /// Advance one Provider teardown stage (old `execute_finalize`).
    async fn finalize(
        &self,
        kind: InteractionKind,
        request: &InteractionEffectRequest<'_>,
    ) -> Result<InteractionFinalize, InteractionEffectError>;
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Construction arguments shared by every driver of the family.
#[derive(Clone)]
pub(crate) struct InteractionDriverArgs {
    pub(crate) zone: String,
    pub(crate) controller_generation: ControllerGeneration,
    pub(crate) effects: Arc<dyn InteractionDriverEffects>,
}

/// Factory for the U9 interaction and shell ResourceTypes.
///
/// Construction is infallible by contract (R3).
pub(crate) struct InteractionDriverFactory {
    types: Vec<ResourceTypeName>,
    args: InteractionDriverArgs,
}

impl InteractionDriverFactory {
    pub(crate) fn new(args: InteractionDriverArgs) -> Self {
        Self {
            types: INTERACTION_TYPES
                .iter()
                .map(|resource_type| ResourceTypeName::new(*resource_type))
                .collect(),
            args,
        }
    }
}

impl core::fmt::Debug for InteractionDriverFactory {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InteractionDriverFactory")
            .field("types", &self.types)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ResourceDriverFactory for InteractionDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(InteractionDriver::new(self.args.clone()))
    }
}

/// One desired interaction resource.
pub(crate) struct InteractionDriver {
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    effects: Arc<dyn InteractionDriverEffects>,
    /// Dependency/child keys already watched (R12: exactly once per target).
    watched: Vec<ResourceKey>,
}

impl InteractionDriver {
    pub(crate) fn new(args: InteractionDriverArgs) -> Self {
        let zone = ZoneId::parse(args.zone).expect("driver zone was validated at construction");
        Self {
            zone,
            controller_generation: args.controller_generation,
            effects: args.effects,
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

    /// The family row this row is served by (old `from_registration`).
    fn kind(
        &self,
        ctx: &ResourceContext,
        envelope: &InteractionSpecEnvelope,
        op: DriverOp,
    ) -> Result<InteractionKind, InteractionDriverError> {
        if ctx.key().zone != self.zone.as_str() {
            return Err(self.error(InteractionDriverErrorKind::SpecInvalid, op));
        }
        let Some(kind) = InteractionKind::from_resource_type(&ctx.key().type_name) else {
            return Err(self.error(InteractionDriverErrorKind::SpecInvalid, op));
        };
        if kind.spec_provider_selector()
            && envelope.provider_ref() != Some(kind.provider_ref())
        {
            return Err(self.error(InteractionDriverErrorKind::SpecInvalid, op));
        }
        Ok(kind)
    }

    /// Structural spec checks beyond decode (old `validate_spec` plus the
    /// structural half of the typed effects' admission).
    fn validate_spec(
        &self,
        kind: InteractionKind,
        envelope: &InteractionSpecEnvelope,
        op: DriverOp,
    ) -> Result<(), InteractionDriverError> {
        let invalid = || self.error(InteractionDriverErrorKind::SpecInvalid, op);
        match kind {
            // The policy envelope is the whole contract (old effect: envelope
            // parse only).
            InteractionKind::DisplayWaylandPolicy => {
                envelope.base_spec::<Value>().map_err(|_| invalid())?;
            }
            InteractionKind::DisplayWaylandSession => {
                envelope
                    .base_spec::<WaylandSessionSpec>()
                    .map_err(|_| invalid())?;
            }
            InteractionKind::AudioService => {
                envelope
                    .spec_with_provider_ref::<AudioServiceSpec>()
                    .map_err(|_| invalid())?;
            }
            InteractionKind::AudioBinding => {
                envelope
                    .spec_with_provider_ref::<AudioBindingSpec>()
                    .map_err(|_| invalid())?;
            }
            InteractionKind::ShellPool => {
                shell_pool_spec(envelope.base(), envelope.provider_ref()).map_err(|_| invalid())?;
            }
            InteractionKind::ShellSession => {
                shell_session_execution(envelope.base(), envelope.provider_ref())
                    .map_err(|_| invalid())?;
                shell_session_pool_ref(envelope.base(), envelope.provider_ref())
                    .map_err(|_| invalid())?;
            }
        }
        Ok(())
    }

    /// Register one internal dependency/child watch exactly once per target
    /// (R12/R17).
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

    /// The dependency references one kind's effect reads (old descriptor
    /// dependency selectors plus the per-kind effect reads).
    fn dependency_refs(
        &self,
        kind: InteractionKind,
        envelope: &InteractionSpecEnvelope,
        op: DriverOp,
    ) -> Result<Vec<ResourceRef>, InteractionDriverError> {
        let invalid = || self.error(InteractionDriverErrorKind::SpecInvalid, op);
        let dependencies = match kind {
            InteractionKind::DisplayWaylandPolicy => Vec::new(),
            InteractionKind::DisplayWaylandSession => {
                let spec = envelope
                    .base_spec::<WaylandSessionSpec>()
                    .map_err(|_| invalid())?;
                vec![
                    spec.guest_ref().clone(),
                    spec.host_ref().clone(),
                    spec.user_ref().clone(),
                    spec.policy_ref().clone(),
                ]
            }
            InteractionKind::AudioService => Vec::new(),
            InteractionKind::AudioBinding => {
                let spec = envelope
                    .spec_with_provider_ref::<AudioBindingSpec>()
                    .map_err(|_| invalid())?;
                vec![spec.service_ref.clone(), spec.target_ref.clone()]
            }
            InteractionKind::ShellPool => {
                let (execution_ref, user_ref) =
                    shell_pool_spec(envelope.base(), envelope.provider_ref())
                        .map_err(|_| invalid())?;
                vec![execution_ref, user_ref]
            }
            InteractionKind::ShellSession => {
                let (execution_ref, user_ref) =
                    shell_session_execution(envelope.base(), envelope.provider_ref())
                        .map_err(|_| invalid())?;
                let mut dependencies = vec![shell_session_pool_ref(
                    envelope.base(),
                    envelope.provider_ref(),
                )
                .map_err(|_| invalid())?];
                dependencies.push(execution_ref);
                if let Some(user_ref) = user_ref {
                    dependencies.push(user_ref);
                }
                dependencies
            }
        };
        Ok(dependencies)
    }

    /// The desired child set of one reconcile pass, as manager child rows.
    ///
    /// Providers declare the children they own; the driver materializes them
    /// into the manager's child shape (F1). Kinds whose Provider realizes
    /// nothing through resource rows return an empty set.
    fn desired_children(
        &self,
        ctx: &ResourceContext,
        kind: InteractionKind,
        envelope: &InteractionSpecEnvelope,
        op: DriverOp,
    ) -> Result<Vec<ChildEnsure>, InteractionDriverError> {
        let invalid = || self.error(InteractionDriverErrorKind::SpecInvalid, op);
        match kind {
            InteractionKind::DisplayWaylandPolicy
            | InteractionKind::AudioService
            | InteractionKind::ShellPool => Ok(Vec::new()),
            InteractionKind::DisplayWaylandSession => {
                let spec = envelope
                    .base_spec::<WaylandSessionSpec>()
                    .map_err(|_| invalid())?;
                let session_ref = key_ref(ctx.key());
                let session_uid = resource_uid(ctx.uid())
                    .map_err(|_| self.error(InteractionDriverErrorKind::SpecInvalid, op))?;
                let intents = crate::interaction_composition::display_owned_child_intents(
                    &self.zone,
                    &session_ref,
                    &session_uid,
                    &spec,
                    ctx.generation(),
                    self.controller_generation.get(),
                )
                .map_err(|_| invalid())?;
                intents.iter().map(owned_child_ensure).collect()
            }
            InteractionKind::AudioBinding => {
                let spec = envelope
                    .spec_with_provider_ref::<AudioBindingSpec>()
                    .map_err(|_| invalid())?;
                let set =
                    d2b_provider_audio_pipewire::AudioBindingController::<
                        crate::audio_dispatch::DaemonAudioMediator,
                    >::child_resources(&key_ref(ctx.key()), &spec)
                    .map_err(|_| invalid())?;
                set.iter()
                    .map(|intent| binding_child_ensure(intent, &self.zone, op))
                    .collect()
            }
            InteractionKind::ShellSession => {
                let (execution_ref, user_ref) =
                    shell_session_execution(envelope.base(), envelope.provider_ref())
                        .map_err(|_| invalid())?;
                let user_ref = user_ref.ok_or_else(invalid)?;
                let pool_ref = shell_session_pool_ref(envelope.base(), envelope.provider_ref())
                    .map_err(|_| invalid())?;
                let process_ref = ResourceRef::parse(&format!(
                    "Process/shell-session-{}",
                    ctx.key().name
                ))
                .map_err(|_| invalid())?;
                let process_spec = json!({
                    "providerRef": "Provider/system-systemd",
                    "executionRef": execution_ref.to_canonical_string(),
                    "domain": "user",
                    "userRef": user_ref.to_canonical_string(),
                    "processClass": "service",
                    "template": "shell-supervisor-main",
                    "desiredLifecycle": "running",
                    "deviceUsage": [],
                    "networkUsage": null,
                    "dependencies": [pool_ref.to_canonical_string()],
                });
                Ok(vec![ChildEnsure {
                    type_name: ResourceTypeName::new(process_ref.resource_type().as_str()),
                    name: process_ref.name().as_str().to_owned(),
                    spec: serde_json::to_vec(&process_spec)
                        .map_err(|_| self.error(InteractionDriverErrorKind::SpecInvalid, op))?,
                    metadata: serde_json::to_vec(&json!({
                        "ownerRef": key_ref(ctx.key()).to_canonical_string(),
                        "labels": {},
                        "annotations": {},
                    }))
                    .map_err(|_| self.error(InteractionDriverErrorKind::SpecInvalid, op))?,
                }])
            }
        }
    }

    /// Retire owned children the desired set no longer derives, in the
    /// family's preserved order (endpoint-first, then the producer Process;
    /// old `mutation_order`).
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
        Ok(owned
            .iter()
            .filter(|row| {
                !row.deleting
                    && desired.iter().any(|child| {
                        child.type_name.as_str() == row.key.type_name && child.name == row.key.name
                    })
            })
            .map(|row| InteractionChild {
                resource_ref: key_ref(&row.key),
                generation: row.generation,
            })
            .collect())
    }

    fn effect_error(&self, error: InteractionEffectError, op: DriverOp) -> InteractionDriverError {
        match error {
            InteractionEffectError::InvalidResource => {
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
                .map_err(|_| self.error(InteractionDriverErrorKind::SpecInvalid, op))?,
            generation: ctx.generation(),
            controller_generation: self.controller_generation.get(),
            spec: envelope.base().clone(),
            provider_ref: envelope
                .provider_ref()
                .and_then(|provider_ref| ResourceRef::parse(provider_ref).ok()),
            children,
        })
    }
}

impl core::fmt::Debug for InteractionDriver {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InteractionDriver")
            .field("zone", &self.zone)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ResourceDriver for InteractionDriver {
    type Error = InteractionDriverError;

    fn classify_error(&self, error: &InteractionDriverError) -> DriverFailure {
        match error.kind.class() {
            FailureClass::Retryable => DriverFailure::retryable(error.op),
            FailureClass::Terminal => DriverFailure::terminal(error.op),
        }
    }

    /// Structural validation (old `validate_spec` plus the structural half of
    /// the typed effect admission): the stored spec decodes, names a Provider
    /// this family owns for the row's ResourceType, and satisfies the kind's
    /// shape checks.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Validate;
        let envelope = self.envelope(ctx, op)?;
        let kind = self.kind(ctx, &envelope, op)?;
        self.validate_spec(kind, &envelope, op)
    }

    /// Discovery and adoption on the realization target (F2): child-bearing
    /// kinds adopt when their complete desired child set is already present
    /// and current; kinds that realize nothing through resource rows adopt
    /// their Provider-side realization in reconcile.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let envelope = self.envelope(ctx, op)?;
        let kind = self.kind(ctx, &envelope, op)?;
        let desired = self.desired_children(ctx, kind, &envelope, op)?;
        if desired.is_empty() {
            return Ok(RecoveryOutcome::Adopted);
        }
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(InteractionDriverErrorKind::ChildMutation, op))?;
        let current = desired.iter().all(|child| {
            owned.iter().any(|row| {
                !row.deleting && child.type_name.as_str() == row.key.type_name && child.name == row.key.name
            })
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
        let kind = self.kind(ctx, &envelope, op)?;

        // Dependency edges (R12/R17): the resources this family's effects
        // read are watched so their readiness or death wakes this actor.
        for dependency in self.dependency_refs(kind, &envelope, op)? {
            self.watch_once(ctx, self.child_key(&dependency)).await;
        }

        // Desired child set through the manager child API (F1): every row is
        // committed before its actor exists.
        let desired = self.desired_children(ctx, kind, &envelope, op)?;
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
            .reconcile(kind, &request)
            .await
            .map_err(|error| self.effect_error(error, op))?;

        ctx.set_status(InteractionDriverStatus {
            ready: outcome.phase == InteractionEffectPhase::Ready,
            resource: outcome.resource,
        });
        if mutated || outcome.phase != InteractionEffectPhase::Ready {
            ctx.requeue_after(kind.resync());
        }
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Teardown (old `prepare_finalize` + `execute_finalize` + `finalize`):
    /// the Provider's teardown stage runs first - the audio lease
    /// finalization, and the AudioService/ShellPool refusals that keep an
    /// owner alive while a dependent Binding/Session remains - then the owned
    /// children retire in the family's preserved order. Idempotent under
    /// retry (R10); a malformed spec skips the Provider stage and still
    /// drains the owned children.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        if let Ok(envelope) = self.envelope(ctx, op)
            && let Ok(kind) = self.kind(ctx, &envelope, op)
            && let Ok(request) = self.request(ctx, &envelope, &[], op)
        {
            match self.effects.finalize(kind, &request).await {
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
pub(crate) fn key_ref(key: &ResourceKey) -> ResourceRef {
    ResourceRef::parse(&format!("{}/{}", key.type_name, key.name))
        .expect("manager keys carry canonical resource references")
}

/// Convert one durable 16-byte uid to its canonical identity (the manager
/// persists the uid as bytes; the Provider effects key on the canonical
/// string).
pub(crate) fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, ()> {
    let mut bytes = *bytes;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).map_err(|_| ())
}

/// Old `BindingChildKind` teardown ranks: endpoints retire before their
/// producing processes (old `mutation_order`).
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
fn owned_child_ensure(intent: &OwnedChildIntent) -> Result<ChildEnsure, InteractionDriverError> {
    let invalid = || InteractionDriverError::new(InteractionDriverErrorKind::SpecInvalid, DriverOp::Reconcile);
    let value: Value = serde_json::from_slice(intent.canonical_resource()).map_err(|_| invalid())?;
    let spec = value.get("spec").cloned().ok_or_else(invalid)?;
    let metadata = value.get("metadata").cloned().unwrap_or_else(|| json!({}));
    Ok(ChildEnsure {
        type_name: ResourceTypeName::new(intent.target().resource_type().as_str()),
        name: intent.target().name().as_str().to_owned(),
        spec: serde_json::to_vec(&spec).map_err(|_| invalid())?,
        metadata: serde_json::to_vec(&json!({
            "ownerRef": metadata.get("ownerRef").cloned().unwrap_or(Value::Null),
            "labels": metadata.get("labels").cloned().unwrap_or_else(|| json!({})),
            "annotations": metadata.get("annotations").cloned().unwrap_or_else(|| json!({})),
        }))
        .map_err(|_| invalid())?,
    })
}

/// One Provider-declared Binding child as a manager child row (old Core
/// `materialize_child_create_payload`: Providers declare intent, Core owns
/// the child body, and the Process Provider stays Core-chosen).
fn binding_child_ensure(
    intent: &BindingChildIntent,
    zone: &ZoneId,
    op: DriverOp,
) -> Result<ChildEnsure, InteractionDriverError> {
    let invalid = || InteractionDriverError::new(InteractionDriverErrorKind::SpecInvalid, op);
    let payload = materialize_child_create_payload(intent, zone).map_err(|_| invalid())?;
    let value = serde_json::from_slice::<Value>(&payload).map_err(|_| invalid())?;
    let spec = value.get("spec").cloned().ok_or_else(invalid)?;
    let metadata = json!({
        "ownerRef": intent.owner_ref().to_canonical_string(),
        "labels": {},
        "annotations": {},
    });
    Ok(ChildEnsure {
        type_name: ResourceTypeName::new(intent.kind().resource_type()),
        name: intent.resource_ref().name().as_str().to_owned(),
        spec: serde_json::to_vec(&spec).map_err(|_| invalid())?,
        metadata: serde_json::to_vec(&metadata).map_err(|_| invalid())?,
    })
}

/// The shell pool execution and user references (old `shell_pool_spec`).
pub(crate) fn shell_pool_spec(
    base: &Value,
    provider_ref: Option<&str>,
) -> Result<(ResourceRef, ResourceRef), InteractionEffectError> {
    if provider_ref != Some("Provider/shell-terminal") {
        return Err(InteractionEffectError::InvalidResource);
    }
    let execution_ref = spec_ref(base, "/executionRef")?;
    if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
        return Err(InteractionEffectError::InvalidResource);
    }
    let user_ref = spec_ref(base, "/userRef")?;
    let login_shell = base
        .pointer("/loginShellRef")
        .and_then(Value::as_str)
        .ok_or(InteractionEffectError::InvalidResource)?;
    if user_ref.resource_type().as_str() != "User"
        || !login_shell.starts_with("artifact://")
        || login_shell.len() > 255
    {
        return Err(InteractionEffectError::InvalidResource);
    }
    Ok((execution_ref, user_ref))
}

/// The shell session execution and optional user references (old
/// `shell_execution`).
pub(crate) fn shell_session_execution(
    base: &Value,
    provider_ref: Option<&str>,
) -> Result<(ResourceRef, Option<ResourceRef>), InteractionEffectError> {
    if provider_ref != Some("Provider/shell-terminal") {
        return Err(InteractionEffectError::InvalidResource);
    }
    let execution_ref = spec_ref(base, "/executionRef")?;
    if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
        return Err(InteractionEffectError::InvalidResource);
    }
    let user_ref = base
        .pointer("/userRef")
        .and_then(Value::as_str)
        .map(ResourceRef::parse)
        .transpose()
        .map_err(|_| InteractionEffectError::InvalidResource)?;
    if user_ref
        .as_ref()
        .is_some_and(|reference| reference.resource_type().as_str() != "User")
        || base
            .pointer("/loginShellRef")
            .and_then(Value::as_str)
            .is_none_or(|shell| !shell.starts_with("artifact://") || shell.len() > 255)
    {
        return Err(InteractionEffectError::InvalidResource);
    }
    Ok((execution_ref, user_ref))
}

/// The session's pool reference (old inline check in `reconcile_shell`).
pub(crate) fn shell_session_pool_ref(
    base: &Value,
    provider_ref: Option<&str>,
) -> Result<ResourceRef, InteractionEffectError> {
    if provider_ref != Some("Provider/shell-terminal") {
        return Err(InteractionEffectError::InvalidResource);
    }
    let pool_ref = spec_ref(base, "/poolRef")?;
    if pool_ref.resource_type().as_str() != "shell-terminal.d2bus.org.ShellPool" {
        return Err(InteractionEffectError::InvalidResource);
    }
    Ok(pool_ref)
}

fn spec_ref(value: &Value, path: &str) -> Result<ResourceRef, InteractionEffectError> {
    value
        .pointer(path)
        .and_then(Value::as_str)
        .and_then(|reference| ResourceRef::parse(reference).ok())
        .ok_or(InteractionEffectError::InvalidResource)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use d2b_resource_runtime::context::{
        ManagerEndpoint, RequeueId, RequeueScheduler, WatchId, WatchRegistration,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::target::TargetHandle;

    use super::*;

    type Log = Arc<parking_lot::Mutex<Vec<String>>>;

    // -- fakes ---------------------------------------------------------------

    /// Scripted typed effects over the caller's ordered log, so the tests
    /// assert one sequence across manager calls and Provider effects.
    struct ScriptedEffects {
        log: Log,
        ready: AtomicBool,
        finalize_pending: AtomicBool,
    }

    impl ScriptedEffects {
        fn shared(log: Log) -> Arc<Self> {
            Arc::new(Self {
                log,
                ready: AtomicBool::new(false),
                finalize_pending: AtomicBool::new(false),
            })
        }

        fn make_ready(&self) {
            self.ready.store(true, Ordering::SeqCst);
        }

        fn hold_finalize(&self) {
            self.finalize_pending.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl InteractionDriverEffects for ScriptedEffects {
        async fn reconcile(
            &self,
            kind: InteractionKind,
            _request: &InteractionEffectRequest<'_>,
        ) -> Result<InteractionEffectOutcome, InteractionEffectError> {
            self.log.lock().push(format!("effect:{}", kind.effect_id()));
            if self.ready.load(Ordering::SeqCst) {
                Ok(InteractionEffectOutcome::projection(
                    InteractionEffectPhase::Ready,
                    json!({"phase": "Ready"}),
                ))
            } else {
                Ok(InteractionEffectOutcome::phase(InteractionEffectPhase::Pending))
            }
        }

        async fn finalize(
            &self,
            kind: InteractionKind,
            _request: &InteractionEffectRequest<'_>,
        ) -> Result<InteractionFinalize, InteractionEffectError> {
            self.log.lock().push(format!("finalize:{}", kind.effect_id()));
            if self.finalize_pending.load(Ordering::SeqCst) {
                Ok(InteractionFinalize::Pending)
            } else {
                Ok(InteractionFinalize::Complete)
            }
        }
    }

    /// Recording manager endpoint over one shared ordered log. Rows are keyed
    /// by `zone/type/name`; the parent row is the fixture row.
    #[derive(Clone)]
    struct RecordingManager {
        log: Log,
        rows: Arc<parking_lot::Mutex<Vec<StoredDesiredResource>>>,
        parent_uid: [u8; 16],
        watches: Arc<parking_lot::Mutex<Vec<ResourceKey>>>,
    }

    impl RecordingManager {
        fn new(log: Log, parent_uid: [u8; 16]) -> Self {
            Self {
                log,
                rows: Arc::new(parking_lot::Mutex::new(Vec::new())),
                parent_uid,
                watches: Arc::new(parking_lot::Mutex::new(Vec::new())),
            }
        }

        fn seed_owned(&self, key: ResourceKey, deleting: bool) {
            self.rows.lock().push(StoredDesiredResource {
                key,
                uid: [0x77; 16],
                generation: 1,
                owner_uid: Some(self.parent_uid),
                provenance: ResourceProvenance::Resource,
                deleting,
                spec: Vec::new(),
                metadata: Vec::new(),
                created_at: 0,
            });
        }

        fn owned_keys(&self) -> Vec<ResourceKey> {
            self.rows
                .lock()
                .iter()
                .filter(|row| row.owner_uid == Some(self.parent_uid))
                .map(|row| row.key.clone())
                .collect()
        }
    }

    #[async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            let key = ResourceKey::new("work", child.type_name.as_str(), child.name.clone());
            self.log.lock().push(format!("ensure:{}/{}", key.type_name, key.name));
            let mut rows = self.rows.lock();
            match rows.iter_mut().find(|row| row.key == key) {
                Some(row) => {
                    row.generation += 1;
                    row.spec = child.spec;
                    row.metadata = child.metadata;
                    Ok(EnsureOutcome::Updated(row.clone()))
                }
                None => {
                    rows.push(StoredDesiredResource {
                        key: key.clone(),
                        uid: [0x88; 16],
                        generation: 1,
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
            self.log.lock().push(format!("get:{}/{}", key.type_name, key.name));
            Ok(self.rows.lock().iter().find(|row| row.key == *key).cloned())
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
            self.log.lock().push(format!("delete:{}/{}", key.type_name, key.name));
            if let Some(row) = self.rows.lock().iter_mut().find(|row| row.key == *key) {
                row.deleting = true;
            }
            Ok(())
        }

        async fn list_owned(
            &self,
            owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            self.log.lock().push("list-owned".to_owned());
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
            registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            self.log.lock().push(format!(
                "watch:{}/{}",
                registration.target.type_name, registration.target.name
            ));
            self.watches.lock().push(registration.target);
            Ok(WatchId(self.watches.lock().len() as u64))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    /// Requeue scheduler recording schedules over the shared log.
    struct RecordingRequeue {
        log: Log,
        schedules: AtomicUsize,
    }

    impl RequeueScheduler for RecordingRequeue {
        fn schedule(&self, _key: ResourceKey, after: Duration) -> RequeueId {
            self.log.lock().push(format!("requeue:{}ms", after.as_millis()));
            RequeueId(self.schedules.fetch_add(1, Ordering::SeqCst) as u64)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    struct Fixture {
        ctx: ResourceContext,
        log: Log,
        effects: Arc<ScriptedEffects>,
        manager: RecordingManager,
    }

    fn fixture(row: StoredDesiredResource) -> Fixture {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let effects = ScriptedEffects::shared(Arc::clone(&log));
        let manager = RecordingManager::new(Arc::clone(&log), row.uid);
        let requeue = RecordingRequeue {
            log: Arc::clone(&log),
            schedules: AtomicUsize::new(0),
        };
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = ResourceContext::new(
            row,
            TargetHandle::Host,
            interaction_spec_decoder(),
            Arc::new(manager.clone()),
            Arc::new(requeue),
            effects_tx,
            notify_tx,
        );
        Fixture {
            ctx,
            log,
            effects,
            manager,
        }
    }

    fn driver(effects: Arc<ScriptedEffects>) -> InteractionDriver {
        InteractionDriver::new(InteractionDriverArgs {
            zone: "work".to_owned(),
            controller_generation: ControllerGeneration::new(3).unwrap(),
            effects,
        })
    }

    fn row(type_name: &str, name: &str, spec: Value) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", type_name, name),
            uid: [0x42; 16],
            generation: 4,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting: false,
            spec: serde_json::to_vec(&spec).unwrap(),
            metadata: b"{}".to_vec(),
            created_at: 0,
        }
    }

    fn display_session_row() -> StoredDesiredResource {
        let spec = WaylandSessionSpec::new(
            ResourceRef::parse("Guest/workstation").unwrap(),
            ResourceRef::parse("Host/host-system").unwrap(),
            ResourceRef::parse("User/alice").unwrap(),
            ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/policy").unwrap(),
            d2b_provider_display_wayland::DisplayIdentity::new(
                "display",
                "#112233",
                "#223344",
                "#334455",
            )
            .unwrap(),
            true,
        )
        .unwrap();
        row(
            "display-wayland.d2bus.org.WaylandSession",
            "display",
            serde_json::to_value(&spec).unwrap(),
        )
    }

    fn audio_binding_row() -> StoredDesiredResource {
        let spec = AudioBindingSpec::new(
            ResourceRef::parse("audio.d2bus.org.AudioService/host-audio").unwrap(),
            ResourceRef::parse("Guest/audio-vm").unwrap(),
            "work",
        )
        .unwrap();
        row(
            "audio.d2bus.org.AudioBinding",
            "guest-audio",
            serde_json::to_value(&spec).unwrap(),
        )
    }

    fn shell_session_row() -> StoredDesiredResource {
        row(
            "shell-terminal.d2bus.org.ShellSession",
            "session-1",
            json!({
                "providerRef": "Provider/shell-terminal",
                "executionRef": "Host/host-system",
                "userRef": "User/alice",
                "loginShellRef": "artifact://shell",
                "poolRef": "shell-terminal.d2bus.org.ShellPool/pool",
            }),
        )
    }

    // -- tests ---------------------------------------------------------------

    #[test]
    fn factory_registers_the_six_family_resource_types() {
        let effects = ScriptedEffects::shared(Arc::new(parking_lot::Mutex::new(Vec::new())));
        let factory = InteractionDriverFactory::new(InteractionDriverArgs {
            zone: "work".to_owned(),
            controller_generation: ControllerGeneration::new(3).unwrap(),
            effects,
        });
        let registered = factory
            .resource_types()
            .iter()
            .map(|resource_type| resource_type.as_str().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(registered, INTERACTION_TYPES.to_vec());
        for registration in INTERACTION_REGISTRATIONS {
            assert_eq!(
                InteractionKind::from_resource_type(registration.resource_type),
                Some(registration.kind)
            );
            assert_eq!(registration.kind.provider_ref(), registration.provider_ref);
            assert_eq!(registration.kind.controller_ref(), registration.controller_ref);
            assert_eq!(registration.kind.resource_type(), registration.resource_type);
        }
    }

    #[tokio::test]
    async fn validate_accepts_an_envelope_spec_without_a_provider_selector() {
        // Display rows are envelope-only (the old descriptor kept no exact
        // Provider selector for them): the session spec decodes without a
        // universal providerRef.
        let mut fixture = fixture(display_session_row());
        let mut driver = driver(Arc::clone(&fixture.effects));
        ResourceDriver::validate(&mut driver, &mut fixture.ctx).await.unwrap();
    }

    #[tokio::test]
    async fn validate_rejects_a_foreign_provider_ref_as_terminal() {
        let mut row = shell_session_row();
        row.spec = serde_json::to_vec(&json!({
            "providerRef": "Provider/display-wayland",
            "executionRef": "Host/host-system",
            "userRef": "User/alice",
            "loginShellRef": "artifact://shell",
            "poolRef": "shell-terminal.d2bus.org.ShellPool/pool",
        }))
        .unwrap();
        let mut fixture = fixture(row);
        let mut driver = driver(Arc::clone(&fixture.effects));
        let failure = ResourceDriver::validate(&mut driver, &mut fixture.ctx).await.unwrap_err();
        assert_eq!(driver.classify_error(&failure).class(), FailureClass::Terminal);
        assert_eq!(driver.classify_error(&failure).op(), DriverOp::Validate);
    }

    #[tokio::test]
    async fn validate_rejects_a_malformed_shell_reference_shape() {
        let mut row = shell_session_row();
        row.spec = serde_json::to_vec(&json!({
            "providerRef": "Provider/shell-terminal",
            "executionRef": "User/alice",
            "userRef": "User/alice",
            "loginShellRef": "artifact://shell",
            "poolRef": "shell-terminal.d2bus.org.ShellPool/pool",
        }))
        .unwrap();
        let mut fixture = fixture(row);
        let mut driver = driver(Arc::clone(&fixture.effects));
        let failure = ResourceDriver::validate(&mut driver, &mut fixture.ctx).await.unwrap_err();
        assert_eq!(driver.classify_error(&failure).class(), FailureClass::Terminal);
    }

    #[tokio::test]
    async fn reconcile_ensures_desired_children_then_runs_the_effect_and_requeues() {
        let mut fixture = fixture(audio_binding_row());
        let mut driver = driver(Arc::clone(&fixture.effects));

        let outcome = ResourceDriver::reconcile(&mut driver, &mut fixture.ctx).await.unwrap();
        assert_eq!(outcome, ReconcileOutcome::Satisfied);

        // The four audio Binding children ride the manager child API (F1)
        // before the typed effect, and the not-ready phase requeues on the
        // Provider's preserved cadence.
        let log = fixture.log.lock().clone();
        let effect_at = log
            .iter()
            .position(|entry| entry == "effect:audio-binding")
            .expect("typed effect ran");
        assert_eq!(log.iter().filter(|entry| entry.starts_with("ensure:")).count(), 4);
        assert!(
            log[..effect_at].iter().all(|entry| !entry.starts_with("effect:")),
            "every ensure is committed before the effect: {log:?}"
        );
        assert_eq!(log.last().map(String::as_str), Some("requeue:300000ms"));
        let status = fixture.ctx.status::<InteractionDriverStatus>().unwrap();
        assert!(!status.ready);
    }

    #[tokio::test]
    async fn reconcile_projects_ready_status_and_registers_each_watch_once() {
        let mut fixture = fixture(display_session_row());
        let mut driver = driver(Arc::clone(&fixture.effects));
        fixture.effects.make_ready();

        ResourceDriver::reconcile(&mut driver, &mut fixture.ctx).await.unwrap();
        ResourceDriver::reconcile(&mut driver, &mut fixture.ctx).await.unwrap();

        let status = fixture.ctx.status::<InteractionDriverStatus>().unwrap();
        assert!(status.ready);
        assert_eq!(status.resource, Some(json!({"phase": "Ready"})));
        let watches = fixture.manager.watches.lock();
        let mut unique = watches.clone();
        unique.sort_by(|left, right| {
            (left.type_name.as_str(), left.name.as_str())
                .cmp(&(right.type_name.as_str(), right.name.as_str()))
        });
        unique.dedup();
        assert_eq!(unique.len(), watches.len(), "one watch per target: {watches:?}");
        // Guest/Host/User/WaylandPolicy dependencies plus the four children.
        assert_eq!(watches.len(), 8);
    }

    #[tokio::test]
    async fn reconcile_retires_obsolete_children_endpoint_first() {
        let mut fixture = fixture(audio_binding_row());
        let mut driver = driver(Arc::clone(&fixture.effects));
        fixture.manager.seed_owned(
            ResourceKey::new("work", "Process", "stale-worker"),
            false,
        );
        fixture
            .manager
            .seed_owned(ResourceKey::new("work", "Endpoint", "stale-endpoint"), false);
        fixture.manager.seed_owned(
            ResourceKey::new("work", "Process", "already-deleting"),
            true,
        );

        ResourceDriver::reconcile(&mut driver, &mut fixture.ctx).await.unwrap();

        let log = fixture.log.lock().clone();
        let deletes = log
            .iter()
            .filter(|entry| entry.starts_with("delete:"))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            deletes,
            vec![
                "delete:Endpoint/stale-endpoint".to_owned(),
                "delete:Process/stale-worker".to_owned(),
            ],
            "endpoint-first / process-last, already-deleting rows left alone: {log:?}"
        );
    }

    #[tokio::test]
    async fn recover_adopts_when_every_desired_child_is_owned_and_missing_otherwise() {
        let mut fixture = fixture(shell_session_row());
        let mut driver = driver(Arc::clone(&fixture.effects));
        assert_eq!(
            ResourceDriver::recover(&mut driver, &mut fixture.ctx).await.unwrap(),
            RecoveryOutcome::Missing
        );
        fixture.manager.seed_owned(
            ResourceKey::new("work", "Process", "shell-session-session-1"),
            false,
        );
        assert_eq!(
            ResourceDriver::recover(&mut driver, &mut fixture.ctx).await.unwrap(),
            RecoveryOutcome::Adopted
        );
        // A child already marked deleting is not current.
        fixture.manager.seed_owned(
            ResourceKey::new("work", "Process", "shell-session-session-1"),
            true,
        );
        // The first (live) row still matches; deleting duplicates are ignored.
        assert_eq!(
            ResourceDriver::recover(&mut driver, &mut fixture.ctx).await.unwrap(),
            RecoveryOutcome::Adopted
        );
    }

    #[tokio::test]
    async fn delete_runs_the_provider_stage_then_retires_every_owned_child() {
        let mut fixture = fixture(audio_binding_row());
        let mut driver = driver(Arc::clone(&fixture.effects));
        fixture
            .manager
            .seed_owned(ResourceKey::new("work", "Endpoint", "endpoint"), false);
        fixture
            .manager
            .seed_owned(ResourceKey::new("work", "Process", "worker"), false);

        ResourceDriver::delete(&mut driver, &mut fixture.ctx).await.unwrap();

        let log = fixture.log.lock().clone();
        assert_eq!(log[0], "finalize:audio-binding");
        assert_eq!(
            log.iter()
                .filter(|entry| entry.starts_with("delete:"))
                .cloned()
                .collect::<Vec<_>>(),
            vec!["delete:Endpoint/endpoint".to_owned(), "delete:Process/worker".to_owned()]
        );
    }

    #[tokio::test]
    async fn delete_is_retryable_while_the_provider_stage_is_pending() {
        let mut fixture = fixture(display_session_row());
        let mut driver = driver(Arc::clone(&fixture.effects));
        fixture.effects.hold_finalize();
        fixture
            .manager
            .seed_owned(ResourceKey::new("work", "Process", "proxy"), false);

        let failure = ResourceDriver::delete(&mut driver, &mut fixture.ctx).await.unwrap_err();
        assert_eq!(driver.classify_error(&failure).class(), FailureClass::Retryable);
        assert_eq!(driver.classify_error(&failure).op(), DriverOp::Delete);
        assert!(fixture.manager.owned_keys().iter().all(|key| {
            fixture.manager.rows.lock().iter().any(|row| row.key == *key && !row.deleting)
        }));
        assert!(
            !fixture
                .log
                .lock()
                .iter()
                .any(|entry| entry.starts_with("delete:"))
        );
    }

    #[tokio::test]
    async fn reconcile_has_no_spawn_surface_beyond_the_manager_child_api() {
        // KTD13: process work rides the manager child rows; the driver's only
        // mutation verbs are ensure/delete on the owned-child API.
        let mut fixture = fixture(display_session_row());
        let mut driver = driver(Arc::clone(&fixture.effects));
        ResourceDriver::reconcile(&mut driver, &mut fixture.ctx).await.unwrap();
        let log = fixture.log.lock().clone();
        assert!(
            log.iter().all(|entry| entry.starts_with("ensure:")
                || entry.starts_with("delete:")
                || entry.starts_with("list-owned")
                || entry.starts_with("watch:")
                || entry.starts_with("requeue:")
                || entry.starts_with("effect:")),
            "no spawn-shaped call may appear: {log:?}"
        );
        assert_eq!(
            log.iter().filter(|entry| entry.starts_with("ensure:")).count(),
            4,
            "the four display children are committed through the manager: {log:?}"
        );
    }
}
