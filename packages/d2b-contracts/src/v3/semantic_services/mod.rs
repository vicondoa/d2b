//! The shared semantic Service and Binding contract bases.
//!
//! This module tree will own the common Service/Binding base spec, status,
//! and schema contract for each frozen semantic pair named by
//! `ADR-046-provider-model-and-packaging` work item `ADR046-provider-004`.
//! Each submodule owns one semantic family; the exact type names, schema
//! identifiers, versions, and fingerprints are settled by that work item and
//! are not stated here.
//!
//! The eight qualified semantic ResourceTypes these bases describe are
//! installed from signed Provider schemas. They are deliberately absent from
//! the standard ResourceType registry, which is closed.
//!
//! Nothing here is implemented yet. Nothing here is a design statement.

pub mod audio;
pub mod security_key;
pub mod telemetry;
pub mod usb;

/// Marks this module as scaffolding that no work item has filled yet.
///
/// The workspace capability-surface scan renders rustdoc for every member and
/// fails closed when a module advertises no public item, and a module
/// declaration alone does not count. This constant exists only to satisfy that
/// gate and carries no design intent: the slice that implements
/// `ADR046-provider-004` should delete it rather than build on it.
pub const UNIMPLEMENTED_SCAFFOLD: () = ();
