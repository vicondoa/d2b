//! Zone routing engine, entrypoint resolver, and routing service.
//!
//! Scaffolding for ADR-046 Wave 2. Every module here is filled by the
//! work item named in its own doc comment.

pub mod engine;
mod realm_entrypoint;
pub mod resolver;
pub mod router;
pub mod service;

pub use realm_entrypoint::{
    DispatchTarget, RealmEntrypoint, RealmEntrypointTable, ResolveError,
};
