//! The Process resource family.
//!
//! This crate is the family's home: the driver for the `Process` and
//! `EphemeralProcess` resource types, their declarations, the spec decoder
//! and driver factory the registry serves, and the neutral launch and
//! supervision primitives every Process Provider exchanges. The production
//! effect implementation stays in the daemon behind the effect port declared
//! here, so the family owns the seam and the daemon owns the host state.

#![deny(missing_docs)]

pub mod backend;
pub mod driver;
pub mod effects;
pub mod execution;
pub mod identity;
pub mod launch_identity;
pub mod operations;
pub mod worker_launch;

// The scripted ProcessDriverEffects recording double. Needed both by
// external crates (d2bd's plane tests, which opt in via the `test-support`
// feature) and by this crate's own tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically when
// compiling this crate's tests, so `cargo test -p d2b-provider-process` works
// without remembering `--features test-support`.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use backend::{
    BackendLaunch, BackendObservation, ProcessEffectBackend, ProcessEffectError,
    ProcessLaunchRequest, ProcessRequest, ProcessStopClass,
};
pub use driver::{
    DeviceWorkerFamily, GuestOwnerIdentitySource, ProcessDriverArgs, device_worker_family,
    device_worker_vm, process_family_descriptors, process_spec_decoder, resolve_guest_owner_uid,
    resource_uid_from_bytes,
};
pub use effects::{ProcessDriverEffects, ProviderAdoption, ProviderLiveness};
pub use execution::{ExecutionMode, execution_target_allowed};
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
