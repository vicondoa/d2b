//! The Volume state contract.
//!
//! This module will own the Volume state surface, named by work item
//! `ADR046-pstate-001`.
//!
//! No part of that contract is implemented yet. Nothing here is a design
//! statement, and no consumer should read a shape from this file.

/// Marks this module as scaffolding that no work item has filled yet.
///
/// The workspace capability-surface scan renders rustdoc for every member and
/// fails closed when a module advertises no public item, so an empty scaffold
/// would break that gate for the whole workspace. This constant exists only to
/// satisfy it and carries no design intent: the slice that implements
/// `ADR046-pstate-001` should delete it rather than build on it.
pub const UNIMPLEMENTED_SCAFFOLD: () = ();
