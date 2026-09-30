//! The EmergencyPolicy resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `EmergencyPolicy` rows.
//!
//! `EmergencyPolicy` carries a zone's emergency posture: the driver converges
//! it as metadata once its desired state is admitted, and the authority class
//! the policy scopes is arbitrated by the quota crate's authority index.
//!
//! The conversion is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.
//!
//! # A reduction is a decision, not a posture (R8, R36, KTD6-KTD10)
//!
//! The rest of this module is what the type exists to decide. An accepted
//! `EmergencyPolicy` row is a committed policy reduction, so it has two
//! enforcement halves and both are decided here, in the shared
//! [`AdmissionDecision`] vocabulary:
//!
//! - [`EmergencyReduction::admit_new_use`] refuses new use at the revoking
//!   stage while the reduction holds, so a request that was admitted against
//!   the earlier graph is refused against this one.
//! - [`EmergencyDrainPlan::of`] turns the Zone's open use into the ordered
//!   typed release each reservation needs: new use is fenced first, the
//!   consumer is detached while its helper legs still exist, and only then
//!   are the helpers finalized and the reservation released.
//!
//! The policy row itself is never blocked by its own reduction: an operator
//! must always be able to change or clear the emergency, or the Zone would be
//! stuck under a flag nothing can lower.
//!
//! # An outage is fenced, never converged (R36, R41)
//!
//! Every answer here is a function of the accepted policy and a census of open
//! use. A census the broker did not durably answer is not an empty census, so
//! it is passed as `None` and the plan it produces is
//! [`EnforcementState::Fenced`]: new use is blocked, no drain is reported
//! finished, and nothing may be released. Guessing that an unreachable broker
//! means "nothing is left" would thaw a Zone under a reduction that is still
//! in force.

use d2b_contracts_resource::v3::{
    AdmissionDecision, AdmissionStage, RefusalReason, ResourceRef,
};
use d2b_contracts_zone_session::v3::emergency_policy::{
    EMERGENCY_DRAIN_FINALIZER, EmergencyPolicySpec, EmergencyScope, effective_scope,
};
use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The `EmergencyPolicy` type's driver declaration.
pub fn emergency_policy_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::EMERGENCY_POLICY)
}

/// The Core finalizer an active emergency reduction holds while it drains.
///
/// The name is the contract's own, so the finalizer a driver writes and the
/// finalizer the teardown waits for cannot drift apart.
pub const fn emergency_drain_finalizer() -> &'static str {
    EMERGENCY_DRAIN_FINALIZER
}

/// The Zone's effective emergency reduction.
///
/// The reduction is the canonical union of every enabled `EmergencyPolicy`
/// row and the tightest deadline among them, so two rows that each stop one
/// thing produce a reduction that stops both. A Zone with no enabled policy
/// has no reduction, and an absent reduction admits exactly what the earlier
/// graph admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmergencyReduction {
    scope: EmergencyScope,
    drain_deadline_seconds: u32,
    policies: usize,
    row: Option<ResourceRef>,
}

impl EmergencyReduction {
    /// The reduction of a Zone with no enabled policy.
    pub const NONE: Self = Self {
        scope: EmergencyScope::new(false, false, false, false),
        drain_deadline_seconds: 0,
        policies: 0,
        row: None,
    };

    /// Derive the Zone's effective reduction from its accepted policy rows.
    pub fn of(policies: &[EmergencyPolicySpec]) -> Self {
        let enabled = policies
            .iter()
            .filter(|policy| policy.enabled())
            .count();
        match effective_scope(policies) {
            Some((scope, drain_deadline_seconds)) => Self {
                scope,
                drain_deadline_seconds,
                policies: enabled,
                row: None,
            },
            None => Self::NONE,
        }
    }

    /// Record the committed row this reduction was read from.
    ///
    /// The binding is by type, not by name: every `EmergencyPolicy` row *is*
    /// the reduction rather than something it blocks, so an operator must
    /// always be able to change or clear it or the Zone is stuck under a flag
    /// nothing can lower. An unbound reduction blocks every row, which is the
    /// safe direction - it can only refuse more, never admit more.
    pub fn bind_to(mut self, row: ResourceRef) -> Self {
        self.row = Some(row);
        self
    }

    /// The committed row this reduction was read from, and with it the type
    /// this reduction governs.
    pub const fn row(&self) -> Option<&ResourceRef> {
        self.row.as_ref()
    }

    /// Whether this candidate is the reduction rather than something it
    /// blocks.
    fn blocks(&self, target: &ResourceRef) -> bool {
        !self
            .row
            .as_ref()
            .is_some_and(|row| row.resource_type() == target.resource_type())
    }

