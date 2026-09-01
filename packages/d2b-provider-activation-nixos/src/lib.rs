//! Activation-NixOS Provider lifecycle and typed effect boundaries.

#![deny(missing_docs)]

pub mod controller;
pub mod diagnostics;
pub mod manifest;
pub mod runner;

pub use controller::{
    ActivationCaller, ActivationController, ActivationError, CallerRole, GenerationObservation,
    GenerationPhase, RetentionPlan, RunnerRequest, RunnerResult, activation_runner_name,
    activation_runner_ref, activation_runner_spec,
};
pub use manifest::ActivationManifest;
pub use runner::{
    ActivationHelper, ActivationRunner, ActivationRunnerError, ActivationRunnerRequest,
    ActivationRunnerResult, RunnerOutcomeCode,
};
