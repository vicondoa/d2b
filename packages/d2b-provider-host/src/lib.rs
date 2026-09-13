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

pub use driver::{
    HostDriver, HostDriverEffects, HostDriverError, HostDriverFactory, HostDriverStatus,
    host_descriptor, host_spec_decoder,
};
