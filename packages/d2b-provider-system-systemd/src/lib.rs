//! systemd System/Process provider.
//!
//! TODO: filled by ADR046-primitives-002.

/// Marks this crate as scaffolding that no work item has filled yet.
///
/// The workspace capability-surface scan renders rustdoc for every member and
/// fails closed when a crate advertises no public item, so an empty scaffold
/// would break that gate for the whole workspace. This constant exists only to
/// satisfy it and carries no design intent: the slice that implements
/// `ADR046-primitives-002` should delete it rather than build on it.
pub const UNIMPLEMENTED_SCAFFOLD: () = ();
