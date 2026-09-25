//! The NixosGeneration resource driver (U12): the v3 `ResourceDriver`
//! conversion of the activation path (R3, R4, R30; KTD7, KTD13).
//!
//! The driver, its spec decoder, its factory, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by live in this crate. The driver's effects are this
//! crate's own implementation ([`crate::effects_service`]) built from the
//! daemon-supplied facet set, so the construction site holds no externally
//! built port (R2) and this crate depends on no daemon runtime.
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
//! 1. Generation retention needs the sibling generations of one
//!    `executionRef`; the new `ResourceContext` exposes only this resource
//!    and its owned children (no zone-wide list), so surplus pruning stays
//!    a desired-state concern (the bundle-ingest `remove` path) rather than
//!    a driver effect.
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

use d2b_contracts_broker::host_generation::{
    HostGenerationHandoffIntent, SourceGenerationCompatibilityFloorV1, target_fingerprint,
};
use d2b_contracts_resource::v3::{
    ActivationDetail, ActivationMode, ActivationOutcomeCode, NIXOS_GENERATION_RESOURCE_TYPE,
    NixosGenerationSpec, ResourceName, ResourcePhase, ResourceRef,
};
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, ChildCreation, ChildCustody, DriverDescriptor,
    WellKnownType,
};

use crate::effects_service::ACTIVATION_EFFECTS_SERVICE;
use crate::{
    ActivationApplicationVerifier, ActivationCaller, ActivationController, CallerRole,
    GenerationObservation, GenerationPhase, RunnerRequest, activation_runner_ref,
    activation_runner_spec,
};

/// The one resource type this factory serves (KTD4 Phase A).
pub const ACTIVATION_TYPE_NAME: &str = NIXOS_GENERATION_RESOURCE_TYPE;

/// The activation-runner child resource type (old `create_runner`).
pub const RUNNER_TYPE_NAME: &str = "EphemeralProcess";

/// The Process Provider the runner is minted under (old `create_runner`).
pub const RUNNER_PROVIDER_REF: &str = "Provider/system-minijail";

/// The one child the NixosGeneration family creates (old `create_runner`).
///
/// The runner is the owned `EphemeralProcess` the driver mints for the
/// members this declaration licenses, and the declaration is the family's own
/// - it travels on the type's [`DriverDescriptor`], so a creation the driver
///   never declared cannot be reached.
pub const ACTIVATION_RUNNER_CREATION: ChildCreation = ChildCreation {
    child: WellKnownType::EPHEMERAL_PROCESS,
    provider_ref: RUNNER_PROVIDER_REF,
    custody: ChildCustody::DriverOwned,
    order: 1,
};

/// Every child creation the NixosGeneration driver declares.
pub const ACTIVATION_CREATIONS: &[ChildCreation] = &[ACTIVATION_RUNNER_CREATION];

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
pub struct ActivationDriverError {
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
pub struct ActivationDriverStatus {
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
    pub const fn phase(self) -> ResourcePhase {
        self.phase
    }

    /// Typed activation detail of the last projection.
    pub const fn detail(self) -> ActivationDetail {
        self.detail
    }

