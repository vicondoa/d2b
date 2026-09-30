//! The ResourceImport resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `ResourceImport` rows.
//!
//! `ResourceImport` names one remote export through a local ZoneLink and
//! materializes the admitted qualified semantic Service projection that export
//! carries. The driver converges the row as metadata once its desired state is
//! admitted, and the projection's own realization belongs to the semantic
//! family's driver.
//!
//! What this conversion produces locally is a projection and a lease, never a
//! copy of what the projection stands for. The primitive resources the owner
//! Zone's Service backs stay in the owner Zone, realized there by ordinary
//! bindings under that Zone's authority, and an importing consumer reaches
//! them through the projection's declared methods - never by attaching the
//! backing resource to something local. That is why the row's contract carries
//! only a local ZoneLink reference and an opaque export key, and why this
//! crate's conversion has no output that names a backing resource.
//!
//! The conversion is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The ResourceTypes this conversion must never materialize locally.
///
/// An import is a lease over the owner Zone's admitted semantic service, so a
/// locally owned storage, device, network, endpoint, or credential source is
/// not something it can produce. Binding rows are absent for the same reason:
/// attaching one of these to a local consumer is a local authority decision
/// this conversion has no standing to make. The qualified semantic Service
/// projection and its lease are the whole of what remains.
pub const IMPORT_FORBIDDEN_LOCAL_TYPES: [WellKnownType; 5] = [
    WellKnownType::VOLUME,
    WellKnownType::DEVICE,
    WellKnownType::NETWORK,
    WellKnownType::ENDPOINT,
    WellKnownType::CREDENTIAL,
];

/// The `ResourceImport` type's driver declaration.
pub fn resource_import_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::RESOURCE_IMPORT)
}
