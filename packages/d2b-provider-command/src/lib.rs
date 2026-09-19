//! The Command provider crate: the Command resource type's driver declaration.
//!
//! The crate owns the Command type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `Command` is a declared launch shape: fixed at startup, family-declared, it
//! materializes the spawn operation the process controller serves. The type is
//! declared here and its rows materialize in the policy-rows unit.

#![deny(missing_docs)]

mod driver;

pub use driver::command_descriptor;

/// The Command ResourceType spec and status shapes owned by this crate.
pub mod command;
pub use command::*;
