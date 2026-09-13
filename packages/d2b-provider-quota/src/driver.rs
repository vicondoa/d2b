//! The Quota resource driver: the v3 `ResourceDriver` conversion of the Core
//! baseline reconciler for `Quota` rows.
//!
//! `Quota` is a scarce-resource scope claim: the driver converges it as
//! metadata once its desired state is admitted. The Host-global authority
//! index that arbitrates the claim's class beside every other scarce class the
//! session admits is controller-session machinery and stays in
//! `d2b-core-controller`.
//!
//! The conversion is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The `Quota` type's driver declaration.
pub fn quota_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::QUOTA)
}
