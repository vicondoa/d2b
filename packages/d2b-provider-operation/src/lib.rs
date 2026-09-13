//! The Operation provider crate: the Operation resource type's driver, its spec decoder,
//! and its driver declaration.
//!
//! The crate owns the Operation type's complete resource knowledge: the driver's
//! validate, recover, reconcile, finalize, and delete verbs and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! `Operation` is a broker operation row: payload schema, authority profile, audit facet, and the handler reference the generic envelope dispatches. This unit declares the type and ships the driver shell; the type's rows materialize in the policy-rows unit.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    OPERATION_TYPE_NAME, OperationDriver, OperationDriverFactory, operation_descriptor,
    operation_spec_decoder,
};
