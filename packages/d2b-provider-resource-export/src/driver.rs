//! The ResourceExport resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `ResourceExport` rows.
//!
//! `ResourceExport` advertises that a locally owned semantic Service is
//! available to another Zone. The driver converges it as metadata once its
//! desired state is admitted, and the export's own realization belongs to the
//! exported resource's family.
//!
//! What this conversion may name is stated here rather than left to a caller's
//! naming discipline. An export never names a primitive source: the `Volume`,
//! `Device`, `Network`, `Endpoint`, and `Credential` rows that a Service backs
//! stay in the owner Zone and are realized there by ordinary bindings under
//! that Zone's authority. Cross-Zone use of one of them requires the owner to
//! export a semantic Service, and the importing Zone then holds a lease over
//! its projection instead of a reference to the backing resource.
//!
//! The conversion is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The ResourceTypes an export may name as its subject.
///
/// The set is empty, and that emptiness is the rule rather than an omission.
/// A `ResourceExport` advertises a qualified semantic Service the owner Zone
/// owns; a primitive source never becomes the subject of an export, so this
/// conversion has no vocabulary with which to name one. An imported
/// projection is not a subject either - a lease over a remote Service is never
/// re-advertised - and that rule is enforced against the stored row, not
/// against this list.
pub const EXPORT_SUBJECT_TYPES: [WellKnownType; 0] = [];

/// The `ResourceExport` type's driver declaration.
pub fn resource_export_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::RESOURCE_EXPORT)
}
