//! Fixed core-controller handlers and pure reconciliation policy.
//!
//! The store-routing half of this crate (the registered-API adapter, the
//! configuration/cleanup generations, the watch/hint admission queue, and the
//! durable store metadata) was deleted with the persistent-database control
//! model. What remains are the domain modules the converted drivers and the
//! daemon import.

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
pub mod providers;
pub mod rbac;
pub mod zone_links;
pub mod zone_status;
pub mod zonelink;

pub use binding_children::{
    BindingChildMaterializationError, BindingChildReconciler, BindingChildResource,
    materialize_child_create_payload, observed_child_from_resource, semantic_child_digest,
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
pub use providers::{CoreReconcileError, fixed_system_core_handlers_ready, provider_observation};
pub use zone_status::{SystemCoreStatusEmitter, ZoneRuntimeMetadata, ZoneStatusInput};
