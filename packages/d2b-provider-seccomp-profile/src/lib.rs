//! The SeccompProfile provider crate: the SeccompProfile resource type's driver declaration.
//!
//! The crate owns the SeccompProfile type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `SeccompProfile` is a syscall filter and nothing else. The canonical spec
//! in `d2b-contracts-resource` has exactly one field, the filter, and its
//! wire mirror denies unknown fields, so a committed row that still carries a
//! namespace set, a cgroup set, a device-node bind, or a mount does not
//! decode into this type at all: confinement is an `ExecutionPolicy`'s
//! business, and access is admitted through the typed binding relationships.
//!
//! `seccomp_profile` still holds the pre-cutover posture row the foundation
//! seed publishes for the system zone. That shape has no successor in the
//! canonical contract; the cutover deletes it together with the seed's use of
//! it.

#![deny(missing_docs)]

mod driver;

/// The pre-cutover posture row shape the foundation seed still publishes.
mod seccomp_profile;

pub use driver::seccomp_profile_descriptor;
pub use seccomp_profile::{
    DeviceBind, DeviceNodeKind, DeviceNodePath, MAX_DEVICE_NODE_PATH_BYTES,
    MAX_SECCOMP_DEVICE_BINDS, MAX_SECCOMP_SYSCALLS, SECCOMP_PROFILE_RESOURCE_TYPE, SeccompCgroups,
    SeccompDeviceAccess, SeccompNamespaces, SeccompProfileContractError, SeccompProfileSpec,
};
