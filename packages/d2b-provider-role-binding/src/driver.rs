//! The RoleBinding resource driver: the v3 `ResourceDriver` conversion of the
//! Core baseline reconciler for `RoleBinding` rows.
//!
//! `RoleBinding` binds a role to its declared subjects: the driver converges
//! it as metadata once its desired state is admitted, and the row carries the
//! role reference and the typed subject, resource, zone, and execution scopes
//! the authorization path resolves. That row is the canonical
//! `RoleBindingSpec` in `d2b-contracts-zone-session`; this crate does not
//! restate it, because a second copy of the grant shape here could drift
//! from the contract the graph publishes and then authorize a shape no
//! admitted row carries.
//!
//! A binding confers no authority of its own. It is evaluated against the
//! prior accepted graph, so a candidate `RoleBinding` is never in the state
//! that decides whether it may be created: a role that would grant `create`
//! on `RoleBinding` does not exist until that binding has already been
//! accepted. Every remaining facet of a binding - its resource, zone, and
//! execution scopes, and its scope narrowing - can only remove authority the
//! `Role` already granted, never add any.
//!
//! The conversion itself is the shared declaration-only metadata driver of
//! `d2b_resource_runtime::metadata`: the type realizes no target-local state,
//! so the shared driver's validate, recover, reconcile, finalize, and delete
//! verbs are the whole conversion, and the only fact this crate owns is the
//! type's identity.

use d2b_resource_types::metadata_descriptor;
use d2b_resource_types::{DriverDescriptor, WellKnownType};

/// The `RoleBinding` type's driver declaration.
pub fn role_binding_descriptor() -> DriverDescriptor {
    metadata_descriptor(WellKnownType::ROLE_BINDING)
}
