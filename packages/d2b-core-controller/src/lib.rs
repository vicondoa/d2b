//! Fixed core-controller handlers and pure reconciliation policy.

// `main.rs` is a library module here, not a binary crate root; the crate turns
// off binary auto-discovery so cargo does not claim it as one. The lint that
// warns about the name is emitted while modules are collected, so it can only
// be allowed at the crate root.
#![allow(special_module_name)]

pub mod api_catalog;
pub mod audit;
pub mod authority;
pub mod authz;
pub mod authz_audit;
pub mod budgets;
pub mod cleanup;
pub mod configuration;
pub mod controllers;
pub mod coordinator;
pub mod dependencies;
pub mod export_import;
pub mod export_import_projection;
pub mod hints;
pub mod main;
pub mod metrics;
pub mod optional_state_admission;
pub mod owner_reconcile;
pub mod ownership;
pub mod provider_effects;
pub mod providers;
pub mod rbac;
pub mod resource_store;
pub mod runtime;
pub mod store;
pub mod tracing;
pub mod user_session_authority;
pub mod watches;
pub mod zone_links;
pub mod zonelink;

pub use controllers::{
    AggregateHealth, CoreHandlerKind, CoreHandlerRegistry, CurrencyAggregation,
    CurrencyAggregationError, HandlerOutcome, HandlerPhase, HandlerStatus,
};
pub use dependencies::{DependencyError, DependencyIndex, DependencyTrigger, UpgradeOrder};
pub use export_import::{
    AdmittedExport, AdmittedImport, ExportImportError, ProjectionServiceIdentity,
    admit_binding_target, admit_export, admit_factory_pair, admit_import, projection_identity,
};
pub use export_import_projection::{
    ProjectionAction, ProjectionController, ProjectionLeaseState, ProjectionLifecycleError,
    ProjectionObservation, ProjectionPhase, ProjectionPlan, ProjectionRouteState,
    ProjectionService, ProjectionServiceObservation,
};
pub use hints::{
    ChangeField, ChangeRecord, ControllerBinding, ControllerHint, ControllerLeaseKey,
    CoreTriggerReason, FairAdmission, HintAdmissionError, HintAdmissionOutcome, HintTarget,
    SuppressionDecision, WatchPlan, WatchPlanError, WatchRegistry, WatchSelector,
};
pub use owner_reconcile::{
    DesiredChild, ObservedChild, OwnerGraph, OwnerGraphError, OwnerIndex, OwnerLimits,
    OwnerMutation, OwnerReconcileError, OwnerReconcilePlan, OwnerTrigger,
};
pub use runtime::{
    CoreAdmissionCounts, CoreControllerSource, CoreDispatchOutcome, CoreReconcileError,
    CoreResourceReconciler, CoreSourceError, RegisteredControllerApi,
};
