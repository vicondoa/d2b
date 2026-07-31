//! The supervisor Provider.
//!
//! This crate will own the supervising Provider that owns the production process-launch effect adapter, named by work item
//! `ADR046-process-001`.
//!
//! Nothing here is implemented yet. Nothing here is a design statement, and no
//! consumer should read a shape from this file.

#![deny(missing_docs)]

/// Marks this crate as scaffolding that no work item has filled yet.
///
/// The workspace capability-surface scan renders rustdoc for every member and
/// fails closed when a crate advertises no public item, so an empty scaffold
/// would break that gate for the whole workspace. This constant exists only to
/// satisfy it and carries no design intent: the slice that implements
/// `ADR046-process-001` should delete it rather than build on it.
pub const UNIMPLEMENTED_SCAFFOLD: () = ();
