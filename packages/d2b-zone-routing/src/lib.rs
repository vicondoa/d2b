//! Zone routing engine, entrypoint resolver, and routing service.
//!
//! Scaffolding for ADR-046 Wave 2. Every module here is filled by the
//! work item named in its own doc comment.

pub mod engine;
pub mod resolver;
pub mod service;

/// Marks this crate as scaffolding that no work item has filled yet.
///
/// The workspace capability-surface scan renders rustdoc for every member and
/// fails closed when a crate advertises no public item. A module declaration
/// is not itself an advertised item, so a crate of empty modules still trips
/// that gate. This constant exists only to satisfy it and carries no design
/// intent: the slices implementing the modules above should delete it.
pub const UNIMPLEMENTED_SCAFFOLD: () = ();
