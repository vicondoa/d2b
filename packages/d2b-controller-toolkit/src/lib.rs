//! Async controller reconciliation toolkit.

pub mod context;
pub mod contract;
pub mod owner_hints;
pub mod queue;
pub mod result;
pub mod runner;

pub use context::{
    Cancellation, CommittedRevisionProof, ContextError, DependencySnapshot, EffectPermit,
    OperationContext, ReconcileContext, ResourceSnapshot,
};
pub use contract::{
    ControllerDescriptor, ControllerExecutionPolicy, ControllerIdentity, ControllerSelector,
    ControllerVerb, DescriptorError, ResourceKey, ResourceRegistration, ResyncPolicy,
    SelectorField, TriggerReason, TriggerSet,
};
pub use owner_hints::{
    MAX_OWNER_HINT_DEPTH, MAX_OWNER_HINT_WORK_ITEMS, OwnedResourceChangedHint, OwnerChangeEvent,
    OwnerHintCoalesceError, OwnerHintCoalesceOutcome,
};
pub use queue::{PendingQueue, PriorityLane, QueueError, QueueHint, QueuePushOutcome, QueuedWork};
pub use result::{
    ControllerHealth, DisruptionClass, DrainResult, FinalizeResult, MutationIntent,
    MutationIntentKind, ObservationResult, ProjectionDisposition, ReconcileDisposition,
    ReconcilePlan, ReconcileProjection, ReconcileReason, ReconcileResult, ResourceMutationBatch,
    StatusPersistence, UpdateAssessment, UpdateAssessmentState, UpgradePlan, UpgradeStage,
    ValidationResult,
};
pub use runner::{
    CommitDecision, CommitOutcome, ControllerSource, FreshSnapshot, HandlerErrorClass,
    HandlerFailure, InitialList, InitialResource, MonotonicClock, ResourceReconciler, Runner,
    RunnerConfig, RunnerCounter, RunnerError, RunnerFuture, RunnerObservation,
    RunnerObservationReason, RunnerObserver, RunnerOutcome, RunnerReport, SourceError, WatchEvent,
    WatchFailure, WatchHint,
};