    /// Whether any policy is enabled.
    pub const fn is_active(&self) -> bool {
        self.policies > 0
    }

    /// The effective scope flags.
    pub const fn scope(&self) -> EmergencyScope {
        self.scope
    }

    /// The tightest drain deadline the enabled policies admit.
    pub const fn drain_deadline_seconds(&self) -> u32 {
        self.drain_deadline_seconds
    }

    /// How many accepted rows contributed to this reduction.
    pub const fn policies(&self) -> usize {
        self.policies
    }

    /// Decide one new use against this reduction.
    ///
    /// A reduction that stops new admissions refuses at the revoking stage,
    /// which is the stage that means "new use is blocked ahead of typed
    /// release". The refusal names the reduction and never the policy row's
    /// text, so a diagnostic cannot echo operator prose.
    pub fn admit_new_use(&self) -> AdmissionDecision {
        if self.scope.stop_new_admissions() {
            return AdmissionDecision::refuse(
                AdmissionStage::Revoke,
                RefusalReason::EmergencyReductionActive,
            );
        }
        AdmissionDecision::Admitted
    }

    /// Decide one new row against this reduction.
    ///
    /// The same decision as [`Self::admit_new_use`] for every row outside the
    /// reduction's own type, which is always admitted so the emergency can be
    /// changed or cleared. A reduction with no bound row blocks every row.
    pub fn admit_new_row(&self, target: &ResourceRef) -> AdmissionDecision {
        if !self.blocks(target) {
            return AdmissionDecision::Admitted;
        }
        self.admit_new_use()
    }
}

/// Whether the Zone still has new use to block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewUseState {
    /// The reduction blocks new use.
    Blocked,
    /// The reduction does not block new use.
    Admitted,
}

/// Whether the Zone's links are disconnected by the reduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneLinkState {
    /// The reduction disconnects every ZoneLink.
    Disconnected,
    /// The reduction leaves links as they are.
    Retained,
}

/// Whether the Zone's provider component processes are stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderProcessState {
    /// The reduction stops provider component processes.
    Stopped,
    /// The reduction leaves provider processes running.
    Retained,
}

/// How far a reduction has been carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnforcementState {
    /// Every outstanding use reached its declared safe state and nothing is
    /// left to release.
    Converged,
    /// The reduction is in force and use is still outstanding.
    Pending,
    /// The Zone's open use could not be established, so the reduction holds
    /// with conservative ownership and nothing is reported as finished.
    Fenced,
}

/// One step of one reservation's typed release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainStep {
    /// Block new use on the reservation before anything is detached.
    FenceNewUse,
    /// Detach the consumer while its helper legs still hold admitted use.
    DetachConsumer,
    /// Finalize one helper implementation child.
    FinalizeHelper(ResourceRef),
    /// Release the source reservation.
    ReleaseSource,
}

/// One open use the reduction has to drive to its declared safe state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReservationDrain {
    source: ResourceRef,
    consumer: ResourceRef,
    helpers: Vec<ResourceRef>,
    steps: Vec<DrainStep>,
}

impl ReservationDrain {
    /// The source whose reservation is held.
    pub const fn source(&self) -> &ResourceRef {
        &self.source
    }

    /// The consumer that must stop using it.
    pub const fn consumer(&self) -> &ResourceRef {
        &self.consumer
    }

    /// The helper implementation children that hold attenuated legs of the
    /// same reservation.
    pub fn helpers(&self) -> &[ResourceRef] {
        &self.helpers
    }

    /// The steps in the order they must happen.
    pub fn steps(&self) -> &[DrainStep] {
        &self.steps
    }
}

/// One open use: a consumer on a source reservation, with its helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenUse {
    source: ResourceRef,
    consumer: ResourceRef,
    helpers: Vec<ResourceRef>,
}

impl OpenUse {
    /// Record one open use and the helpers that hold legs of its reservation.
    pub fn new(
        source: ResourceRef,
        consumer: ResourceRef,
        helpers: impl IntoIterator<Item = ResourceRef>,
    ) -> Self {
        Self { source, consumer, helpers: helpers.into_iter().collect() }
    }

    /// The source whose reservation is held.
    pub const fn source(&self) -> &ResourceRef {
        &self.source
    }

    /// The consumer using it.
    pub const fn consumer(&self) -> &ResourceRef {
        &self.consumer
    }

    /// The helper implementation children.
    pub fn helpers(&self) -> &[ResourceRef] {
        &self.helpers
    }
}

