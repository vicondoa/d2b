//! Non-resource controller-session machinery: the assignment transport, the
//! per-Zone coordinator, the legacy-state migration receipts, the fixed
//! handler catalog, and the generic owner-child reconciliation Core owns for
//! every declaring family.
//!
//! The store-routing half of this crate (the registered-API adapter, the
//! configuration/cleanup generations, the watch/hint admission queue, and the
//! durable store metadata) was deleted with the persistent-database control
//! model. The resource-domain modules left with the types they belong to:
//! `zone_status` moved to `d2b-provider-zone`, `zone_links`/`zonelink` to
//! `d2b-provider-zone-link`, `rbac` to `d2b-provider-role`, and `providers`
//! to `d2b-provider-provider`. The Host-global authority index and its
//! durable operation adapter stay here with the coordinator they serve: they
//! arbitrate every scarce-resource class the session admits, not one type's
//! rows.

// `main.rs` is a library module here, not a binary crate root; the crate turns
// off binary auto-discovery so cargo does not claim it as one. The lint that
// warns about the name is emitted while modules are collected, so it can only
// be allowed at the crate root.
#![allow(special_module_name)]

pub mod authority;
pub mod authority_persistence;
pub mod binding_children;
pub mod controller_assignment;
pub mod controllers;
pub mod coordinator;
pub mod main;
pub mod migration;
pub mod owner_reconcile;

pub use binding_children::{
    BindingChildMaterializationError, BindingChildResource, materialize_child_create_payload,
    observed_child_from_resource, semantic_child_digest,
};
pub use controller_assignment::{
    AssignmentEpoch, AssignmentError, AssignmentGrantError, AssignmentIdentity, AssignmentPhase,
    AssignmentRequest, AssignmentScope, AssignmentTarget, AssignmentTransportError, AssignmentVerb,
    CONTROLLER_ASSIGNMENT_STREAM_CREDIT, CONTROLLER_ASSIGNMENT_STREAM_ID,
    ControllerAssignmentExpectation, ControllerAssignmentGrant, ControllerAssignmentGrantStore,
    ControllerAssignmentRegistry, ControllerRoleContract, ControllerSessionBinding,
    GrantDisposition, MAX_ASSIGNMENT_GRANT_RESOURCE_TYPES, MAX_ASSIGNMENT_GRANT_SCOPES,
    MAX_ASSIGNMENT_GRANT_VERBS, MAX_CONTROLLER_ASSIGNMENT_GRANT_BYTES,
    MAX_SCOPED_COMMIT_TRANSPORT_BYTES, OwnerChildScope, ResourceClientLease, ScopedCommitTransport,
    ScopedResourceFilter, ScopedResourceMutation, ScopedResourceQuery, ScopedResourceScope,
};
pub use controllers::{
    AggregateHealth, CoreHandlerKind, CoreHandlerRegistry, CurrencyAggregation,
    CurrencyAggregationError, HandlerOutcome, HandlerPhase, HandlerStatus,
};
pub use d2b_controller_toolkit::{DependencySnapshot, ResourceKey, ResourceSnapshot};
pub use owner_reconcile::{
    DesiredChild, MAX_OWNER_CHILD_BATCH, MAX_OWNER_CHILD_DEPENDENCIES, ObservedChild,
    OwnedChildIntent, OwnedChildKind, OwnerBatchRecovery, OwnerBatchResult, OwnerChildBatch,
    OwnerChildIdentity, OwnerGraph, OwnerGraphError, OwnerIndex, OwnerLimits, OwnerMutation,
    OwnerReconcileError, OwnerReconcilePlan, OwnerTrigger, ProcessSchedulingClass, TeardownPlan,
};
