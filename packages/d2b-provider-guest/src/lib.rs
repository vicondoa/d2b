//! The Guest resource family: the driver for the `Guest` type over the four
//! runtime Providers that realize it, the driver declaration the resource
//! plane registers the type by, and the Guest-side target-control service the
//! guest daemon serves.
//!
//! One factory serves every runtime Provider row of the family
//! (`runtime-cloud-hypervisor`, `runtime-qemu-media`,
//! `runtime-azure-container-apps`, `runtime-azure-virtual-machine`): a stored
//! spec selects its Provider row, and that selection picks the kind whose
//! children, effect keying, and teardown order the driver preserves. The
//! kinds' identity, controller, and Provider references come from the
//! realizer crates' own constants, so the family table cannot drift from the
//! Providers it serves.
//!
//! Everything the driver needs from outside arrives through the driver
//! effect port ([`GuestDriverEffects`]): the Cloud Hypervisor controller
//! session (the real host path) and the preserved framework state machines
//! the daemon drives for the other three kinds. The production implementation
//! lives in the daemon behind that port, so this crate owns no host state and
//! depends on no daemon runtime.

#![deny(missing_docs)]

pub mod driver;
pub mod guest_spec;
pub mod shutdown;
pub mod target_control;
pub mod target_service;

// The scripted GuestDriverEffects recording double. Needed both by
// external crates (d2bd's plane tests, which opt in via the `test-support`
// feature) and by this crate's own tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically to
// this crate's unit tests. Integration tests that need it declare
// `required-features`, so run those with `--features test-support` (or let
// the Bazel `*_test_support` target compile them).
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use driver::{
    GUEST_REGISTRATIONS, GUEST_TYPE_NAME, GuestChildObservation, GuestChildSurface, GuestDriverArgs,
    GuestDriverEffects, GuestDriverFactory, GuestDriverStatus, GuestEffectError, GuestEffectOutcome,
    GuestEffectPhase, GuestEffectRequest, GuestFinalizeStage, GuestKind, GuestRegistration,
    GuestStatusSink, declared_dependency_refs, decode_metadata, guest_descriptor,
    guest_spec_decoder, guest_status_sink, key_ref, resource_uid, view_phase,
};
pub use guest_spec::{GUEST_RESOURCE_TYPE, GuestSpec};
pub use shutdown::{
    CloudHypervisorShutdown, GracefulVmShutdown, ProviderGuestState, ProviderKind,
    ProviderRequestOutcome, ProviderShutdownTarget, ProviderVmmExitOutcome,
};
pub use target_control::{
    GuestTargetSession, SessionTargetControlChannel, guest_target_ref, session_target_control,
};
pub use target_service::{
    GuestTargetEffect, GuestTargetEffectError, GuestTargetEffects, GuestTargetRefusal,
    GuestTargetService, production_guest_target_effects, target_control_services,
};
