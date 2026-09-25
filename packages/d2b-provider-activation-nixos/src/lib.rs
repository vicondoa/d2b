//! Activation-NixOS Provider lifecycle, typed effect boundaries, and the
//! `NixosGeneration` resource driver.
//!
//! This crate is the activation family's home: the pure activation policy,
//! the driver for the `NixosGeneration` resource type with its spec
//! decoder, factory, and declaration, and the implementation of the
//! family's driver effects. The effects are served from this crate itself
//! over the daemon-supplied facet set (see [`crate::effects_service`] and
//! [`crate::facets`]), so the daemon composes the family's declared effects
//! service from the generated registration table and no daemon module
//! implements the family's effect traits any more.

#![deny(missing_docs)]

pub mod controller;
pub mod driver;
pub mod effects_service;
pub mod facets;
pub mod vocabulary;

// `test_support` is needed both by external crates (which opt in via the
// `test-support` feature) and by this crate's OWN tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically when
// compiling this crate's tests, so `cargo test -p d2b-provider-activation-nixos`
// works without anyone having to remember `--features test-support`.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use controller::{
    ActivationApplicationVerifier, ActivationCaller, ActivationController, ActivationError,
    ActivationTrust, ActivationTrustExpectation, ActivationVerificationError, CallerRole,
    FailClosedActivationVerifier, GenerationObservation, GenerationPhase, RunnerRequest,
    RunnerResult, SignedActivationApplicationVerifier, TrustStatus, activation_runner_name,
    activation_runner_ref, activation_runner_spec,
};
pub use driver::{
    ACTIVATION_CREATIONS, ACTIVATION_RUNNER_CREATION, ACTIVATION_TYPE_NAME,
    ActivationDriverArgs, ActivationDriverEffects, ActivationDriverError,
    ActivationDriverFactory, ActivationDriverStatus, HostHandoffResult, RUNNER_PROVIDER_REF,
    RUNNER_TYPE_NAME, activation_descriptor, activation_spec_decoder,
};
pub use effects_service::{
    ACTIVATION_EFFECTS_SERVICE, ActivationEffectsService, ActivationEffectsServiceFactory,
};
pub use facets::{ActivationBrokerDispatch, ActivationEffectFacets};
pub use vocabulary::{
    ACTIVATION_RUNNER_STEPS, ActivationRunnerStep, declared_runner_step, is_declared_runner_step,
};