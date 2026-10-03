//! The serving `EmergencyPolicy` driver (U40, R36).
//!
//! The declaration-only metadata driver converged an `EmergencyPolicy` row as
//! an opaque JSON object and enforced nothing. This module is the row's actual
//! conversion: the driver reads its own committed policy, publishes the Zone's
//! effective reduction where the manager-boundary admission reads it, and
//! holds [`emergency_drain_finalizer`](crate::emergency_drain_finalizer) while
//! the Zone's open use is still outstanding.
//!
//! # What the driver does and does not decide
//!
//! The driver never refuses a mutation. Admission is the manager boundary's
//! job, and the reduction reaches it by being published rather than by being
//! checked here: a check inside a driver runs after the row is committed, so
//! refusing there would leave the row behind. What the driver owns is the
//! Zone's reduction as a published fact and the drain that the reduction
//! requires.
//!
//! # The finalizer is what makes the drain a gate
//!
//! While a reduction holds, this row must not be removed out from under use
//! the reduction is draining. `pre_drain` is the step that runs after the
//! durable deleting mark is committed and before anything is released, so it
//! is where the finalizer the contract names is held: a reduction that has not
//! converged keeps the pass failing, and the teardown waits rather than
//! cutting a live consumer off its source.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_zone_session::v3::emergency_policy::{
    EMERGENCY_POLICY_RESOURCE_TYPE, EmergencyPolicySpec,
};
use d2b_resource_runtime::context::{ResourceContext, SpecDecoder, typed_spec_decoder};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureKinds};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};

use crate::facets::ZoneEmergencyRuntime;
use crate::{
    EmergencyDrainPlan, EmergencyReduction, EnforcementState, emergency_drain_finalizer,
};

/// Why one committed `EmergencyPolicy` row was not read as a policy.
///
/// Every variant is a refusal to decide. A row whose scope, deadline, or
/// reason this contract does not admit is refused rather than reconciled with
/// its authority dropped: a reduction that silently lost its scope would
/// report a Zone as fenced while admitting everything into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmergencyPolicyError {
    /// The committed desired state is not an `EmergencyPolicy` row.
    UnreadableDesiredState,
    /// The row is this Zone's, but the policy it states does not validate.
    InvalidPolicy,
    /// The row belongs to another Zone, so this driver will not reduce it.
    ForeignZone(String),
    /// The reduction is in force and the Zone's open use has not drained, so
    /// the finalizer this row holds is not released yet.
    DrainPending,
}

impl core::fmt::Display for EmergencyPolicyError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnreadableDesiredState => {
                formatter.write_str("emergency-policy-desired-state-unreadable")
            }
            Self::InvalidPolicy => formatter.write_str("emergency-policy-invalid"),
            Self::ForeignZone(zone) => write!(formatter, "emergency-policy-foreign-zone: {zone}"),
            Self::DrainPending => formatter.write_str("emergency-drain-pending"),
        }
    }
}

impl std::error::Error for EmergencyPolicyError {}

/// Read one committed `EmergencyPolicy` row's desired state as its policy.
///
/// The bytes are the canonical desired-state object the manager stored and the
/// decode is the contract's own closed wire mirror, so a field the contract
/// does not declare is refused rather than ignored.
pub fn emergency_policy_of_spec(
    desired_state: &[u8],
) -> Result<EmergencyPolicySpec, EmergencyPolicyError> {
    serde_json::from_slice(desired_state).map_err(|_| EmergencyPolicyError::UnreadableDesiredState)
}

/// The manager-wired decode hook for the `EmergencyPolicy` type.
///
/// The stored spec envelope IS the policy row, so the decode is the policy
/// itself rather than the generic JSON object the metadata driver accepted.
pub fn emergency_spec_decoder() -> std::sync::Arc<dyn SpecDecoder> {
    typed_spec_decoder(emergency_policy_of_spec)
}

