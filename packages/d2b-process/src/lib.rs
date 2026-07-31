//! Neutral process launch and supervision primitives.
//!
//! Process Providers receive only the provider-neutral conformance types.
//! Core-owned effect adapters use [`ProcessEffectBackend`] to reach the local
//! broker or service manager without exposing a path, numeric identity, process
//! identifier, unit name, cgroup, argument vector, environment, or descriptor
//! to Provider code.

#![deny(missing_docs)]

mod backend;

pub use backend::{
    BackendLaunch, BackendObservation, ProcessEffectBackend, ProcessEffectError, ProcessRequest,
    ProcessStopClass,
};

pub use d2b_process_conformance::{
    AdoptionCandidate, CompiledDigests, ConfigurationDigest, IdentityBinding, LaunchTicket,
    LaunchedProcess, ObservedIdentity, OperationBinding, PidfdEvidence, ProcessConformanceError,
    ProcessIdentityDigest, ProcessLaunchEffectPort, StopClass, WaitReapOwner,
};
