//! Async controller reconciliation toolkit.

pub mod context;
pub mod owner_hints;
pub mod queue;
pub mod result;
pub mod runner;

pub use context::{
    Cancellation, CommittedRevisionProof, ContextError, ControllerIdentity, DependencySnapshot,
    EffectPermit, OperationContext, ReconcileContext, ResourceKey, ResourceSnapshot, TriggerReason,
    TriggerSet,
};
pub use owner_hints::{
    MAX_OWNER_HINT_DEPTH, MAX_OWNER_HINT_WORK_ITEMS, OwnedResourceChangedHint, OwnerChangeEvent,
    OwnerHintCoalesceError, OwnerHintCoalesceOutcome,
};
pub use queue::{PendingQueue, PriorityLane, QueueError, QueueHint, QueuePushOutcome, QueuedWork};
pub use result::{
    ControllerHealth, DisruptionClass, DrainResult, FinalizeResult, MutationIntent,
    MutationIntentKind, ObservationResult, ProjectionDisposition, ReconcileDisposition,
    ReconcilePlan, ReconcileProjection, ReconcileResult, ResourceMutationBatch, StatusPersistence,
    UpdateAssessment, UpdateAssessmentState, UpgradePlan, UpgradeStage, ValidationResult,
};
pub use runner::{
    CommitDecision, CommitOutcome, ControllerDescriptor, ControllerSource, FreshSnapshot,
    InitialList, InitialResource, ResourceReconciler, Runner, RunnerConfig, RunnerError,
    RunnerFuture, RunnerReport, SourceError, WatchEvent, WatchFailure, WatchHint,
};