/// What the Zone can prove about its own open use.
///
/// A census is a statement about committed reservations and the use still
/// outstanding on them. An absent census is not this type: the broker could
/// not answer, and [`EmergencyDrainPlan::of`] takes `None` for that so the two
/// can never be confused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenUseCensus {
    sources: Vec<ResourceRef>,
    zone_links: u32,
    provider_processes: u32,
    outstanding: Vec<OpenUse>,
}

impl OpenUseCensus {
    /// Record the Zone's reservations and the use still outstanding on them.
    pub fn new(
        sources: impl IntoIterator<Item = ResourceRef>,
        zone_links: u32,
        provider_processes: u32,
        outstanding: impl IntoIterator<Item = OpenUse>,
    ) -> Self {
        Self {
            sources: sources.into_iter().collect(),
            zone_links,
            provider_processes,
            outstanding: outstanding.into_iter().collect(),
        }
    }

    /// The reservations the broker holds.
    pub fn sources(&self) -> &[ResourceRef] {
        &self.sources
    }

    /// The ZoneLinks currently connected.
    pub const fn zone_links(&self) -> u32 {
        self.zone_links
    }

    /// The provider component processes still running.
    pub const fn provider_processes(&self) -> u32 {
        self.provider_processes
    }

    /// The use that has not reached its safe state yet.
    pub fn outstanding(&self) -> &[OpenUse] {
        &self.outstanding
    }
}

/// The ordered work one emergency reduction requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmergencyDrainPlan {
    new_use: NewUseState,
    zone_links: ZoneLinkState,
    provider_processes: ProviderProcessState,
    drains: Vec<ReservationDrain>,
    deadline_seconds: u32,
    state: EnforcementState,
}

impl EmergencyDrainPlan {
    /// Whether the reduction blocks new use.
    pub const fn new_use(&self) -> NewUseState {
        self.new_use
    }

    /// What the reduction does to ZoneLinks.
    pub const fn zone_links(&self) -> ZoneLinkState {
        self.zone_links
    }

    /// What the reduction does to provider component processes.
    pub const fn provider_processes(&self) -> ProviderProcessState {
        self.provider_processes
    }

    /// One typed release per reservation that still holds use.
    pub fn drains(&self) -> &[ReservationDrain] {
        &self.drains
    }

    /// The drain deadline the enabled policies admit.
    pub const fn deadline_seconds(&self) -> u32 {
        self.deadline_seconds
    }

    /// How far the reduction has been carried.
    pub const fn state(&self) -> EnforcementState {
        self.state
    }

    /// Whether the reduction has been carried all the way.
    pub const fn is_converged(&self) -> bool {
        matches!(self.state, EnforcementState::Converged)
    }

    /// Whether a reservation may be released.
    ///
    /// A release is a claim that nothing still holds the source, so it is
    /// permitted only once the reduction has converged. A pending or fenced
    /// plan refuses it: the owner is still there, and the Zone is still
    /// accountable for it.
    pub const fn may_release(&self) -> bool {
        self.is_converged()
    }
}

/// Plan one emergency reduction against the Zone's open use.
///
/// The plan is the whole ordered reduction: fence, then detach each consumer
/// while its helpers still exist, then finalize the helpers, then release.
/// A reservation whose source the census does not name cannot be released
/// safely - the plan does not know who owns it - so the plan is
/// [`EnforcementState::Fenced`] with conservative ownership rather than a
/// partial release that would free a source someone still holds.
pub fn plan_drain(
    reduction: &EmergencyReduction,
    census: Option<&OpenUseCensus>,
) -> EmergencyDrainPlan {
    let new_use = if reduction.scope().stop_new_admissions() {
        NewUseState::Blocked
    } else {
        NewUseState::Admitted
    };
    let zone_links = if reduction.scope().disconnect_zone_links() {
        ZoneLinkState::Disconnected
    } else {
        ZoneLinkState::Retained
    };
    let provider_processes = if reduction.scope().stop_provider_processes() {
        ProviderProcessState::Stopped
    } else {
        ProviderProcessState::Retained
    };
    let Some(census) = census.filter(|census| {
        !reduction.is_active()
            || census
                .outstanding()
                .iter()
                .all(|open| census.sources().contains(open.source()))
    }) else {
        // The reduction holds, the Zone is fenced, and nothing is reported
        // as drained: the conservative answer is the only one that cannot
        // free a source a consumer still holds.
        return EmergencyDrainPlan {
            new_use: NewUseState::Blocked,
            zone_links,
            provider_processes,
            drains: Vec::new(),
            deadline_seconds: reduction.drain_deadline_seconds(),
            state: if reduction.is_active() {
                EnforcementState::Fenced
            } else {
                EnforcementState::Converged
            },
        };
    };
    let drains = if reduction.scope().drain_ongoing_operations() {
        census
            .outstanding()
            .iter()
            .map(|open| ReservationDrain {
                source: open.source.clone(),
                consumer: open.consumer.clone(),
                helpers: open.helpers.clone(),
                steps: drain_steps(open),
            })
            .collect()
    } else {
        Vec::new()
    };
    let state = if drains.is_empty()
        && (zone_links == ZoneLinkState::Retained || census.zone_links() == 0)
        && (provider_processes == ProviderProcessState::Retained
            || census.provider_processes() == 0)
    {
        EnforcementState::Converged
    } else {
        EnforcementState::Pending
    };
    EmergencyDrainPlan {
        new_use,
        zone_links,
        provider_processes,
        drains,
        deadline_seconds: reduction.drain_deadline_seconds(),
        state,
    }
}

