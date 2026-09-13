//! The User provider crate: the `User` resource type's driver, its spec
//! decoder, and its driver declaration.
//!
//! The crate owns the User type's complete resource knowledge: the closed
//! User base contract, the driver's validate, recover, reconcile, finalize,
//! and delete verbs, and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! Everything the driver needs from outside arrives through the driver effect
//! port ([`UserDriverEffects`]): local NSS discovery with its opaque identity
//! digest, which the daemon realizes over the preserved `UserReconciler`. The
//! production implementation lives in the daemon behind that port, so this
//! crate depends on no daemon runtime.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    UserDriver, UserDriverEffects, UserDriverError, UserDriverFactory, UserDriverStatus,
    user_descriptor, user_spec_decoder,
};
