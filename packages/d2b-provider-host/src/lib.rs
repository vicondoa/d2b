//! The Host provider crate: the `Host` resource type's driver, its spec
//! decoder, and its driver declaration.
//!
//! The crate owns the Host type's complete resource knowledge: the closed
//! Host base contract, the admission fence that pins `spec.providerRef` to
//! `Provider/system-core`, the driver's validate, recover, reconcile,
//! finalize, and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! Everything the driver needs from outside arrives through the driver effect
//! port ([`HostDriverEffects`]): the bounded capability/platform/proc probe
//! with its degraded fallback, which the daemon realizes over the preserved
//! `HostReconciler`. The production implementation lives in the daemon behind
//! that port, so this crate depends on no daemon runtime.

#![deny(missing_docs)]

mod driver;

// The scripted HostDriverEffects recording double. Needed both by
// external crates (d2bd's plane tests, which opt in via the `test-support`
// feature) and by this crate's own tests. Gating on
// `any(test, feature = "test-support")` makes it available automatically when
// compiling this crate's tests, so `cargo test -p d2b-provider-host` works
// without remembering `--features test-support`.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use driver::{
    HostDriver, HostDriverEffects, HostDriverError, HostDriverFactory, HostDriverStatus,
    host_descriptor, host_spec_decoder,
};
