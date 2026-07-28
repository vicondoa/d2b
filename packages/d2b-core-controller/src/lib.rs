//! Core-side reconciliation indexes, hints, suppression, and admission.

pub mod dependencies;
pub mod hints;
pub mod owner_reconcile;
pub mod rbac;

pub use dependencies::{DependencyError, DependencyIndex, DependencyTrigger, UpgradeOrder};
pub use hints::{
    ChangeField, ChangeRecord, ControllerBinding, ControllerHint, ControllerLeaseKey,
    CoreTriggerReason, FairAdmission, HintAdmissionError, HintTarget, SuppressionDecision,
    WatchPlan, WatchPlanError, WatchRegistry, WatchSelector,
};
pub use owner_reconcile::{
    DesiredChild, ObservedChild, OwnerGraph, OwnerGraphError, OwnerIndex, OwnerLimits,
    OwnerMutation, OwnerReconcileError, OwnerReconcilePlan, OwnerTrigger,
};
