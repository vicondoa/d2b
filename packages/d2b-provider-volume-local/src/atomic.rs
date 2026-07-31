//! Scaffold for the atomic write and replace sequence.
//!
//! This module is the file reserved for `ADR-046` work item
//! `ADR046-pstate-003`. It is a scaffold: it holds no behaviour and states
//! no contract.
//!
//! Nothing here is implemented yet. Nothing here is a design statement, and no
//! consumer should read a shape from this file.

/// Marks this module as scaffolding that no work item has filled yet.
///
/// The workspace capability-surface scan renders rustdoc for every member and
/// fails closed when a module advertises no public item. This constant exists
/// only to satisfy that gate and carries no design intent: the slice that
/// fills this file should delete it rather than build on it.
pub const UNIMPLEMENTED_SCAFFOLD: () = ();
