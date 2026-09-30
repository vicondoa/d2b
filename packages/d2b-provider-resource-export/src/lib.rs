//! The ResourceExport provider crate: the ResourceExport resource type's driver declaration.
//!
//! The crate owns the ResourceExport type's identity, the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by, and the closed set of ResourceTypes an export may
//! name as its subject. The conversion itself - validate, recover, reconcile,
//! finalize, and delete - is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`, so this crate cannot diverge from its
//! siblings on it.
//!
//! `ResourceExport` advertises that a locally owned semantic Service is
//! available to another Zone. It never advertises a primitive source: the
//! backing resources a Service realizes stay in the owner Zone and are
//! realized there by ordinary bindings under that Zone's authority.

#![deny(missing_docs)]

mod driver;

pub use driver::{EXPORT_SUBJECT_TYPES, resource_export_descriptor};
