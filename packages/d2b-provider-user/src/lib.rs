//! The User provider crate: the `User` resource type's driver, its spec
//! decoder, its driver declaration, and the implementation of the family's
//! driver effects.
//!
//! The crate owns the User type's complete resource knowledge: the closed
//! User base contract, the driver's validate, recover, reconcile, finalize,
//! and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! The family's driver effects (U5) are implemented by this crate itself
//! ([`crate::effects_service`]): the bounded local-account probe
//! ([`crate::probe`]) runs inside the crate over the preserved
//! `UserReconciler`, and the family reads no daemon state of its own, so
//! the daemon-supplied facet set (the declared
//! [`crate::facets::UserEffectFacets`]) is empty today. The daemon hosts
//! the family's declared effects service
//! ([`crate::effects_service::USER_EFFECTS_SERVICE`]) per zone from the
//! family's registered factory; no externally built port appears at any
//! construction site (R2).

#![deny(missing_docs)]

mod driver;

mod effects_service;
mod facets;
mod probe;

// The scripted UserDriverEffects recording double. Needed both by
// external crates (d2bd's plane tests, which opt in via the `test-support`
// feature) and by this crate's own tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically to
// this crate's unit tests. Integration tests that need it declare
// `required-features`, so run those with `--features test-support` (or let
// the Bazel `*_test_support` target compile them).
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use driver::{
    UserDriver, UserDriverEffects, UserDriverError, UserDriverFactory, UserDriverStatus,
    user_descriptor, user_spec_decoder,
};
pub use effects_service::{
    USER_EFFECTS_SERVICE, UserEffectsService, UserEffectsServiceFactory,
};
pub use facets::UserEffectFacets;