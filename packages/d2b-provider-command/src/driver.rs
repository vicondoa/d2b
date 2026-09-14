//! The Command resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `Command` rows.
//!
//! `Command` is a declared launch shape: fixed at startup, family-declared, it
//! materializes the spawn operation the process controller serves. The type is
//! declared here and its rows materialize in the policy-rows unit.
//!
//! The conversion is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The `Command` type's driver declaration.
pub fn command_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::COMMAND)
}
