//! Controller registration contracts, manager-served snapshots, and the
//! owner-change hint DTO.
//!
//! The store-driven reconcile machinery this crate existed for (`Runner`,
//! `ControllerSource`, `PendingQueue`, the reconcile result/mutation protocol,
//! and the reconcile-pass context) was deleted with the persistent resource
//! database it coordinated.

pub mod context;
pub mod contract;
pub mod owner_hints;
pub mod state_migration;

pub use context::{DependencySnapshot, ResourceSnapshot};
pub use contract::{
    ControllerDescriptor, ControllerExecutionPolicy, ControllerIdentity, ControllerSelector,
    ControllerVerb, DescriptorError, ResourceKey, ResourceRegistration, ResyncPolicy,
    SelectorField, TriggerReason, TriggerSet,
};
pub use owner_hints::{
    MAX_OWNER_HINT_DEPTH, MAX_OWNER_HINT_WORK_ITEMS, OwnedResourceChangedHint, OwnerChangeEvent,
    OwnerHintCoalesceError, OwnerHintCoalesceOutcome,
};
pub use state_migration::{
    MigrationMember, MigrationPlan, MigrationPlanError, MigrationWorkerPhase,
    plan as plan_state_migration,
};
