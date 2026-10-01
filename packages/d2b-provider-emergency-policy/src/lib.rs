//! The EmergencyPolicy provider crate: the EmergencyPolicy resource type's driver.
//!
//! The crate owns the EmergencyPolicy type's identity, the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by, and the conversion that makes the type mean
//! something: the driver in [`serving`] reads a committed policy row, publishes
//! the Zone's effective reduction, and holds the drain finalizer while the
//! Zone's open use is still outstanding.
//!
//! `EmergencyPolicy` carries a Zone's emergency posture. What the reduction
//! refuses, and the ordered typed release it drives existing use through, are
//! in [`driver`] and are decided here rather than in the daemon composition.
//! The two daemon-owned facts the reduction acts on - the Zone's open use and
//! the live reduction the manager-boundary admission reads - are declared as
//! the per-Zone runtime in [`facets`].

#![deny(missing_docs)]

mod driver;
mod facets;
mod serving;

pub use driver::{
    DrainStep, EmergencyDrainPlan, EmergencyReduction, EnforcementState, NewUseState, OpenUse,
    OpenUseCensus, ProviderProcessState, ReservationDrain, ZoneLinkState, plan_drain,
};
pub use facets::{OpenUseSource, ZoneEmergencyRuntime, install, remove, runtime};
pub use serving::{
    EmergencyPolicyDriver, EmergencyPolicyDriverFactory, EmergencyPolicyError,
    EmergencyPolicyStatus, emergency_policy_descriptor, emergency_policy_of_spec,
    emergency_spec_decoder, held_drain_finalizer,
};

use d2b_contracts_zone_session::v3::emergency_policy::EMERGENCY_DRAIN_FINALIZER;

/// The Core finalizer an active emergency reduction holds while it drains.
///
/// The name is the contract's own, so the finalizer a driver writes and the
/// finalizer the teardown waits for cannot drift apart.
pub const fn emergency_drain_finalizer() -> &'static str {
    EMERGENCY_DRAIN_FINALIZER
}