/// The `EmergencyPolicy` resource's published enforcement state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmergencyPolicyStatus {
    /// The reduction this row currently states.
    pub reduction: EmergencyReduction,
    /// How far the reduction has been carried.
    pub state: EnforcementState,
    /// The tightest deadline the enabled policies admit.
    pub deadline_seconds: u32,
    /// How many reservations still hold outstanding use.
    pub pending_drains: usize,
}

/// The `EmergencyPolicy` resource's driver (U40, R36).
///
/// The driver is built over its Zone's [`ZoneEmergencyRuntime`], which the
/// composition root installs. A Zone with no installed runtime has no
/// reduction to publish and no open use to drain, so the driver reports that
/// rather than inventing either.
pub struct EmergencyPolicyDriver {
    zone: d2b_contracts_resource::v3::ZoneId,
    runtime: Option<std::sync::Arc<ZoneEmergencyRuntime>>,
}

impl EmergencyPolicyDriver {
    /// Build the driver for one Zone.
    pub fn new(
        zone: d2b_contracts_resource::v3::ZoneId,
        runtime: Option<std::sync::Arc<ZoneEmergencyRuntime>>,
    ) -> Self {
        Self { zone, runtime }
    }

    /// The exact reference this row carries.
    ///
    /// The Zone is checked from the durable key rather than from the decoded
    /// spec: a row whose key names another Zone is refused, so a policy
    /// committed under the wrong Zone cannot reduce this one.
    fn own_reference(&self, ctx: &ResourceContext) -> Result<ResourceRef, EmergencyPolicyError> {
        let key = ctx.key();
        if key.zone != self.zone.as_str() {
            return Err(EmergencyPolicyError::ForeignZone(key.zone.clone()));
        }
        ResourceRef::parse(&format!("{}/{}", key.type_name, key.name))
            .map_err(|_| EmergencyPolicyError::UnreadableDesiredState)
    }

    /// The reduction this row states, bound to its own reference so it can
    /// never block the row that carries it.
    fn reduction(&self, ctx: &ResourceContext) -> Result<EmergencyReduction, EmergencyPolicyError> {
        let reference = self.own_reference(ctx)?;
        let spec: &EmergencyPolicySpec = ctx
            .spec::<EmergencyPolicySpec>()
            .map_err(|_| EmergencyPolicyError::UnreadableDesiredState)?;
        Ok(EmergencyReduction::of(std::slice::from_ref(spec)).bind_to(reference))
    }

    /// The plan this row's reduction drives against the Zone's open use.
    ///
    /// A Zone with no installed runtime has no census to offer, and an absent
    /// census is a fenced Zone rather than an empty one.
    async fn plan(&self, reduction: &EmergencyReduction) -> EmergencyDrainPlan {
        match self.runtime.as_ref() {
            Some(runtime) => runtime.plan(reduction).await,
            None => crate::plan_drain(reduction, None),
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriver for EmergencyPolicyDriver {
    type Error = EmergencyPolicyError;

    fn classify_error(&self, error: &Self::Error) -> DriverFailure {
        // A row this crate cannot read as a policy is terminal against the
        // row, not a transient read failure: retrying the same bytes decodes
        // the same way. A drain that has not converged is the opposite - it
        // is exactly what a later pass resolves, so it defers and requeues.
        match error {
            EmergencyPolicyError::DrainPending => DriverFailure::not_yet_because(
                DriverOp::Delete,
                FailureKinds::CORE_DRAIN_PENDING,
                format!("{}: {}", held_drain_finalizer(), error),
            ),
            _ => DriverFailure::refused_because(
                DriverOp::Validate,
                FailureKinds::CORE_SPEC_INVALID,
                error.to_string(),
            ),
        }
    }
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        self.own_reference(ctx)?;
        // The policy is read here rather than only at reconcile so a row that
        // cannot state a reduction never reaches the point where it would
        // publish one.
        let _: &EmergencyPolicySpec = ctx
            .spec::<EmergencyPolicySpec>()
            .map_err(|_| EmergencyPolicyError::UnreadableDesiredState)?;
        Ok(())
    }

    async fn recover(&mut self, _ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        // A policy row realizes nothing on a target, so there is no local
        // state to discover. The reduction it states is the whole row, and
        // `reconcile` republishes it from the committed spec.
        Ok(RecoveryOutcome::Adopted)
    }

    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
        let reduction = self.reduction(ctx)?;
        // The row is only enforcement once the manager-boundary admission can
        // read it, so publication is the driver's own effect rather than a
        // side effect of a row that merely converged.
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.publish(&reduction);
        }
        let plan = self.plan(&reduction).await;
        ctx.set_status(EmergencyPolicyStatus {
            reduction,
            state: plan.state(),
            deadline_seconds: plan.deadline_seconds(),
            pending_drains: plan.drains().len(),
        });
        Ok(ReconcileOutcome::Satisfied)
    }

