//! The EmergencyPolicy provider crate: the EmergencyPolicy resource type's driver, its spec decoder,
//! and its driver declaration.
//!
//! The crate owns the EmergencyPolicy type's complete resource knowledge: the driver's
//! validate, recover, reconcile, finalize, and delete verbs and the
//! [`DriverDescriptor`](d2b_resource_types::DriverDescriptor) the plane
//! registers the type by.
//!
//! `EmergencyPolicy` carries a zone's emergency posture. The driver converges it as metadata once its desired state is admitted; the authority class the policy scopes is arbitrated by the quota crate's authority index.

#![deny(missing_docs)]

mod driver;

pub use driver::{
    EMERGENCY_POLICY_TYPE_NAME, EmergencyPolicyDriver, EmergencyPolicyDriverFactory, emergency_policy_descriptor,
    emergency_policy_spec_decoder,
};
