//! NixosGeneration resource driver (U12): the v3 `ResourceDriver`
//! conversion of the daemon-owned activation path (R3, R4, R30; KTD7,
//! KTD13).
//!
//! The driver keeps the preserved activation behavior and nothing else: the
//! pure `ActivationController` policy decides whether an activation runner
//! is planned, a Host target dispatches the preserved
//! `ApplyHostGenerationHandoff` broker effect through the effects port, and
//! a Guest target mints the activation-runner `EphemeralProcess` as an owned
//! child through the manager (KTD13: the runner is a Process resource and
//! the owning controller never spawns; the launch parameters travel on the
//! sanctioned typed channel - the EphemeralProcess spec's `activationInput`
//! - never as argv).
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`ActivationDriverFactory`] registration under
//!   `activation-nixos.d2bus.org.NixosGeneration`.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`] (rejoin the owned runner).
//! - `plan`/`reconcile`/`execute_effect` -> [`ResourceDriver::reconcile`].
//! - `finalize`/`execute_finalize` -> [`ResourceDriver::delete`]: the
//!   durable deleting mark is the manager's (R10), so the old finalizer
//!   dance is not part of the new plane; the owned runner retires first
//!   (F3).
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11).
//!
//! Three preserved behaviors do not map onto one resource's context and are
//! reported rather than invented:
//! 1. Generation retention (`ActivationController::retention_plan`) needs
//!    the sibling generations of one `executionRef`; the new
//!    `ResourceContext` exposes only this resource and its owned children
//!    (no zone-wide list), so surplus pruning stays a desired-state concern
//!    (the bundle-ingest `remove` path) rather than a driver effect.
//! 2. A runner child's terminal phase and outcome code are actor-local
//!    (R11): a parent cannot read another actor's status, so the driver
//!    rejoins an in-flight runner and waits (`WatchCondition::Ready`
//!    dependency edge) instead of classifying its outcome. No success is
//!    ever fabricated; the projection stays non-terminal until the
//!    child-status surface exists.
//! 3. The typed activation status projection is in-memory (R11); its
//!    durable `status.resource.activationDetail` publication belongs to the
//!    manager view model, not this driver.
#![allow(dead_code)]

use std::sync::Arc;

use d2b_contracts_broker::broker_wire::{
    ApplyHostGenerationHandoffResponse, BrokerCallerRole, BrokerRequest, BrokerResponse,
};
use d2b_contracts_broker::host_generation::{
    ApplyHostGenerationHandoff, HandoffCallerRole, HandoffState, HostGenerationHandoffIntent,
    SourceGenerationCompatibilityFloorV1, target_fingerprint,
};
use d2b_contracts_resource::v3::{
    ActivationDetail, ActivationMode, ActivationOutcomeCode, NIXOS_GENERATION_RESOURCE_TYPE,
    NixosGenerationSpec, ResourcePhase, ResourceRef,
};
use d2b_provider_activation_nixos::{
    ActivationApplicationVerifier, ActivationCaller, ActivationController, CallerRole,
    GenerationObservation, GenerationPhase, RunnerRequest, activation_runner_ref,
    activation_runner_spec,
};
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};

use crate::{ServerState, dispatch_broker_request_as};

/// The one resource type this factory serves (KTD4 Phase A).
pub(crate) const ACTIVATION_TYPE_NAME: &str = NIXOS_GENERATION_RESOURCE_TYPE;

/// The activation-runner child resource type (old `create_runner`).
const RUNNER_TYPE_NAME: &str = "EphemeralProcess";

/// The Process Provider the runner is minted under (old `create_runner`).
const RUNNER_PROVIDER_REF: &str = "Provider/system-minijail";

/// Preserved provider retention window (`d2b.providers.activationNixos.
/// retainedGenerations` default). The policy object carries it; surplus
/// pruning itself is a desired-state concern in the new plane (see the
/// module note).
const RETAINED_GENERATIONS: usize = 3;

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationDriverErrorKind {
    /// The durable spec did not decode as the closed generation contract.
    SpecInvalid,
    /// The pure activation policy refused the resource (or its prior
    /// generation reference).
    Policy,
    /// The manager refused a child mutation or the owned-child read.
    ChildMutation,
}

impl core::fmt::Display for ActivationDriverErrorKind {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::SpecInvalid => "activation-spec-invalid",
            Self::Policy => "activation-policy-refused",
            Self::ChildMutation => "activation-child-mutation-failed",
        })
    }
}

/// Typed driver failure; redacted at the erased boundary through
/// [`ResourceDriver::classify_error`] (R13).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ActivationDriverError {
    kind: ActivationDriverErrorKind,
    op: DriverOp,
}

impl ActivationDriverError {
    fn new(kind: ActivationDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }
}

impl core::fmt::Display for ActivationDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.kind.fmt(formatter)
    }
}

impl std::error::Error for ActivationDriverError {}

/// The in-memory status projection (R11: never persisted). Carries exactly
/// the three closed fields the old durable projection wrote, so no
/// free-form (or credential-bearing) value can reach it by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActivationDriverStatus {
    phase: ResourcePhase,
    detail: ActivationDetail,
    outcome: Option<ActivationOutcomeCode>,
}

impl ActivationDriverStatus {
    /// A runner resource is staged but has not settled yet.
    const fn staged() -> Self {
        Self {
            phase: ResourcePhase::Pending,
            detail: ActivationDetail::Staged,
            outcome: None,
        }
    }

    /// The staged runner is being applied (old `Pending`/`applying`).
    const fn applying() -> Self {
        Self {
            phase: ResourcePhase::Pending,
            detail: ActivationDetail::Applying,
            outcome: None,
        }
    }

    /// One applied policy outcome, projected exactly as the old
    /// `publish_status` projected it.
    const fn projected(
        phase: ResourcePhase,
        detail: ActivationDetail,
        outcome: Option<ActivationOutcomeCode>,
    ) -> Self {
        Self {
            phase,
            detail,
            outcome,
        }
    }

    /// Universal phase of the last projection.
    pub(crate) const fn phase(self) -> ResourcePhase {
        self.phase
    }

    /// Typed activation detail of the last projection.
    pub(crate) const fn detail(self) -> ActivationDetail {
        self.detail
    }

    /// Terminal outcome code of the last projection, when one was reached.
    pub(crate) const fn outcome(self) -> Option<ActivationOutcomeCode> {
        self.outcome
    }
}

// ---------------------------------------------------------------------------
// Spec decode hook
// ---------------------------------------------------------------------------

