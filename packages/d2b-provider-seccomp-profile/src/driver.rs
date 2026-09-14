//! The SeccompProfile resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `SeccompProfile` rows.
//!
//! `SeccompProfile` declares the device-node binds and the posture a role
//! references. The type is declared here and its rows materialize in the
//! policy-rows unit.
//!
//! The conversion is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The `SeccompProfile` type's driver declaration.
pub fn seccomp_profile_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::SECCOMP_PROFILE)
}
