//! The Operation provider crate: the Operation resource type's driver declaration.
//!
//! The crate owns the Operation type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `Operation` is the single externally callable contract: its payload and
//! result schemas, the authority, audit, descriptor-carriage, and bounds
//! facets, and the trusted implementation that answers it. Every one of those
//! facets is the canonical contract in `d2b-contracts-resource`, and this
//! module re-exports them rather than restating them.
//!
//! What it still holds locally is the pre-cutover row a committed `Command`
//! materializes, carrying an `ownerRef` and an inherited wire discriminant.
//! The canonical contract has no such row, so it has no successor; the cutover
//! deletes it together with the foundation seed's materialization.

#![deny(missing_docs)]

mod driver;

/// The canonical `Operation` contract, re-exported, and the one retired row
/// shape that still has a caller.
pub mod operation;

pub use driver::operation_descriptor;
