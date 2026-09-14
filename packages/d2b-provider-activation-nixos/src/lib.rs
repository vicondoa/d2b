//! Activation-NixOS Provider lifecycle, typed effect boundaries, and the
//! `NixosGeneration` resource driver.
//!
//! This crate is the activation family's home: the pure activation policy
//! and the driver for the `NixosGeneration` resource type with its spec
//! decoder, factory, and declaration. The production effect implementation
//! stays in the daemon behind the driver's effect port, so the family owns
//! the seam and the daemon owns the broker dispatch.

#![deny(missing_docs)]

pub mod controller;
pub mod driver;

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
    ActivationDriver, ActivationDriverArgs, ActivationDriverEffects, ActivationDriverError,
    ActivationDriverFactory, ActivationDriverStatus, HostHandoffResult, RUNNER_PROVIDER_REF,
    RUNNER_TYPE_NAME, activation_descriptor, activation_spec_decoder,
};
