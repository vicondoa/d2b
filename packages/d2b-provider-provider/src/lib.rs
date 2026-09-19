//! The Provider provider crate: the `Provider` resource type's driver, its
//! spec decoder, its effect port, and its driver declaration.
//!
//! The crate owns the `Provider` type's complete resource knowledge: the
//! readiness observation the driver applies to a Provider's owned controller
//! `Process` rows and state `Volume` rows, the phases the pure provider policy
//! projects, the fixed provider identities the two internally hosted providers
//! observe, and the [`DriverDescriptor`](d2b_resource_types::DriverDescriptor)
//! the plane registers the type by.
//!
//! Everything the driver needs from outside arrives through the driver effect
//! port ([`ProviderDriverEffects`]): the live controller-session evidence the
//! manager cannot serve, because a converted row's status is in-memory only.
//! The production implementation lives in the daemon behind that port, so this
//! crate depends on no daemon type.


mod driver;
pub mod providers;

// The scripted ProviderDriverEffects recording double. Needed both by
// external crates (d2bd's plane tests, which opt in via the `test-support`
// feature) and by this crate's own tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically to
// this crate's unit tests. Integration tests that need it declare
// `required-features`, so run those with `--features test-support` (or let
// the Bazel `*_test_support` target compile them).
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use driver::{
    FailClosedProviderDriverEffects, PROVIDER_TYPE_NAME, ProviderDriver, ProviderDriverArgs,
    ProviderDriverEffects, ProviderDriverFactory, ProviderDriverStatus, SYSTEM_CORE_HOST_REF,
    SYSTEM_CORE_PROVIDER_REF, SYSTEM_MINIJAIL_PROVIDER_REF, provider_descriptor,
    provider_spec_decoder,
};
