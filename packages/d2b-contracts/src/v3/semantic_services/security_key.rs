//! The shared security-key semantic Service and Binding base contract.
//!
//! This module will own the common base spec, status, and schema contract for
//! the frozen security-key Service and Binding pair named by
//! `ADR-046-provider-model-and-packaging` work item `ADR046-provider-004`.
//!
//! Nothing here is implemented yet. Nothing here is a design statement, and no
//! consumer should read a shape from this file.

/// Marks this module as scaffolding that no work item has filled yet.
///
/// The workspace capability-surface scan renders rustdoc for every member and
/// fails closed when a module advertises no public item. This constant exists
/// only to satisfy that gate and carries no design intent: the slice that
/// implements `ADR046-provider-004` should delete it rather than build on it.
pub const UNIMPLEMENTED_SCAFFOLD: () = ();
