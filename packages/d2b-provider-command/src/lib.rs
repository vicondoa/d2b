//! The Command provider crate: the Command resource type's driver, its spec decoder,
//! and its driver declaration.
//!
//! The crate owns the Command type's complete resource knowledge: the driver's
//! validate, recover, reconcile, finalize, and delete verbs and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! `Command` is a declared launch shape: fixed at startup, family-declared, it materializes the spawn operation the process controller serves. This unit declares the type and ships the driver shell; the type's rows materialize in the policy-rows unit, which owns the seed and the placeholder validation.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    COMMAND_TYPE_NAME, CommandDriver, CommandDriverFactory, command_descriptor,
    command_spec_decoder,
};
