//! The Process resource family.
//!
//! This crate is the family's home: the driver for the `Process` and
//! `EphemeralProcess` resource types, their declarations, the spec decoder
//! and driver factory the registry serves, the neutral launch and
//! supervision primitives every Process Provider exchanges - and the
//! implementation of the family's driver effects (U1), served by this crate
//! itself over the daemon-supplied declared facets ([`facets`]) and hosted
//! per zone as a declared service ([`effects_service`]).

#![deny(missing_docs)]

pub mod backend;
pub mod driver;
pub mod effects;
pub mod effects_service;
pub mod execution;
pub mod facets;
pub mod identity;
pub mod launch_identity;
pub mod operations;
pub mod worker_launch;

// The scripted ProcessProviderRuntime recording double. Needed both by
// external crates (d2bd's plane tests, which opt in via the `test-support`
// feature) and by this crate's own tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically when
// compiling this crate's tests, so `cargo test -p d2b-provider-process` works
// without remembering `--features test-support`.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use backend::{
    BackendLaunch, BackendObservation, LaunchedSnapshot, ProcessEffectBackend, ProcessEffectError,
    ProcessLaunchRequest, ProcessRequest, ProcessStopClass,
};
pub use driver::{
    DeviceWorkerFamily, GuestOwnerIdentitySource, ProcessDriverArgs, device_worker_family,
    device_worker_vm, process_family_descriptors, process_spec_decoder, resolve_guest_owner_uid,
    resource_uid_from_bytes,
};
pub use effects::{ProcessDriverEffects, ProviderAdoption, ProviderLiveness};
pub use effects_service::{
    PROCESS_EFFECTS_SERVICE, ProcessEffectsService, ProcessEffectsServiceFactory,
};
pub use execution::{ExecutionMode, execution_target_allowed};
pub use facets::{
    CommittedProviderIdentitySource, ProcessEffectFacets, ProcessProviderRuntime,
    ProcessResourceContext, ProviderLaunch,
};
pub use identity::{ProcessFamilySpec, ProcessResourceIdentity, decode_metadata_owner_ref};
pub use launch_identity::{LaunchRow, resolve_launch_identity};
pub use operations::{INSPECT_PROCESS_FAMILY, INVALID_PROCESS_TYPE, process_family_operations};
pub use worker_launch::{
    DeviceWorkerLaunch, GpuWorkerParams, ServingWorkerLaunch, ServingWorkerRoot, SwtpmFlushParams,
    SwtpmWorkerParams, VideoWorkerParams,
};

pub use d2b_process_conformance::{
    AdoptionCandidate, CompiledDigests, ConfigurationDigest, IdentityBinding, LaunchTicket,
    LaunchedProcess, ObservedIdentity, OperationBinding, PidfdEvidence, ProcessConformanceError,
    ProcessIdentityDigest, ProcessLaunchEffectPort, StopClass, WaitReapOwner,
};
