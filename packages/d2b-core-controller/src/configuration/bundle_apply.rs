//! The bundle application and diff pass of the configuration service.
//!
//! This module is the file reserved for the bundle-application half of
//! `ADR-046` work item `ADR046-network-008`: bundle application, the diff, and
//! prior-bundle retention. It is a scaffold. The configuration service and
//! every item it has landed so far live in [`super`], unmoved, and nothing was
//! relocated here, because relocating a type whose private fields the service
//! reads would be a rewrite rather than a move.
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
