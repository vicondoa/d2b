//! The SeccompProfile provider crate: the SeccompProfile resource type's driver declaration.
//!
//! The crate owns the SeccompProfile type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `SeccompProfile` declares the device-node binds and the posture a role
//! references. The type is declared here and its rows materialize in the
//! policy-rows unit.

#![deny(missing_docs)]

mod driver;

/// The Seccomp Profile ResourceType spec and status shapes owned by this crate.
mod seccomp_profile;

pub use driver::seccomp_profile_descriptor;
pub use seccomp_profile::{
    DeviceBind, DeviceNodeKind, DeviceNodePath, SECCOMP_PROFILE_RESOURCE_TYPE,
    SeccompCgroups, SeccompDeviceAccess, SeccompNamespaces, SeccompProfileSpec,
};