/// The manager-wired decode hook for `NixosGeneration` rows. The spec
/// contract itself enforces the Provider reference, the Host/Guest
/// execution target, the artifact identifier, and the prior-generation
/// type, so a successful decode is the whole validation fence.
pub(crate) fn activation_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| serde_json::from_slice::<NixosGenerationSpec>(bytes))
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The preserved broker response reduced to the fields the outcome mapping
/// reads (old `host_handoff_outcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostHandoffResult {
    /// The coordinator recorded completion of a strict generation
    /// transition.
    Completed {
        source_generation: u64,
        target_generation: u64,
    },
    /// The request was refused before mutation.
    Refused,
    /// Source was preserved after refusal or rollback.
    RolledBack,
    /// No terminal coordinator state was reached, the broker refused the
    /// call, or the dispatch itself failed.
    Incomplete,
}

/// Provider-facing effect surface the activation driver needs. The
/// production implementation dispatches the preserved broker request; test
/// doubles implement the same seam (R4).
#[async_trait::async_trait]
pub(crate) trait ActivationDriverEffects: Send + Sync + 'static {
    /// Dispatch one `ApplyHostGenerationHandoff` for the authenticated
    /// execution target.
    async fn apply_host_generation_handoff(
        &self,
        target: ResourceRef,
        intent: HostGenerationHandoffIntent,
    ) -> HostHandoffResult;
}

/// Production effects over the preserved broker boundary (old
/// `execute_host_handoff` dispatch): caller role `Lifecycle` on the typed
/// request, admin-uid daemon caller on the dispatch.
pub(crate) struct ProductionActivationDriverEffects {
    state: Arc<ServerState>,
}

impl ProductionActivationDriverEffects {
    pub(crate) fn new(state: Arc<ServerState>) -> Self {
        Self { state }
    }
}

