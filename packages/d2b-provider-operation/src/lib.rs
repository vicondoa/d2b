//! The Operation provider crate: the Operation resource type's driver declaration.
//!
//! The crate owns the Operation type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `Operation` is a broker operation row: it declares the payload schema, the
//! authority profile, the audit facet, and the handler reference the generic
//! envelope dispatches. The type is declared here and its rows materialize in
//! the policy-rows unit.

#![deny(missing_docs)]

mod driver;

/// The Operation ResourceType spec and status shapes owned by this crate.
pub mod operation;

pub use driver::operation_descriptor;
pub use operation::*;
