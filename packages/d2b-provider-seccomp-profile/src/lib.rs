//! The SeccompProfile provider crate: the SeccompProfile resource type's driver, its spec decoder,
//! and its driver declaration.
//!
//! The crate owns the SeccompProfile type's complete resource knowledge: the driver's
//! validate, recover, reconcile, finalize, and delete verbs and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! `SeccompProfile` declares the device-node binds and posture a role references. This unit declares the type and ships the driver shell; the type's rows materialize in the policy-rows unit.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    SECCOMP_PROFILE_TYPE_NAME, SeccompProfileDriver, SeccompProfileDriverFactory, seccomp_profile_descriptor,
    seccomp_profile_spec_decoder,
};