/// The steps one open use needs, in the order they must happen.
///
/// The order is the whole point: the consumer is detached while its helpers
/// still hold their legs, because a helper that is finalized first is a
/// helper whose cleanup has nothing left to detach, and the source is
/// released last, because a release is a claim that nobody holds it.
fn drain_steps(open: &OpenUse) -> Vec<DrainStep> {
    let mut steps = vec![DrainStep::FenceNewUse, DrainStep::DetachConsumer];
    steps.extend(open.helpers().iter().cloned().map(DrainStep::FinalizeHelper));
    steps.push(DrainStep::ReleaseSource);
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "Volume/data";
    const CONSUMER: &str = "Guest/vm";
    const HELPER: &str = "Endpoint/virtiofsd";
    const SECOND_SOURCE: &str = "Device/gpu";

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).expect("a canonical reference")
    }

    fn policy(enabled: bool, scope: EmergencyScope, deadline: u32) -> EmergencyPolicySpec {
        EmergencyPolicySpec::new(enabled, scope, deadline, "operator reduction")
            .expect("the policy validates")
    }

    /// A reduction that blocks new use and drains what is already running.
    fn reducing() -> EmergencyReduction {
        EmergencyReduction::of(&[policy(
            true,
            EmergencyScope::new(true, false, false, true),
            30,
        )])
    }

    fn census() -> OpenUseCensus {
        OpenUseCensus::new(
            [reference(SOURCE), reference(SECOND_SOURCE)],
            0,
            0,
            [OpenUse::new(
                reference(SOURCE),
                reference(CONSUMER),
                [reference(HELPER)],
            )],
        )
    }

    #[test]
    fn a_disabled_zone_has_no_reduction_and_admits_what_the_earlier_graph_admitted() {
        let reduction = EmergencyReduction::of(&[policy(
            false,
            EmergencyScope::new(true, true, true, true),
            30,
        )]);
        assert!(!reduction.is_active());
        assert_eq!(reduction.admit_new_use(), AdmissionDecision::Admitted);
        let plan = plan_drain(&reduction, None);
        assert_eq!(plan.state(), EnforcementState::Converged);
        assert!(plan.may_release());
    }

    #[test]
    fn enabled_policies_compose_toward_the_stricter_reduction_and_the_tighter_deadline() {
        let reduction = EmergencyReduction::of(&[
            policy(true, EmergencyScope::new(true, false, false, false), 120),
            policy(true, EmergencyScope::new(false, true, false, true), 45),
            policy(false, EmergencyScope::new(true, true, true, true), 5),
        ]);
        assert_eq!(reduction.policies(), 2, "a disabled row contributes nothing");
        assert!(reduction.scope().stop_new_admissions());
        assert!(reduction.scope().disconnect_zone_links());
        assert!(reduction.scope().drain_ongoing_operations());
        assert!(!reduction.scope().stop_provider_processes());
        assert_eq!(reduction.drain_deadline_seconds(), 45);
    }

    #[test]
    fn a_reduction_refuses_new_use_at_the_revoking_stage_with_its_own_reason() {
        assert_eq!(
            reducing().admit_new_use(),
            AdmissionDecision::refuse(
                AdmissionStage::Revoke,
                RefusalReason::EmergencyReductionActive
            )
        );
        // A reduction that only drains keeps admitting new use: the two
        // flags are separate decisions, not one "emergency on" switch.
        let drain_only = EmergencyReduction::of(&[policy(
            true,
            EmergencyScope::new(false, false, false, true),
            30,
        )]);
        assert_eq!(drain_only.admit_new_use(), AdmissionDecision::Admitted);
    }

    #[test]
    fn existing_use_drains_in_order_and_helpers_outlive_the_consumer_detach() {
        let plan = plan_drain(&reducing(), Some(&census()));
        assert_eq!(plan.new_use(), NewUseState::Blocked);
        assert_eq!(plan.state(), EnforcementState::Pending);
        assert!(!plan.may_release(), "outstanding use is still held");
        assert_eq!(plan.drains().len(), 1);
        let drain = &plan.drains()[0];
        assert_eq!(drain.source(), &reference(SOURCE));
        assert_eq!(drain.consumer(), &reference(CONSUMER));
        assert_eq!(
            drain.steps(),
            [
                DrainStep::FenceNewUse,
                DrainStep::DetachConsumer,
                DrainStep::FinalizeHelper(reference(HELPER)),
                DrainStep::ReleaseSource,
            ],
            "the consumer detaches while its helper leg still exists, and the source is released last"
        );
    }

    #[test]
    fn a_zone_with_nothing_outstanding_has_converged_and_may_release() {
        let empty = OpenUseCensus::new([reference(SOURCE)], 0, 0, []);
        let plan = plan_drain(&reducing(), Some(&empty));
        assert_eq!(plan.state(), EnforcementState::Converged);
        assert!(plan.is_converged());
        assert!(plan.may_release());
        assert!(plan.drains().is_empty());
    }

    #[test]
    fn a_census_the_broker_could_not_answer_fences_the_zone_instead_of_converging_it() {
        let plan = plan_drain(&reducing(), None);
        assert_eq!(plan.state(), EnforcementState::Fenced);
        assert!(!plan.is_converged(), "an unreachable broker is not an empty Zone");
        assert!(!plan.may_release());
        assert_eq!(plan.new_use(), NewUseState::Blocked, "the reduction still holds");
        assert!(plan.drains().is_empty(), "nothing is reported as drained");
        assert_eq!(plan.deadline_seconds(), 30);
    }

    #[test]
    fn open_use_whose_source_the_census_cannot_name_is_fenced_not_partially_released() {
        let dangling = OpenUseCensus::new(
            [reference(SOURCE)],
            0,
            0,
            [OpenUse::new(
                reference(SECOND_SOURCE),
                reference(CONSUMER),
                [reference(HELPER)],
            )],
        );
        let plan = plan_drain(&reducing(), Some(&dangling));
        assert_eq!(plan.state(), EnforcementState::Fenced);
        assert!(!plan.may_release());
    }

    #[test]
    fn a_reduction_that_stops_provider_processes_waits_for_them_to_be_gone() {
        let stopping = EmergencyReduction::of(&[policy(
            true,
            EmergencyScope::new(true, false, true, true),
            10,
        )]);
        let plan = plan_drain(&stopping, Some(&census()));
        assert_eq!(plan.provider_processes(), ProviderProcessState::Stopped);
        assert_eq!(plan.state(), EnforcementState::Pending);
        let settled = OpenUseCensus::new([reference(SOURCE)], 0, 0, []);
        assert_eq!(
            plan_drain(&stopping, Some(&settled)).state(),
            EnforcementState::Converged
        );
    }

    #[test]
    fn a_reduction_that_asks_for_no_drain_schedules_no_release_of_untouched_reservations() {
        // Blocking new use without asking for a drain leaves committed
        // reservations alone: a reduction may not release use it was not
        // asked to drain, or a policy flag would retire sources.
        let block_only = EmergencyReduction::of(&[policy(
            true,
            EmergencyScope::new(true, false, false, false),
            30,
        )]);
        let plan = plan_drain(&block_only, Some(&census()));
        assert!(plan.drains().is_empty());
        assert_eq!(plan.new_use(), NewUseState::Blocked);
        assert_eq!(plan.state(), EnforcementState::Converged);
    }

    #[test]
    fn a_reduction_never_blocks_its_own_type() {
        let reduction = reducing().bind_to(reference("EmergencyPolicy/zone"));
        for policy_row in ["EmergencyPolicy/zone", "EmergencyPolicy/clear"] {
            assert_eq!(
                reduction.admit_new_row(&reference(policy_row)),
                AdmissionDecision::Admitted,
                "an operator must always be able to change or clear the emergency"
            );
        }
        assert_eq!(
            reduction.admit_new_row(&reference("Process/job")),
            AdmissionDecision::refuse(
                AdmissionStage::Revoke,
                RefusalReason::EmergencyReductionActive
            )
        );
    }

    #[test]
    fn the_drain_finalizer_is_the_contract_s_own_name() {
        assert_eq!(emergency_drain_finalizer(), "core.emergency-drain");
    }
}
