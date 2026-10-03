//! The ResourceImport provider crate: the ResourceImport resource type's driver declaration.
//!
//! The crate owns the ResourceImport type's identity, the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by, and the closed set of ResourceTypes an import may
//! never materialize locally. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `ResourceImport` names one remote export through a local ZoneLink and
//! materializes the admitted qualified semantic Service projection that
//! export carries, under the exporting Zone's consumer policy, capability
//! ceiling, and lease. It is not a locally owned primitive source: the backing
//! resources the owner Zone's Service realizes stay in that Zone.

#![deny(missing_docs)]

mod driver;

pub use driver::{IMPORT_FORBIDDEN_LOCAL_TYPES, resource_import_descriptor};
