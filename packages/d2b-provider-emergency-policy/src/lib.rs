//! The EmergencyPolicy provider crate: the EmergencyPolicy resource type's driver declaration.
//!
//! The crate owns the EmergencyPolicy type's identity and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by. The conversion itself - validate, recover,
//! reconcile, finalize, and delete - is the shared declaration-only metadata
//! driver of `d2b_resource_runtime::metadata`, so this crate cannot diverge
//! from its siblings on it.
//!
//! `EmergencyPolicy` carries a zone's emergency posture: the driver converges
//! it as metadata once its desired state is admitted, and the authority class
//! the policy scopes is arbitrated by the quota crate's authority index.

#![deny(missing_docs)]

mod driver;

pub use driver::emergency_policy_descriptor;