    async fn pre_drain(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        // The finalizer is the contract's own name and is what the teardown
        // waits on. While the reduction holds and the Zone's open use has not
        // drained, this row keeps the pass failing, so the row cannot be
        // released out from under a consumer the reduction is still draining.
        let reduction = self.reduction(ctx)?;
        if !reduction.is_active() {
            return Ok(());
        }
        if self.plan(&reduction).await.is_converged() {
            return Ok(());
        }
        Err(EmergencyPolicyError::DrainPending)
    }

    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        // A removed policy row publishes no reduction. That is exactly what
        // lets a Zone recover, so the teardown clears the publication rather
        // than leaving the last reduction in force over a row that is gone.
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.publish(&EmergencyReduction::NONE);
        }
        Ok(())
    }
}

/// [`ResourceDriverFactory`] for the `EmergencyPolicy` type.
pub struct EmergencyPolicyDriverFactory {
    zone: d2b_contracts_resource::v3::ZoneId,
}

impl EmergencyPolicyDriverFactory {
    /// Build the factory for one Zone.
    ///
    /// The Zone's runtime is resolved per driver rather than captured here:
    /// the composition root installs it while it builds the plane, and one
    /// registered descriptor serves every Zone a process runs.
    pub fn new(zone: d2b_contracts_resource::v3::ZoneId) -> Self {
        Self { zone }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for EmergencyPolicyDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        // The type name is the contract's own constant, so the descriptor and
        // the decoder cannot drift apart.
        static TYPES: std::sync::LazyLock<Vec<ResourceTypeName>> = std::sync::LazyLock::new(|| {
            vec![ResourceTypeName::new(EMERGENCY_POLICY_RESOURCE_TYPE)]
        });
        &TYPES
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(EmergencyPolicyDriver::new(
            self.zone.clone(),
            crate::facets::runtime(&self.zone),
        ))
    }
}

/// The `EmergencyPolicy` type's driver declaration (U40, R36).
///
/// This is a serving declaration, not the declaration-only metadata one: the
/// type realizes no target-local state, but it does own the Zone's emergency
/// reduction, and the decoder and factory below are what carry that.
pub fn emergency_policy_descriptor(
    zone: d2b_contracts_resource::v3::ZoneId,
) -> d2b_resource_types::DriverDescriptor {
    use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};
    DriverDescriptor {
        resource_type: WellKnownType::EMERGENCY_POLICY,
        allowed_sources: AllowedSources::BUILTIN,
        verbs: CONVERTED_TYPE_VERBS,
        // The type names no placement anchor, so the plane reconciles it on
        // its own Host domain, exactly as the declaration-only form did.
        execution: d2b_resource_runtime::metadata::METADATA_EXECUTION_DOMAINS,
        exportable: false,
        reads: &[],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: emergency_spec_decoder(),
        factory: std::sync::Arc::new(EmergencyPolicyDriverFactory::new(zone)),
    }
}

/// The finalizer this crate's driver holds while a reduction drains, and the
/// name the teardown waits for.
///
/// Re-exported here so the driver's gate and the contract's name are read from
/// one place: a driver that held a different string than the contract would
/// make the teardown wait on a finalizer nothing ever wrote.
pub const fn held_drain_finalizer() -> &'static str {
    emergency_drain_finalizer()
}