    /// Terminal outcome code of the last projection, when one was reached.
    pub const fn outcome(self) -> Option<ActivationOutcomeCode> {
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
pub fn activation_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| serde_json::from_slice::<NixosGenerationSpec>(bytes))
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The preserved broker response reduced to the fields the outcome mapping
/// reads (old `host_handoff_outcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostHandoffResult {
    /// The coordinator recorded completion of a strict generation
    /// transition.
    Completed {
        /// The generation the coordinator transitioned away from.
        source_generation: u64,
        /// The generation the coordinator transitioned to.
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
pub trait ActivationDriverEffects: Send + Sync + 'static {
    /// Dispatch one `ApplyHostGenerationHandoff` for the authenticated
    /// execution target.
    async fn apply_host_generation_handoff(
        &self,
        target: ResourceRef,
        intent: HostGenerationHandoffIntent,
    ) -> HostHandoffResult;
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
/// activation driver factory for one zone: the zone's own name and the
/// daemon-supplied facet set. The driver's effects are this crate's own
/// implementation ([`crate::effects_service::ActivationEffectsService`])
/// built from those facets, and the application verifier is the preserved
/// fail-closed gate (old `set_verifier`) this crate owns - the construction
/// site holds no externally built port (R2).
pub struct ActivationDriverArgs {
    /// The zone the driver serves.
    pub zone: String,
    /// The daemon-supplied facet set the family's own effects implementation
    /// is built from.
    pub facets: crate::facets::ActivationEffectFacets,
}

/// [`ResourceDriverFactory`] for the `NixosGeneration` resource type.
/// Construction is infallible by contract (R3).
pub struct ActivationDriverFactory {
    types: [ResourceTypeName; 1],
    args: ActivationDriverArgs,
    /// Test-support verifier override: production binds the crate's own
    /// fail-closed verifier; a test scripts the gate so the full
    /// facets -> factory -> driver -> handoff seam is observable through
    /// [`ResourceDriverFactory::create`].
    #[cfg(any(test, feature = "test-support"))]
    verifier: Option<Arc<dyn ActivationApplicationVerifier>>,
}

impl ActivationDriverFactory {
    pub(crate) fn new(args: ActivationDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(ACTIVATION_TYPE_NAME)],
            args,
            #[cfg(any(test, feature = "test-support"))]
            verifier: None,
        }
    }

    /// Test-support constructor over a scripted application verifier: the
    /// production binding is the crate's own fail-closed gate, so a test
    /// that needs the handoff to reach the effects (the seam test) scripts
    /// the gate here instead of weakening the production binding.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_verifier(
        args: ActivationDriverArgs,
        verifier: Arc<dyn ActivationApplicationVerifier>,
    ) -> Self {
        Self {
            types: [ResourceTypeName::new(ACTIVATION_TYPE_NAME)],
            args,
            verifier: Some(verifier),
        }
    }