impl core::fmt::Debug for ProductionActivationDriverEffects {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProductionActivationDriverEffects")
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl ActivationDriverEffects for ProductionActivationDriverEffects {
    async fn apply_host_generation_handoff(
        &self,
        target: ResourceRef,
        intent: HostGenerationHandoffIntent,
    ) -> HostHandoffResult {
        let request = BrokerRequest::ApplyHostGenerationHandoff(ApplyHostGenerationHandoff {
            caller_role: HandoffCallerRole::Lifecycle,
            target,
            intent,
        });
        match dispatch_broker_request_as(
            &self.state,
            request,
            BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
        ) {
            Ok(BrokerResponse::ApplyHostGenerationHandoff(response)) => {
                host_handoff_result(&response)
            }
            Ok(BrokerResponse::Error(_)) | Ok(_) | Err(_) => HostHandoffResult::Incomplete,
        }
    }
}

fn host_handoff_result(response: &ApplyHostGenerationHandoffResponse) -> HostHandoffResult {
    match response.state {
        HandoffState::Completed => HostHandoffResult::Completed {
            source_generation: response.source_generation,
            target_generation: response.target_generation,
        },
        HandoffState::Refused => HostHandoffResult::Refused,
        HandoffState::RolledBack => HostHandoffResult::RolledBack,
        _ => HostHandoffResult::Incomplete,
    }
}

/// Preserved outcome mapping: a recorded completion is success unless the
/// coordinator reported no strict generation transition.
fn host_handoff_outcome(result: HostHandoffResult) -> ActivationOutcomeCode {
    match result {
        HostHandoffResult::Completed {
            source_generation,
            target_generation,
        } => {
            if target_generation == source_generation {
                ActivationOutcomeCode::StaleGeneration
            } else {
                ActivationOutcomeCode::Succeeded
            }
        }
        HostHandoffResult::Refused => ActivationOutcomeCode::HelperRefused,
        HostHandoffResult::RolledBack => ActivationOutcomeCode::RolledBack,
        HostHandoffResult::Incomplete => ActivationOutcomeCode::HelperFailed,
    }
}

/// Preserved detail projection (old `activation_detail`).
fn activation_detail(
    mode: ActivationMode,
    outcome: ActivationOutcomeCode,
    phase: ResourcePhase,
) -> ActivationDetail {
    if outcome == ActivationOutcomeCode::Adopted {
        return ActivationDetail::Adopted;
    }
    if outcome == ActivationOutcomeCode::RolledBack {
        return ActivationDetail::RolledBack;
    }
    if outcome.is_success() {
        return match mode {
            ActivationMode::Boot => ActivationDetail::BootDefault,
            ActivationMode::Switch | ActivationMode::Test => ActivationDetail::Applied,
            ActivationMode::Adopt => ActivationDetail::Adopted,
        };
    }
    if phase == ResourcePhase::Ready {
        ActivationDetail::Superseded
    } else {
        ActivationDetail::Planning
    }
}

/// Preserved phase projection (old `generation_phase`).
fn generation_phase(phase: ResourcePhase) -> GenerationPhase {
    match phase {
        ResourcePhase::Pending => GenerationPhase::Pending,
        ResourcePhase::Ready => GenerationPhase::Ready,
        ResourcePhase::Succeeded => GenerationPhase::Succeeded,
        ResourcePhase::Failed => GenerationPhase::Failed,
        ResourcePhase::Degraded => GenerationPhase::Degraded,
        ResourcePhase::Deleted => GenerationPhase::Deleted,
        ResourcePhase::Unknown => GenerationPhase::Pending,
    }
}

/// Preserved ordinal derivation: the trailing bounded generation number of
/// the resource name, else the durable row generation.
fn ordinal_from_name(name: &str) -> Option<u64> {
    name.rsplit('-')
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Everything the composition unit must construct to instantiate the
/// activation driver factory for one zone. The application verifier is the
/// preserved fail-closed gate (old `set_verifier`).
pub(crate) struct ActivationDriverArgs {
    pub(crate) zone: String,
    pub(crate) effects: Arc<dyn ActivationDriverEffects>,
    pub(crate) verifier: Arc<dyn ActivationApplicationVerifier>,
}

/// [`ResourceDriverFactory`] for the `NixosGeneration` resource type.
/// Construction is infallible by contract (R3).
pub(crate) struct ActivationDriverFactory {
    types: [ResourceTypeName; 1],
    args: ActivationDriverArgs,
}

impl ActivationDriverFactory {
    pub(crate) fn new(args: ActivationDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(ACTIVATION_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for ActivationDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(ActivationDriver::new(ActivationDriverArgs {
            zone: self.args.zone.clone(),
            effects: Arc::clone(&self.args.effects),
            verifier: Arc::clone(&self.args.verifier),
        }))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One `NixosGeneration` resource's driver.
pub(crate) struct ActivationDriver {
    zone: String,
    effects: Arc<dyn ActivationDriverEffects>,
    verifier: Arc<dyn ActivationApplicationVerifier>,
    controller: ActivationController,
    /// The owned runner child a settle watch is already registered for
    /// (R12 dependency edge: one registration per child).
    watched_runner: std::sync::Mutex<Option<ResourceKey>>,
}

impl ActivationDriver {
    pub(crate) fn new(args: ActivationDriverArgs) -> Self {
        Self {
            zone: args.zone,
            effects: args.effects,
            verifier: args.verifier,
            controller: ActivationController::new(RETAINED_GENERATIONS),
            watched_runner: std::sync::Mutex::new(None),
        }
    }

    fn error(&self, kind: ActivationDriverErrorKind, op: DriverOp) -> ActivationDriverError {
        ActivationDriverError::new(kind, op)
    }

    /// The closed generation contract of this row (the spec's own
    /// constructor is the Provider/execution/artifact fence).
    fn decoded_spec(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<NixosGenerationSpec, ActivationDriverError> {
        ctx.spec::<NixosGenerationSpec>()
            .cloned()
            .map_err(|_| self.error(ActivationDriverErrorKind::SpecInvalid, op))
    }

    /// This generation's reference, derived from the durable key.
    fn generation_ref(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<ResourceRef, ActivationDriverError> {
        ResourceRef::parse(&format!(
            "{}/{}",
            ctx.key().type_name.as_str(),
            ctx.key().name.as_str()
        ))
        .map_err(|_| self.error(ActivationDriverErrorKind::SpecInvalid, op))
    }

    /// The deterministic owned-runner key for this generation (old
    /// `activation_runner_ref`).
    fn runner_key(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<ResourceKey, ActivationDriverError> {
        let runner_ref = activation_runner_ref(&self.generation_ref(ctx, op)?);
        Ok(ResourceKey::new(
            &self.zone,
            RUNNER_TYPE_NAME,
            runner_ref.name().as_str(),
        ))
    }

    /// The owned runner child, when it exists (old `find_runner_resource`:
    /// identity is the deterministic name; ownership is the manager's).
    async fn runner_child(
        &self,
        ctx: &mut ResourceContext,
        op: DriverOp,
    ) -> Result<Option<ResourceKey>, ActivationDriverError> {
        let expected = self.runner_key(ctx, op)?;
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(ActivationDriverErrorKind::ChildMutation, op))?;
        Ok(owned
            .into_iter()
            .find(|row| row.key == expected)
            .map(|row| row.key))
    }

    /// The observed phase of this generation: the last in-memory projection,
    /// else `Pending` (old `status_phase(...).unwrap_or(Pending)`).
    fn observed_phase(&self, ctx: &ResourceContext) -> ResourcePhase {
        ctx.status::<ActivationDriverStatus>()
            .map_or(ResourcePhase::Pending, |status| status.phase())
    }

    /// The prior-generation observations the pure policy consumes. The new
    /// plane keeps status actor-local (R11), so a sibling's phase is not
    /// observable here; the policy reads the prior entry's name and ordinal
    /// only (membership check and handoff source generation). A missing or
    /// cross-execution prior row fails closed exactly as the old
    /// same-execution sibling set did.
    async fn prior_observations(
        &self,
        ctx: &mut ResourceContext,
        spec: &NixosGenerationSpec,
        op: DriverOp,
    ) -> Result<Vec<GenerationObservation>, ActivationDriverError> {
        let Some(prior) = spec.prior_generation_ref() else {
            return Ok(Vec::new());
        };
        let key = ResourceKey::new(
            &self.zone,
            ACTIVATION_TYPE_NAME,
            prior.name().as_str(),
        );
        let row = ctx
            .get(&key)
            .await
            .map_err(|_| self.error(ActivationDriverErrorKind::ChildMutation, op))?
            .ok_or_else(|| self.error(ActivationDriverErrorKind::Policy, op))?;
        let prior_spec = serde_json::from_slice::<NixosGenerationSpec>(&row.spec)
            .map_err(|_| self.error(ActivationDriverErrorKind::Policy, op))?;
        if prior_spec.execution_ref() != spec.execution_ref() {
            return Err(self.error(ActivationDriverErrorKind::Policy, op));
        }
        let ordinal = ordinal_from_name(&row.key.name).unwrap_or(row.generation);
        Ok(vec![GenerationObservation::terminal(
            row.key.name.as_str(),
            GenerationPhase::Pending,
            ordinal,
        )])
    }

    /// The preserved application-verification gate (old
    /// `application_is_verified`).
    fn application_is_verified(&self, request: &RunnerRequest) -> bool {
        self.verifier
            .verify_application(&self.controller, request)
            .is_ok()
    }

    /// The worker-style gate: the runner must be exactly the request the
    /// policy planned from this spec.
    fn runner_matches_spec(
        &self,
        spec: &NixosGenerationSpec,
        request: &RunnerRequest,
        observed: &GenerationObservation,
    ) -> bool {
        request.system_artifact_id == *spec.system_artifact_id()
            && request.execution_ref == *spec.execution_ref()
            && request.activation_mode == spec.activation_mode()
            && request.target_generation == observed.ordinal()
    }

    /// Execute one planned Host-target activation (old `execute_runner` +
    /// `execute_host_handoff`, same order: intent match, verification gate,
    /// source derivation, compatibility floor, dispatch).
    async fn execute_host_runner(
        &self,
        spec: &NixosGenerationSpec,
        request: &RunnerRequest,
        prior: &[GenerationObservation],
        observed: &GenerationObservation,
    ) -> ActivationOutcomeCode {
        if !self.runner_matches_spec(spec, request, observed) {
            return ActivationOutcomeCode::TargetMismatch;
        }
        if !self.application_is_verified(request) {
            return ActivationOutcomeCode::HelperRefused;
        }
        if request.execution_ref.resource_type().as_str() != "Host" {
            return ActivationOutcomeCode::TargetMismatch;
        }
        let source_generation = spec
            .prior_generation_ref()
            .and_then(|reference| {
                prior
                    .iter()
                    .find(|observation| observation.name() == reference.name().as_str())
                    .map(GenerationObservation::ordinal)
            })
            .unwrap_or_else(|| observed.ordinal().saturating_sub(1));
        if source_generation == 0 || observed.ordinal() <= source_generation {
            return ActivationOutcomeCode::StaleGeneration;
        }
        let Ok(compatibility) = SourceGenerationCompatibilityFloorV1::new(
            source_generation,
            target_fingerprint(
                spec.execution_ref(),
                spec.system_artifact_id(),
                observed.ordinal(),
            ),
        ) else {
            // Preserved: the old floor construction error was folded into
            // the helper-failed outcome by `execute_runner`.
            return ActivationOutcomeCode::HelperFailed;
        };
        let intent = HostGenerationHandoffIntent {
            source_generation,
            target_generation: observed.ordinal(),
            system_artifact_id: spec.system_artifact_id().clone(),
            activation_mode: spec.activation_mode(),
            compatibility,
        };
        let result = self
            .effects
            .apply_host_generation_handoff(spec.execution_ref().clone(), intent)
            .await;
        host_handoff_outcome(result)
    }

    /// Mint the activation-runner `EphemeralProcess` as an owned child
    /// (KTD13): the manager commits the child row before its actor exists
    /// (F1). The launch parameters travel on the spec's typed
    /// `activationInput`; the spec stays argv-free.
    async fn ensure_runner(
        &self,
        ctx: &mut ResourceContext,
        request: &RunnerRequest,
        op: DriverOp,
    ) -> Result<(), ActivationDriverError> {
        let mut runner_spec = serde_json::to_value(activation_runner_spec(request))
            .map_err(|_| self.error(ActivationDriverErrorKind::SpecInvalid, op))?;
        let runner_object = runner_spec
            .as_object_mut()
            .ok_or_else(|| self.error(ActivationDriverErrorKind::SpecInvalid, op))?;
        runner_object.insert(
            "providerRef".to_owned(),
            serde_json::json!(RUNNER_PROVIDER_REF),
        );
        let runner_key = self.runner_key(ctx, op)?;
        let child = ChildEnsure {
            type_name: ResourceTypeName::new(RUNNER_TYPE_NAME),
            name: runner_key.name.clone(),
            spec: serde_json::to_vec(&runner_spec)
                .map_err(|_| self.error(ActivationDriverErrorKind::SpecInvalid, op))?,
            metadata: Vec::new(),
        };
        ctx.ensure_child(child)
            .await
            .map_err(|_| self.error(ActivationDriverErrorKind::ChildMutation, op))?;
        Ok(())
    }

    /// Register the settle dependency edge on the owned runner once per
    /// child (R12): the old durable-status watch that re-triggered the
    /// parent is an internal watch on the new plane.
    async fn watch_runner_settle(&self, ctx: &mut ResourceContext, runner: &ResourceKey) {
        let already_watched = self
            .watched_runner
            .lock()
            .map(|watched| watched.as_ref() == Some(runner))
            .unwrap_or(true);
        if already_watched {
            return;
        }
        if ctx
            .watch(runner.clone(), WatchCondition::Ready)
            .await
            .is_ok()
            && let Ok(mut watched) = self.watched_runner.lock()
        {
            *watched = Some(runner.clone());
        }
    }

    /// Project one applied policy outcome into the in-memory status slot
    /// (old `publish_status`, R11).
    fn project(
        &self,
        ctx: &mut ResourceContext,
        spec: &NixosGenerationSpec,
        outcome: ActivationOutcomeCode,
        result: &d2b_provider_activation_nixos::RunnerResult,
    ) {
        let detail = activation_detail(spec.activation_mode(), outcome, result.phase());
        ctx.set_status(ActivationDriverStatus::projected(
            result.phase(),
            detail,
            result.audit_codes().first().copied(),
        ));
    }
}

#[async_trait::async_trait]
impl ResourceDriver for ActivationDriver {
    type Error = ActivationDriverError;

    /// The old path classified every failure retryable
    /// (`HandlerFailure::retryable()`); the conversion preserves that
    /// classification through the closed boundary.
    fn classify_error(&self, error: &ActivationDriverError) -> DriverFailure {
        DriverFailure::retryable(error.op)
    }

    /// Structural validation of the durable spec: the closed generation
    /// contract, whose own constructor enforces the Provider reference, the
    /// Host/Guest execution target, the artifact identifier, and the
    /// prior-generation reference type.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        self.decoded_spec(ctx, DriverOp::Validate)?;
        Ok(())
    }

    /// Discovery and adoption on the resource's target (F2). A Host target
    /// realizes through the broker authority - nothing local to adopt, so
    /// reconcile re-dispatches the replay-safe handoff. A Guest target
    /// rejoins its owned runner child (old: restart rejoins the existing
    /// runner and does not duplicate the operation).
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let spec = self.decoded_spec(ctx, DriverOp::Recover)?;
        if spec.execution_ref().resource_type().as_str() == "Host" {
            return Ok(RecoveryOutcome::Missing);
        }
        if let Some(runner) = self.runner_child(ctx, DriverOp::Recover).await? {
            self.watch_runner_settle(ctx, &runner).await;
            ctx.set_status(ActivationDriverStatus::staged());
            Ok(RecoveryOutcome::Adopted)
        } else {
            Ok(RecoveryOutcome::Missing)
        }
    }

    /// One reconcile pass, preserving the old per-generation ordering:
    /// terminal success converges, an in-flight runner is rejoined, the pure
    /// policy plans, and exactly one effect runs per pass (Host handoff,
    /// adopt projection, verification refusal, or runner minting).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let spec = self.decoded_spec(ctx, DriverOp::Reconcile)?;
        // Old `ordinal_from_resource`: the trailing bounded generation
        // number, else the durable row generation.
        let ordinal = ordinal_from_name(ctx.key().name.as_str()).unwrap_or(ctx.generation());
        let observed = GenerationObservation::terminal(
            ctx.key().name.as_str(),
            generation_phase(self.observed_phase(ctx)),
            ordinal,
        );

        // Preserved: a generation already at a successful terminal phase
        // converges without effects.
        if matches!(
            observed.phase(),
            GenerationPhase::Ready | GenerationPhase::Succeeded
        ) {
            return Ok(ReconcileOutcome::Satisfied);
        }

        // Preserved rejoin: the owned runner is the realization in flight
        // (old child phase Pending -> staged, Ready -> applying). The
        // child's terminal outcome is actor-local (R11), so the driver
        // waits on the settle edge instead of fabricating a classification:
        // the first pass that observes the child projects `staged`, and a
        // later pass (the settle edge, or any other trigger) projects
        // `applying` while the outcome surface is still unavailable.
        if let Some(runner) = self.runner_child(ctx, DriverOp::Reconcile).await? {
            let settle_watched = self
                .watched_runner
                .lock()
                .map(|watched| watched.as_ref() == Some(&runner))
                .unwrap_or(true);
            ctx.set_status(if settle_watched {
                ActivationDriverStatus::applying()
            } else {
                ActivationDriverStatus::staged()
            });
            self.watch_runner_settle(ctx, &runner).await;
            return Ok(ReconcileOutcome::Satisfied);
        }

        let prior = self
            .prior_observations(ctx, &spec, DriverOp::Reconcile)
            .await?;
        let caller = ActivationCaller::new(CallerRole::Lifecycle, spec.execution_ref().clone());
        let planned = self
            .controller
            .reconcile(&spec, &caller, &prior, observed.clone())
            .map_err(|_| self.error(ActivationDriverErrorKind::Policy, DriverOp::Reconcile))?;

        let Some(request) = planned.runner_requests().first().cloned() else {
            if spec.activation_mode() == ActivationMode::Adopt {
                let applied = self
                    .controller
                    .apply_runner_result(&spec, ActivationOutcomeCode::Adopted, observed)
                    .map_err(|_| {
                        self.error(ActivationDriverErrorKind::Policy, DriverOp::Reconcile)
                    })?;
                self.project(ctx, &spec, ActivationOutcomeCode::Adopted, &applied);
            }
            return Ok(ReconcileOutcome::Satisfied);
        };

        if request.execution_ref.resource_type().as_str() == "Host" {
            let outcome = self
                .execute_host_runner(&spec, &request, &prior, &observed)
                .await;
            let applied = self
                .controller
                .apply_runner_result(&spec, outcome, observed)
                .map_err(|_| self.error(ActivationDriverErrorKind::Policy, DriverOp::Reconcile))?;
            self.project(ctx, &spec, outcome, &applied);
            return Ok(ReconcileOutcome::Satisfied);
        }

        // Guest target: the preserved verification gate runs before the
        // runner resource is created; a refusal never mints the child.
        if !self.application_is_verified(&request) {
            let outcome = ActivationOutcomeCode::HelperRefused;
            let applied = self
                .controller
                .apply_runner_result(&spec, outcome, observed)
                .map_err(|_| self.error(ActivationDriverErrorKind::Policy, DriverOp::Reconcile))?;
            self.project(ctx, &spec, outcome, &applied);
            return Ok(ReconcileOutcome::Satisfied);
        }
        self.ensure_runner(ctx, &request, DriverOp::Reconcile)
            .await?;
        // Arm the settle edge on the freshly minted child so the actor is
        // woken when it settles (R12); the projection stays non-terminal.
        let runner = self.runner_key(ctx, DriverOp::Reconcile)?;
        self.watch_runner_settle(ctx, &runner).await;
        ctx.set_status(ActivationDriverStatus::staged());
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown. The call nudges each owned child through its
    /// own finalize-before-delete pass - the activation runner retires before
    /// its generation may - and requeues this pass while any child row is
    /// still live. Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| self.error(ActivationDriverErrorKind::ChildMutation, DriverOp::Delete))?;
        Ok(())
    }

    /// Teardown (R10, F3): the durable deleting mark is already committed.
    /// The owned runner retires through the manager - the child has no
    /// finalizers of its own, so its row disappears once cleanup completes,
    /// and the parent row follows it. Idempotent under retry.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let runner = self.runner_key(ctx, DriverOp::Delete)?;
        ctx.delete(&runner)
            .await
            .map_err(|_| self.error(ActivationDriverErrorKind::ChildMutation, DriverOp::Delete))
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a scripted effect port and a recording
// manager endpoint with one shared ordered log (R4; F1/AE1 observed as the
// manager records it).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use d2b_contracts_broker::host_generation::{
        HostGenerationHandoffIntent, SourceGenerationCompatibilityFloorV1, target_fingerprint,
    };
    use d2b_contracts_resource::v3::{
        ActivationDetail, ActivationMode, ActivationOutcomeCode, ArtifactId, NixosGenerationSpec,
        ResourcePhase, ResourceRef,
    };
    use d2b_provider_activation_nixos::{
        ActivationApplicationVerifier, ActivationController, ActivationVerificationError,
        RunnerRequest, activation_runner_ref,
    };
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, ResourceContext, WatchCondition, WatchId, WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{
        ResourceKey, ResourceProvenance, StoredDesiredResource,
    };
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;

    use super::{
        ACTIVATION_TYPE_NAME, ActivationDriverArgs, ActivationDriverFactory,
        ActivationDriverStatus, HostHandoffResult, activation_spec_decoder, ordinal_from_name,
    };

    // -- fakes ---------------------------------------------------------------

    /// Scripted host-handoff port: records each dispatch and returns the
    /// next scripted result.
    struct FakeActivationEffects {
        dispatches: parking_lot::Mutex<Vec<(ResourceRef, HostGenerationHandoffIntent)>>,
        results: parking_lot::Mutex<Vec<HostHandoffResult>>,
    }

    impl FakeActivationEffects {
        fn new(result: HostHandoffResult) -> Arc<Self> {
            Arc::new(Self {
                dispatches: parking_lot::Mutex::new(Vec::new()),
                results: parking_lot::Mutex::new(vec![result]),
            })
        }

        fn dispatches(&self) -> Vec<(ResourceRef, HostGenerationHandoffIntent)> {
            self.dispatches.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl super::ActivationDriverEffects for FakeActivationEffects {
        async fn apply_host_generation_handoff(
            &self,
            target: ResourceRef,
            intent: HostGenerationHandoffIntent,
        ) -> HostHandoffResult {
            self.dispatches.lock().push((target, intent));
            self.results
                .lock()
                .pop()
                .unwrap_or(HostHandoffResult::Incomplete)
        }
    }

    /// Allowlist verifier: the production artifact/application adapter stand-in.
    struct AllowVerifier;

    impl ActivationApplicationVerifier for AllowVerifier {
        fn verify_application(
            &self,
            _controller: &ActivationController,
            _request: &RunnerRequest,
        ) -> Result<(), ActivationVerificationError> {
            Ok(())
        }
    }

    /// Recording manager endpoint over one shared ordered log; owned rows
    /// carry the generation's uid as their owner.
    #[derive(Clone)]
    struct RecordingManager {
        zone: String,
        owner_uid: [u8; 16],
        log: Arc<parking_lot::Mutex<Vec<String>>>,
        rows: Arc<parking_lot::Mutex<Vec<StoredDesiredResource>>>,
        next_uid: Arc<std::sync::atomic::AtomicU64>,
    }

    impl RecordingManager {
        fn new(owner_uid: [u8; 16]) -> Self {
            Self {
                zone: "work".to_owned(),
                owner_uid,
                log: Arc::new(parking_lot::Mutex::new(Vec::new())),
                rows: Arc::new(parking_lot::Mutex::new(Vec::new())),
                next_uid: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            }
        }

        fn with_row(self, row: StoredDesiredResource) -> Self {
            self.rows.lock().push(row);
            self
        }

        fn log(&self) -> Vec<String> {
            self.log.lock().clone()
        }

        fn row(&self, key: &ResourceKey) -> Option<StoredDesiredResource> {
            self.rows.lock().iter().find(|row| row.key == *key).cloned()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            let id = format!("{}/{}", child.type_name.as_str(), child.name);
            self.log.lock().push(format!("ensure:{id}"));
            let next = self
                .next_uid
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut uid = [0u8; 16];
            uid[..8].copy_from_slice(&next.to_be_bytes());
            let row = StoredDesiredResource {
                key: ResourceKey::new(&self.zone, child.type_name.as_str(), &child.name),
                uid,
                generation: 1,
                owner_uid: Some(self.owner_uid),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: child.spec,
                metadata: child.metadata,
                created_at: 0,
            };
            let mut rows = self.rows.lock();
            let outcome = match rows.iter_mut().find(|existing| existing.key == row.key) {
                Some(existing) if existing.spec == row.spec => EnsureOutcome::Unchanged(existing.clone()),
                Some(existing) => {
                    *existing = row.clone();
                    EnsureOutcome::Updated(row.clone())
                }
                None => {
                    rows.push(row.clone());
                    EnsureOutcome::Created(row.clone())
                }
            };
            // The spawn notification the manager emits after the commit (F1).
            self.log.lock().push(format!("spawned:{id}"));
            Ok(outcome)
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self.row(key))
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
            self.log.lock().push(format!(
                "delete:{}/{}",
                key.type_name, key.name
            ));
            self.rows.lock().retain(|row| row.key != *key);
            Ok(())
        }

        async fn list_owned(
            &self,
            owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self
                .rows
                .lock()
                .iter()
                .filter(|row| row.owner_uid.as_ref() == Some(&owner_uid))
                .cloned()
                .collect())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            assert_eq!(registration.condition, WatchCondition::Ready);
            self.log.lock().push(format!(
                "watch:{}/{}",
                registration.target.type_name, registration.target.name
            ));
            Ok(WatchId(1))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    /// Dead requeue: the preserved activation flows never schedule one.
    struct NullRequeue;

    impl d2b_resource_runtime::context::RequeueScheduler for NullRequeue {
        fn schedule(
            &self,
            _key: ResourceKey,
            _after: std::time::Duration,
        ) -> d2b_resource_runtime::context::RequeueId {
            d2b_resource_runtime::context::RequeueId(0)
        }

        fn cancel(&self, _id: d2b_resource_runtime::context::RequeueId) {}
    }

    // -- fixtures ------------------------------------------------------------

    const GENERATION_UID: [u8; 16] = [0x42; 16];

    fn generation_spec(
        execution_ref: &str,
        mode: ActivationMode,
        prior: Option<&str>,
    ) -> Vec<u8> {
        let spec = NixosGenerationSpec::new(
            ResourceRef::parse("Provider/activation-nixos").expect("activation provider"),
            ResourceRef::parse(execution_ref).expect("execution ref"),
            "system-artifact",
            mode,
            prior.map(|name| {
                ResourceRef::parse(&format!("{ACTIVATION_TYPE_NAME}/{name}"))
                    .expect("prior generation ref")
            }),
        )
        .expect("activation spec");
        serde_json::to_vec(&spec).expect("activation spec JSON")
    }

    fn generation_row(
        name: &str,
        execution_ref: &str,
        mode: ActivationMode,
        prior: Option<&str>,
        uid: [u8; 16],
    ) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", ACTIVATION_TYPE_NAME, name),
            uid,
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting: false,
            spec: generation_spec(execution_ref, mode, prior),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    struct Fixture {
        ctx: ResourceContext,
        manager: RecordingManager,
    }

    fn fixture(row: StoredDesiredResource, manager: RecordingManager) -> Fixture {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = ResourceContext::new(
            row,
            TargetHandle::Host,
            activation_spec_decoder(),
            Arc::new(manager.clone()),
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        );
        Fixture { ctx, manager }
    }

    async fn driver(
        effects: Arc<FakeActivationEffects>,
        verifier: Arc<dyn ActivationApplicationVerifier>,
    ) -> Box<dyn DynResourceDriver> {
        let factory = ActivationDriverFactory::new(ActivationDriverArgs {
            zone: "work".to_owned(),
            effects,
            verifier,
        });
        factory
            .create(&ResourceKey::new("work", ACTIVATION_TYPE_NAME, "gen-1"))
            .await
    }

    fn status(ctx: &ResourceContext) -> ActivationDriverStatus {
        *ctx.status::<ActivationDriverStatus>().expect("projected status")
    }

    // -- factory and validation ----------------------------------------------

    #[tokio::test]
    async fn factory_registers_only_the_generation_resource_type() {
        let factory = ActivationDriverFactory::new(ActivationDriverArgs {
            zone: "work".to_owned(),
            effects: FakeActivationEffects::new(HostHandoffResult::Incomplete),
            verifier: Arc::new(AllowVerifier),
        });
        assert_eq!(factory.resource_types().len(), 1);
        assert_eq!(factory.resource_types()[0].as_str(), ACTIVATION_TYPE_NAME);
    }

    #[tokio::test]
    async fn validate_rejects_a_spec_outside_the_closed_generation_contract() {
        let effects = FakeActivationEffects::new(HostHandoffResult::Incomplete);
        let mut f = fixture(
            generation_row("gen-1", "Host/host-system", ActivationMode::Switch, None, GENERATION_UID),
            RecordingManager::new(GENERATION_UID),
        );
        let mut d = driver(effects, Arc::new(AllowVerifier)).await;
        d.validate(&mut f.ctx).await.expect("valid contract");

        // Malformed spec bytes and a foreign Provider both fail the gate.
        let mut malformed = generation_row(
            "gen-1",
            "Host/host-system",
            ActivationMode::Switch,
            None,
            GENERATION_UID,
        );
        malformed.spec = b"{\"providerRef\":\"Provider/other\"}".to_vec();
        let mut f_malformed = fixture(malformed, RecordingManager::new(GENERATION_UID));
        let mut d_malformed = driver(
            FakeActivationEffects::new(HostHandoffResult::Incomplete),
            Arc::new(AllowVerifier),
        )
        .await;
        assert!(d_malformed.validate(&mut f_malformed.ctx).await.is_err());
    }

    // -- host-target reconcile ------------------------------------------------

    #[tokio::test]
    async fn host_target_dispatches_the_preserved_handoff_intent_and_projects_success() {
        let effects = FakeActivationEffects::new(HostHandoffResult::Completed {
            source_generation: 1,
            target_generation: 2,
        });
        let manager = RecordingManager::new(GENERATION_UID).with_row(generation_row(
            "gen-1",
            "Host/host-system",
            ActivationMode::Switch,
            None,
            [0x41; 16],
        ));
        let mut f = fixture(
            generation_row(
                "gen-2",
                "Host/host-system",
                ActivationMode::Switch,
                Some("gen-1"),
                GENERATION_UID,
            ),
            manager,
        );
        let mut d = driver(effects.clone(), Arc::new(AllowVerifier)).await;

        assert_eq!(
            d.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );

        let dispatches = effects.dispatches();
        assert_eq!(dispatches.len(), 1);
        let (target, intent) = &dispatches[0];
        assert_eq!(target.to_canonical_string(), "Host/host-system");
        assert_eq!(intent.source_generation, 1);
        assert_eq!(intent.target_generation, 2);
        assert_eq!(intent.activation_mode, ActivationMode::Switch);
        assert_eq!(intent.system_artifact_id.as_str(), "system-artifact");
        let expected_floor = SourceGenerationCompatibilityFloorV1::new(
            1,
            target_fingerprint(
                &ResourceRef::parse("Host/host-system").expect("execution ref"),
                &ArtifactId::parse("system-artifact").expect("artifact"),
                2,
            ),
        )
        .expect("compatibility floor");
        assert_eq!(intent.compatibility, expected_floor);

        let projected = status(&f.ctx);
        assert_eq!(projected.phase(), ResourcePhase::Ready);
        assert_eq!(projected.detail(), ActivationDetail::Applied);
        assert_eq!(projected.outcome(), Some(ActivationOutcomeCode::Succeeded));
        // A Host target realizes through the broker: no runner child.
        assert!(f.manager.log().is_empty());
    }

    #[tokio::test]
    async fn host_refusal_projects_helper_refused_without_minting_a_runner() {
        let effects = FakeActivationEffects::new(HostHandoffResult::Refused);
        let mut f = fixture(
            generation_row(
                "gen-2",
                "Host/host-system",
                ActivationMode::Boot,
                None,
                GENERATION_UID,
            ),
            RecordingManager::new(GENERATION_UID),
        );
        let mut d = driver(effects.clone(), Arc::new(AllowVerifier)).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");

        assert_eq!(effects.dispatches().len(), 1);
        let projected = status(&f.ctx);
        assert_eq!(projected.phase(), ResourcePhase::Failed);
        assert_eq!(projected.detail(), ActivationDetail::Planning);
        assert_eq!(
            projected.outcome(),
            Some(ActivationOutcomeCode::HelperRefused)
        );
        assert!(f.manager.log().is_empty());
    }

    #[tokio::test]
    async fn host_target_without_a_prior_row_fails_closed_before_dispatch() {
        let effects = FakeActivationEffects::new(HostHandoffResult::Completed {
            source_generation: 1,
            target_generation: 2,
        });
        // The prior generation is absent from the store.
        let mut f = fixture(
            generation_row(
                "gen-2",
                "Host/host-system",
                ActivationMode::Switch,
                Some("gen-1"),
                GENERATION_UID,
            ),
            RecordingManager::new(GENERATION_UID),
        );
        let mut d = driver(effects.clone(), Arc::new(AllowVerifier)).await;
        assert!(d.reconcile(&mut f.ctx).await.is_err());
        assert!(effects.dispatches().is_empty());

        // A prior row for another execution context is not a sibling.
        let mut cross = fixture(
            generation_row(
                "gen-2",
                "Host/host-system",
                ActivationMode::Switch,
                Some("gen-1"),
                GENERATION_UID,
            ),
            RecordingManager::new(GENERATION_UID).with_row(generation_row(
                "gen-1",
                "Host/other-host",
                ActivationMode::Switch,
                None,
                [0x41; 16],
            )),
        );
        let mut cross_driver = driver(effects.clone(), Arc::new(AllowVerifier)).await;
        assert!(cross_driver.reconcile(&mut cross.ctx).await.is_err());
        assert!(effects.dispatches().is_empty());
    }

    #[tokio::test]
    async fn offline_verification_refuses_before_the_handoff_is_dispatched() {
        let effects = FakeActivationEffects::new(HostHandoffResult::Completed {
            source_generation: 1,
            target_generation: 2,
        });
        let mut f = fixture(
            generation_row(
                "gen-2",
                "Host/host-system",
                ActivationMode::Switch,
                Some("gen-1"),
                GENERATION_UID,
            ),
            RecordingManager::new(GENERATION_UID).with_row(generation_row(
                "gen-1",
                "Host/host-system",
                ActivationMode::Switch,
                None,
                [0x41; 16],
            )),
        );
        let mut d = driver(
            effects.clone(),
            Arc::new(d2b_provider_activation_nixos::FailClosedActivationVerifier),
        )
        .await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");

        assert!(
            effects.dispatches().is_empty(),
            "the fail-closed verifier must refuse before any effect"
        );
        let projected = status(&f.ctx);
        assert_eq!(
            projected.outcome(),
            Some(ActivationOutcomeCode::HelperRefused)
        );
    }

    // -- guest-target reconcile -----------------------------------------------

    #[tokio::test]
    async fn guest_target_mints_the_runner_as_an_owned_process_resource() {
        let effects = FakeActivationEffects::new(HostHandoffResult::Incomplete);
        let manager = RecordingManager::new(GENERATION_UID);
        let mut f = fixture(
            generation_row(
                "gen-1",
                "Guest/guest-a",
                ActivationMode::Switch,
                None,
                GENERATION_UID,
            ),
            manager.clone(),
        );
        let mut d = driver(effects.clone(), Arc::new(AllowVerifier)).await;
        assert_eq!(
            d.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );

        // KTD13: the launch is a Process-resource mint through the manager,
        // never a spawn from this controller, and the child row is committed
        // before its spawn notification (F1).
        let log = manager.log();
        let ensure = log
            .iter()
            .position(|entry| entry.starts_with("ensure:EphemeralProcess/"))
            .expect("runner ensure recorded");
        let spawned = log
            .iter()
            .position(|entry| entry.starts_with("spawned:EphemeralProcess/"))
            .expect("runner spawn recorded");
        assert!(ensure < spawned, "persist-before-spawn (F1): {log:?}");
        assert!(
            effects.dispatches().is_empty(),
            "a guest target never takes the host handoff path"
        );

        let runner_key = ResourceKey::new(
            "work",
            "EphemeralProcess",
            super::activation_runner_ref(
                &ResourceRef::parse(&format!("{ACTIVATION_TYPE_NAME}/gen-1"))
                    .expect("generation ref"),
            )
            .name()
            .as_str(),
        );
        let runner = manager.row(&runner_key).expect("runner row committed");
        assert_eq!(runner.owner_uid, Some(GENERATION_UID));

        // The launch parameters travel on the sanctioned typed channel: the
        // runner spec is argv-free and carries the typed activation input.
        let spec: serde_json::Value =
            serde_json::from_slice(&runner.spec).expect("runner spec JSON");
        assert_eq!(spec["providerRef"], "Provider/system-minijail");
        assert_eq!(spec["template"], "activation-nixos-runner");
        assert_eq!(spec["activationInput"]["targetGeneration"], 1);
        assert_eq!(spec["activationInput"]["activationMode"], "switch");
        assert_eq!(spec["activationInput"]["systemArtifactId"], "system-artifact");
        assert_no_launch_argv(&spec);

        assert_eq!(status(&f.ctx).detail(), ActivationDetail::Staged);
        assert_eq!(log.iter().filter(|entry| entry.starts_with("ensure:")).count(), 1);
    }

    #[tokio::test]
    async fn guest_rejoin_waits_on_the_settle_edge_without_duplicating_the_runner() {
        let effects = FakeActivationEffects::new(HostHandoffResult::Incomplete);
        let manager = RecordingManager::new(GENERATION_UID);
        let mut f = fixture(
            generation_row(
                "gen-1",
                "Guest/guest-a",
                ActivationMode::Switch,
                None,
                GENERATION_UID,
            ),
            manager.clone(),
        );
        let mut d = driver(effects, Arc::new(AllowVerifier)).await;
        d.reconcile(&mut f.ctx).await.expect("first reconcile");
        d.reconcile(&mut f.ctx).await.expect("rejoin reconcile");

        let log = manager.log();
        assert_eq!(
            log.iter().filter(|entry| entry.starts_with("ensure:")).count(),
            1,
            "an existing runner is rejoined, not re-minted: {log:?}"
        );
        assert_eq!(
            log.iter().filter(|entry| entry.starts_with("watch:")).count(),
            1,
            "one settle watch per child: {log:?}"
        );
        assert_eq!(status(&f.ctx).detail(), ActivationDetail::Applying);
    }

    #[tokio::test]
    async fn recover_adopts_an_existing_runner_and_reports_missing_without_one() {
        let effects = FakeActivationEffects::new(HostHandoffResult::Incomplete);
        let manager = RecordingManager::new(GENERATION_UID);
        let row = generation_row(
            "gen-1",
            "Guest/guest-a",
            ActivationMode::Switch,
            None,
            GENERATION_UID,
        );
        let mut f = fixture(row.clone(), manager.clone());
        let mut d = driver(effects.clone(), Arc::new(AllowVerifier)).await;
        assert_eq!(
            d.recover(&mut f.ctx).await.expect("recover"),
            RecoveryOutcome::Missing
        );
        assert!(
            f.ctx.status::<ActivationDriverStatus>().is_none(),
            "no runner to rejoin: recovery projects nothing"
        );
        assert!(manager.log().is_empty());

        // The Host target realizes through the broker authority.
        let mut host = fixture(
            generation_row(
                "gen-1",
                "Host/host-system",
                ActivationMode::Switch,
                None,
                GENERATION_UID,
            ),
            RecordingManager::new(GENERATION_UID),
        );
        let mut host_driver = driver(effects, Arc::new(AllowVerifier)).await;
        assert_eq!(
            host_driver.recover(&mut host.ctx).await.expect("recover"),
            RecoveryOutcome::Missing
        );
    }

    // -- finalize: the owned runner retires before the generation (F3) --------

    #[tokio::test]
    async fn finalize_finalizes_the_owned_runner_before_the_generation_teardown() {
        let manager = RecordingManager::new(GENERATION_UID).with_row(StoredDesiredResource {
            owner_uid: Some(GENERATION_UID),
            ..generation_row(
                "runner-gen-1",
                "Guest/workstation",
                ActivationMode::Switch,
                None,
                [0x77; 16],
            )
        });
        let Fixture { mut ctx, manager } = fixture(
            generation_row(
                "gen-1",
                "Guest/workstation",
                ActivationMode::Switch,
                None,
                GENERATION_UID,
            ),
            manager,
        );
        let mut d = driver(
            FakeActivationEffects::new(HostHandoffResult::Incomplete),
            Arc::new(AllowVerifier),
        )
        .await;

        // A live owned runner: the pass requeues instead of tearing down.
        let failure = d.finalize(&mut ctx).await.expect_err("owned runner still live");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert!(
            manager.log().iter().any(|call| call.starts_with("delete:")),
            "the owned runner is nudged through its own finalize-before-delete pass"
        );

        // The manager removed the retired runner row: the same pass converges.
        d.finalize(&mut ctx).await.expect("converged once the runner retired");
    }

    // -- teardown -------------------------------------------------------------

    #[tokio::test]
    async fn delete_retires_only_the_owned_runner_child_and_is_idempotent() {
        let effects = FakeActivationEffects::new(HostHandoffResult::Incomplete);
        let manager = RecordingManager::new(GENERATION_UID);
        let mut f = fixture(
            generation_row(
                "gen-2",
                "Guest/guest-a",
                ActivationMode::Switch,
                None,
                GENERATION_UID,
            ),
            manager.clone(),
        );
        let mut d = driver(effects, Arc::new(AllowVerifier)).await;
        d.reconcile(&mut f.ctx).await.expect("mint runner");
        d.delete(&mut f.ctx).await.expect("delete");
        d.delete(&mut f.ctx).await.expect("retry delete");

        let runner_name = activation_runner_ref(
            &ResourceRef::parse(&format!("{ACTIVATION_TYPE_NAME}/gen-2"))
                .expect("generation ref"),
        )
        .name()
        .as_str()
        .to_owned();
        let deletions = manager
            .log()
            .into_iter()
            .filter(|entry| entry.starts_with("delete:"))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            deletions,
            BTreeSet::from([format!("delete:EphemeralProcess/{runner_name}")]),
            "only the owned runner child is retired, once per pass"
        );
        let runner_key = ResourceKey::new("work", "EphemeralProcess", &runner_name);
        assert!(
            manager.row(&runner_key).is_none(),
            "the runner child retires through the manager"
        );
    }

    // -- preserved projections -------------------------------------------------

    #[test]
    fn generation_ordinals_are_taken_from_bounded_names() {
        assert_eq!(ordinal_from_name("gen-7"), Some(7));
        assert_eq!(ordinal_from_name("dev-vm--gen-7"), Some(7));
        assert_eq!(ordinal_from_name("gen-0"), None);
        assert_eq!(ordinal_from_name("gen"), None);
    }

    #[test]
    fn boot_success_projects_the_default_and_switch_success_projects_applied() {
        assert_eq!(
            super::activation_detail(
                ActivationMode::Boot,
                ActivationOutcomeCode::Succeeded,
                ResourcePhase::Succeeded,
            ),
            ActivationDetail::BootDefault
        );
        assert_eq!(
            super::activation_detail(
                ActivationMode::Switch,
                ActivationOutcomeCode::Succeeded,
                ResourcePhase::Succeeded,
            ),
            ActivationDetail::Applied
        );
        assert_eq!(
            super::activation_detail(
                ActivationMode::Switch,
                ActivationOutcomeCode::RolledBack,
                ResourcePhase::Failed,
            ),
            ActivationDetail::RolledBack
        );
        assert_eq!(
            super::activation_detail(
                ActivationMode::Switch,
                ActivationOutcomeCode::HelperFailed,
                ResourcePhase::Failed,
            ),
            ActivationDetail::Planning
        );
    }

    /// The runner spec must stay argv-free: the launch parameters cross on
    /// the typed activation-input channel (KTD13 contract fence).
    fn assert_no_launch_argv(value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    assert!(
                        !matches!(key.as_str(), "argv" | "args" | "command" | "cmd" | "exec"),
                        "launch argv must not travel in the runner resource: {key}"
                    );
                    assert_no_launch_argv(value);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(assert_no_launch_argv),
            _ => {}
        }
    }
}
