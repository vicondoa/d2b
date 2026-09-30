//! The SeccompProfile resource driver: the v3 `ResourceDriver` conversion of
//! the Core baseline reconciler for `SeccompProfile` rows.
//!
//! `SeccompProfile` is a syscall filter and nothing else. The canonical spec
//! in `d2b-contracts-resource` has exactly one field, the filter, and its wire
//! mirror denies unknown fields, so a committed row that still carries a
//! namespace set, a cgroup set, a device-node bind, or a mount does not
//! decode into this type at all. That is the difference this type's contract
//! exists to keep: a profile that also granted access would hand a workload
//! reach through a name some provider or role happens to select.
//!
//! The conversion itself is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The `SeccompProfile` type's driver declaration.
pub fn seccomp_profile_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::SECCOMP_PROFILE)
}