    /// The application verifier one created driver gates on: the crate's
    /// own fail-closed gate in production; the scripted override when a
    /// test supplies one.
    #[cfg(any(test, feature = "test-support"))]
    fn verifier(&self) -> Arc<dyn ActivationApplicationVerifier> {
        self.verifier
            .clone()
            .unwrap_or_else(|| Arc::new(crate::FailClosedActivationVerifier))
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn verifier(&self) -> Arc<dyn ActivationApplicationVerifier> {
        Arc::new(crate::FailClosedActivationVerifier)
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for ActivationDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(ActivationDriver::new(
            self.args.zone.clone(),
            Arc::new(crate::effects_service::ActivationEffectsService::new(
                self.args.facets.clone(),
            )),
            self.verifier(),
        ))
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
    watched_runner: tokio::sync::Mutex<Option<ResourceKey>>,
}

impl ActivationDriver {
    /// Build one driver over the family's own effects implementation (the
    /// factory constructs it from the declared facets) or a scripted seam
    /// (tests): the driver never constructs a port itself.
    pub(crate) fn new(
        zone: String,
        effects: Arc<dyn ActivationDriverEffects>,
        verifier: Arc<dyn ActivationApplicationVerifier>,
    ) -> Self {
        Self {
            zone,
            effects,
            verifier,
            controller: ActivationController::new(),
            watched_runner: tokio::sync::Mutex::new(None),
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
        let name = ResourceName::parse(&row.key.name)
            .map_err(|_| self.error(ActivationDriverErrorKind::Policy, op))?;
        Ok(vec![GenerationObservation::terminal(
            name,
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
            .await
            .as_ref()
            .is_some_and(|watched| *watched == *runner);
        if already_watched {
            return;
        }
        if ctx
            .watch(runner.clone(), WatchCondition::Ready)
            .await
            .is_ok()
        {
            *self.watched_runner.lock().await = Some(runner.clone());
        }
    }

    /// Project one applied policy outcome into the in-memory status slot
    /// (old `publish_status`, R11).
    fn project(
        &self,
        ctx: &mut ResourceContext,
        spec: &NixosGenerationSpec,
        outcome: ActivationOutcomeCode,
        result: &crate::RunnerResult,
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
        let name = ResourceName::parse(ctx.key().name.as_str())
            .map_err(|_| self.error(ActivationDriverErrorKind::Policy, DriverOp::Reconcile))?;
        let observed = GenerationObservation::terminal(
            name,
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
                .await
                .as_ref()
                .is_some_and(|watched| *watched == runner);
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
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The execution domains the NixosGeneration type can be reconciled in.
///
/// Derived from the placement contract: `NixosGeneration` names the canonical
/// `spec.executionRef` anchor (`PlacementAnchor::canonical_for` resolves
/// `ExecutionRef`), and the spec constructor admits a `Host` or a `Guest`
/// there, so a generation row is driven in either domain.
const ACTIVATION_EXECUTION_DOMAINS: &[&str] = &["host", "guest"];

/// The resource types the driver reads while reconciling.
///
/// Derived from the driver's row reads: the policy consumes the prior
/// generation row of the same `executionRef` (membership and handoff source
/// generation).
const ACTIVATION_READS: &[WellKnownType] = &[WellKnownType::NIXOS_GENERATION];

/// The NixosGeneration type's driver declaration.
///
/// `NixosGeneration` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane
/// cannot serve the activation generations without it, so it must be
/// registered before the plane opens. The type is not exportable:
/// `ResourceExport` admits only qualified `*.d2bus.org.*Service` types, so a
/// generation can never be an export subject. The driver serves no broker
/// operations and declares the one child creation it performs - the owned
/// activation runner ([`ACTIVATION_RUNNER_CREATION`]) - and the family's
/// declared effects service ([`crate::effects_service::ACTIVATION_EFFECTS_SERVICE`])
/// rides the declaration, so a zone that cannot host it refuses startup by
/// name (R5).
pub fn activation_descriptor(args: ActivationDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::NIXOS_GENERATION,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: ACTIVATION_EXECUTION_DOMAINS,
        exportable: false,
        reads: ACTIVATION_READS,
        operations: &[],
        creations: ACTIVATION_CREATIONS,
        startup: &[],
        services: &[ACTIVATION_EFFECTS_SERVICE],
        decoder: activation_spec_decoder(),
        factory: Arc::new(ActivationDriverFactory::new(args)),
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

    use d2b_contracts_broker::broker_wire::ApplyHostGenerationHandoffResponse;
    use d2b_contracts_broker::host_generation::{
        HandoffState, SourceGenerationCompatibilityFloorV1, target_fingerprint,
    };
    use d2b_contracts_resource::v3::{
        ActivationDetail, ActivationMode, ActivationOutcomeCode, ArtifactId, NixosGenerationSpec,
        ResourcePhase, ResourceRef,
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

    use super::{
        ACTIVATION_TYPE_NAME, ActivationApplicationVerifier, ActivationController,
        ActivationDriver, ActivationDriverArgs, ActivationDriverFactory, ActivationDriverStatus,
        HostHandoffResult, RunnerRequest, activation_runner_ref, activation_spec_decoder,
        ordinal_from_name,
    };
    use crate::ActivationVerificationError;
    use crate::test_support::FakeActivationEffects;

    // -- fakes ---------------------------------------------------------------

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
            self.log.lock().push(format!("ensure:{id}")); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
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
            let mut rows = self.rows.lock(); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
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
            self.log.lock().push(format!("spawned:{id}")); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
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
            self.log.lock().push(format!( // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                "delete:{}/{}",
                key.type_name, key.name
            ));
            self.rows.lock().retain(|row| row.key != *key); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
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
            self.log.lock().push(format!( // async-gate-allow: synchronous lock acquisition, no await while the guard is held
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
        // The driver's own typed seam, scripted: production builds the same
        // seam from the facets (the factory); tests drive the behavior
        // directly over the recording double.
        Box::new(ActivationDriver::new("work".to_owned(), effects, verifier))
    }

    fn status(ctx: &ResourceContext) -> ActivationDriverStatus {
        *ctx.status::<ActivationDriverStatus>().expect("projected status")
    }

    // -- factory and validation ----------------------------------------------

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn factory_registers_only_the_generation_resource_type() {
        let factory = ActivationDriverFactory::new(ActivationDriverArgs {
            zone: "work".to_owned(),
            facets: crate::test_support::recording_facets(
                crate::test_support::RecordingBrokerDispatch::new(),
            ),
        });
        assert_eq!(factory.resource_types().len(), 1);
        assert_eq!(factory.resource_types()[0].as_str(), ACTIVATION_TYPE_NAME);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    /// The facets -> factory -> driver -> handoff seam the refactor
    /// introduced: a driver created through the factory dispatches through
    /// the facets' scripted broker and reduces the response exactly as the
    /// seam-constructed driver does, end to end through `reconcile`. A
    /// wiring mistake in `create` - dropping the facets, binding the wrong
    /// verifier - fails here, not at runtime.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_factory_wires_the_facets_into_the_created_driver_end_to_end() {
        let broker = crate::test_support::RecordingBrokerDispatch::with_responses(vec![Ok(
            ApplyHostGenerationHandoffResponse {
                target: ResourceRef::parse("Host/host-system").expect("ref"),
                state: HandoffState::Completed,
                source_generation: 1,
                target_generation: 2,
                source_remains_usable: false,
                summary: "scripted".to_owned(),
            },
        )]);
        let factory = ActivationDriverFactory::with_verifier(
            ActivationDriverArgs {
                zone: "work".to_owned(),
                facets: crate::test_support::recording_facets(broker.clone()),
            },
            Arc::new(AllowVerifier),
        );
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
        let mut d = factory
            .create(&ResourceKey::new("work", ACTIVATION_TYPE_NAME, "gen-2"))
            .await;

        assert_eq!(
            d.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );

        let requests = broker.requests();
        assert_eq!(
            requests.len(),
            1,
            "the factory-created driver dispatches through the facets"
        );
        let handoff = &requests[0];
        assert_eq!(handoff.target.to_canonical_string(), "Host/host-system");
        assert_eq!(handoff.intent.source_generation, 1);
        assert_eq!(handoff.intent.target_generation, 2);
        let projected = status(&f.ctx);
        assert_eq!(projected.phase(), ResourcePhase::Ready);
        assert_eq!(projected.detail(), ActivationDetail::Applied);
        assert_eq!(projected.outcome(), Some(ActivationOutcomeCode::Succeeded));
    }

    /// The production binding is observed too: a driver created through the
    /// production factory refuses before any dispatch (the fail-closed
    /// verifier the factory binds), so the factory cannot silently bind an
    /// allow-all gate.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_production_factory_created_driver_refuses_before_dispatch() {
        let broker = crate::test_support::RecordingBrokerDispatch::new();
        let factory = ActivationDriverFactory::new(ActivationDriverArgs {
            zone: "work".to_owned(),
            facets: crate::test_support::recording_facets(broker.clone()),
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
        let mut d = factory
            .create(&ResourceKey::new("work", ACTIVATION_TYPE_NAME, "gen-2"))
            .await;

        d.reconcile(&mut f.ctx).await.expect("reconcile");

        assert!(
            broker.requests().is_empty(),
            "the production factory's fail-closed verifier refuses before any dispatch"
        );
        let projected = status(&f.ctx);
        assert_eq!(projected.outcome(), Some(ActivationOutcomeCode::HelperRefused));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
            Arc::new(crate::FailClosedActivationVerifier),
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
