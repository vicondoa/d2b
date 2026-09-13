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
pub mod worker_launch;

pub use backend::{
    BackendLaunch, BackendObservation, ProcessEffectBackend, ProcessEffectError,
    ProcessLaunchRequest, ProcessRequest, ProcessStopClass,
};
pub use driver::{
    CommittedProviderIdentitySource, DeviceWorkerFamily, GuestOwnerIdentitySource,
    ProcessDriverArgs, device_worker_family, device_worker_vm, process_family_descriptors,
    process_spec_decoder, resolve_guest_owner_uid, resource_uid_from_bytes,
};
pub use effects::{ProcessDriverEffects, ProviderAdoption, ProviderLiveness};
pub use execution::execution_target_allowed;
pub use identity::{ProcessFamilySpec, ProcessResourceIdentity, decode_metadata_owner_ref};
pub use launch_identity::{LaunchRow, resolve_launch_identity};
pub use worker_launch::{
    DeviceWorkerLaunch, GpuWorkerParams, ServingWorkerLaunch, ServingWorkerRoot, SwtpmFlushParams,
    SwtpmWorkerParams, VideoWorkerParams,
};

pub use d2b_process_conformance::{
    AdoptionCandidate, CompiledDigests, ConfigurationDigest, IdentityBinding, LaunchTicket,
    LaunchedProcess, ObservedIdentity, OperationBinding, PidfdEvidence, ProcessConformanceError,
    ProcessIdentityDigest, ProcessLaunchEffectPort, StopClass, WaitReapOwner,
};
