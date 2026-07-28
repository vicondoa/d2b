//! Core-to-toolkit source and reconciler adapters.

use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use d2b_contracts::v3::ZoneRevision;
use d2b_controller_toolkit::{
    CommitDecision, CommitOutcome, ControllerDescriptor, ControllerHealth, ControllerSource,
    DependencySnapshot, DisruptionClass, DrainResult, FinalizeResult, FreshSnapshot,
    HandlerFailure, InitialList, ObservationResult, OperationContext, ReconcileContext,
    ReconcilePlan, ReconcileProjection, ReconcileReason, ReconcileResult, ResourceKey,
    ResourceReconciler, ResourceSnapshot, SourceError, StatusPersistence, UpdateAssessment,
    UpdateAssessmentState, UpgradePlan, ValidationResult, WatchEvent, WatchFailure,
};

use crate::{
    ChangeRecord, ControllerHint, ControllerLeaseKey, HintAdmissionError, SuppressionDecision,
};

/// Core adapter construction or hint dispatch failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreSourceError {
    Hint(HintAdmissionError),
    WatchClosed,
}

impl core::fmt::Display for CoreSourceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Hint(_) => "core hint is invalid",
            Self::WatchClosed => "controller watch is closed",
        })
    }
}

impl std::error::Error for CoreSourceError {}

/// In-process Core source that converts durable changes into toolkit hints.
pub struct CoreControllerSource {
    descriptor: ControllerDescriptor,
    initial: InitialList,
    snapshots: Mutex<BTreeMap<ResourceKey, FreshSnapshot>>,
    watch_tx: tokio::sync::mpsc::UnboundedSender<Result<WatchEvent, WatchFailure>>,
    watch_rx:
        tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<Result<WatchEvent, WatchFailure>>>,
    outcomes: Mutex<Vec<ReconcileReason>>,
    checkpoints: AtomicUsize,
    checkpoint_notify: tokio::sync::Notify,
}

impl CoreControllerSource {
    /// Construct a source bound to one complete registered descriptor.
    pub fn new(
        descriptor: ControllerDescriptor,
        initial: InitialList,
        snapshots: BTreeMap<ResourceKey, FreshSnapshot>,
    ) -> Arc<Self> {
        let (watch_tx, watch_rx) = tokio::sync::mpsc::unbounded_channel();
        Arc::new(Self {
            descriptor,
            initial,
            snapshots: Mutex::new(snapshots),
            watch_tx,
            watch_rx: tokio::sync::Mutex::new(watch_rx),
            outcomes: Mutex::new(Vec::new()),
            checkpoints: AtomicUsize::new(0),
            checkpoint_notify: tokio::sync::Notify::new(),
        })
    }

    /// Apply Core suppression and dispatch one canonical toolkit hint.
    pub fn dispatch_change(
        &self,
        controller: ControllerLeaseKey,
        change: ChangeRecord,
        operation: OperationContext,
    ) -> Result<SuppressionDecision, CoreSourceError> {
        let decision = change.suppression();
        if decision != SuppressionDecision::Dispatch {
            return Ok(decision);
        }
        let hint = ControllerHint::new(controller, change.target, change.revision, change.reasons)
            .map_err(CoreSourceError::Hint)?
            .into_watch_hint(operation);
        self.watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(hint))))
            .map_err(|_| CoreSourceError::WatchClosed)?;
        Ok(decision)
    }

    /// Close the watch after all admitted changes.
    pub fn close_watch(&self) -> Result<(), CoreSourceError> {
        self.watch_tx
            .send(Ok(WatchEvent::Closed))
            .map_err(|_| CoreSourceError::WatchClosed)
    }

    /// Return the number of durable checkpoints.
    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.load(Ordering::Acquire)
    }

    /// Wait for at least one durable checkpoint.
    pub async fn wait_for_checkpoint(&self) {
        loop {
            let notified = self.checkpoint_notify.notified();
            if self.checkpoint_count() > 0 {
                return;
            }
            notified.await;
        }
    }

    /// Return persisted closed outcome reasons.
    pub fn persisted_outcomes(&self) -> Vec<ReconcileReason> {
        self.outcomes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl ControllerSource for CoreControllerSource {
    fn register(
        &self,
        descriptor: &ControllerDescriptor,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(
            (descriptor == &self.descriptor)
                .then_some(())
                .ok_or(SourceError::Integrity),
        )
    }

    fn list_initial(
        &self,
        _descriptor: &ControllerDescriptor,
    ) -> impl Future<Output = Result<InitialList, SourceError>> + Send {
        std::future::ready(Ok(self.initial.clone()))
    }

    fn open_watch(
        &self,
        _descriptor: &ControllerDescriptor,
        _after_revision: ZoneRevision,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(Ok(()))
    }

    async fn receive_watch(&self) -> Result<WatchEvent, WatchFailure> {
        self.watch_rx
            .lock()
            .await
            .recv()
            .await
            .unwrap_or(Ok(WatchEvent::Closed))
    }

    fn read_fresh(
        &self,
        key: &ResourceKey,
    ) -> impl Future<Output = Result<FreshSnapshot, SourceError>> + Send {
        std::future::ready(
            self.snapshots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(key)
                .cloned()
                .ok_or(SourceError::Unavailable),
        )
    }

    fn write_starting(
        &self,
        _context: &ReconcileContext,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(Ok(()))
    }

    fn await_expedited_commit(
        &self,
        context: &ReconcileContext,
    ) -> impl Future<Output = Result<CommitDecision, SourceError>> + Send {
        std::future::ready(Ok(CommitDecision::Committed {
            zone: context.target().zone().clone(),
            resource_uid: context.target().uid().clone(),
            generation: context.generation(),
            revision: context.revision(),
            operation_id: context.operation().operation_id().to_owned(),
        }))
    }

    fn commit_result(
        &self,
        context: &ReconcileContext,
        _result: &ReconcileResult,
    ) -> impl Future<Output = Result<CommitOutcome, SourceError>> + Send {
        std::future::ready(Ok(CommitOutcome::Committed(context.revision())))
    }

    fn complete_expedited(
        &self,
        _context: &ReconcileContext,
        projection: &ReconcileProjection,
        _status_persistence: StatusPersistence,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.persist_outcome(projection)
    }

    fn persist_outcome(
        &self,
        projection: &ReconcileProjection,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.outcomes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(projection.reason());
        std::future::ready(Ok(()))
    }

    fn checkpoint(
        &self,
        _context: &ReconcileContext,
        _revision: ZoneRevision,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.checkpoints.fetch_add(1, Ordering::Release);
        self.checkpoint_notify.notify_waiters();
        std::future::ready(Ok(()))
    }

    fn schedule_requeue(
        &self,
        _key: &ResourceKey,
        _at_tick: u64,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(Ok(()))
    }
}

/// Core reconcile adapter error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreReconcileError;

impl core::fmt::Display for CoreReconcileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("core reconcile contract failed")
    }
}

impl std::error::Error for CoreReconcileError {}

/// Core's baseline reconciler for metadata-only convergence.
pub struct CoreResourceReconciler {
    descriptor: ControllerDescriptor,
}

impl CoreResourceReconciler {
    /// Bind the reconciler to its complete signed descriptor.
    pub fn new(descriptor: ControllerDescriptor) -> Arc<Self> {
        Arc::new(Self { descriptor })
    }
}

impl ResourceReconciler for CoreResourceReconciler {
    type Error = CoreReconcileError;

    fn classify_error(&self, _error: &Self::Error) -> HandlerFailure {
        HandlerFailure::terminal()
    }

    fn describe(&self) -> impl Future<Output = Result<ControllerDescriptor, Self::Error>> + Send {
        std::future::ready(Ok(self.descriptor.clone()))
    }

    fn validate_spec(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
    ) -> impl Future<Output = Result<ValidationResult, Self::Error>> + Send {
        std::future::ready(Ok(if resource.canonical_json().is_empty() {
            ValidationResult::Invalid {
                reason: ReconcileReason::InvalidSpec,
            }
        } else {
            ValidationResult::Valid
        }))
    }

    async fn plan(
        &self,
        _context: &ReconcileContext,
        _resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> Result<ReconcilePlan, Self::Error> {
        tokio::task::yield_now().await;
        ReconcilePlan::new(Vec::new(), true).map_err(|_| CoreReconcileError)
    }

    fn reconcile(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
        _plan: &ReconcilePlan,
    ) -> impl Future<Output = Result<ReconcileResult, Self::Error>> + Send {
        std::future::ready(
            context
                .authorize_effect()
                .map_err(|_| CoreReconcileError)
                .map(|_| ReconcileResult::converged(resource.revision(), resource.generation())),
        )
    }

    fn observe(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
    ) -> impl Future<Output = Result<ObservationResult, Self::Error>> + Send {
        std::future::ready(Ok(ObservationResult::new(ReconcileResult::converged(
            resource.revision(),
            resource.generation(),
        ))))
    }

    fn finalize(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
    ) -> impl Future<Output = Result<FinalizeResult, Self::Error>> + Send {
        std::future::ready(Ok(FinalizeResult::new(ReconcileResult::converged(
            resource.revision(),
            resource.generation(),
        ))))
    }

    fn health(&self) -> impl Future<Output = Result<ControllerHealth, Self::Error>> + Send {
        std::future::ready(Ok(ControllerHealth::Healthy))
    }

    fn drain(
        &self,
        _deadline_tick: u64,
    ) -> impl Future<Output = Result<DrainResult, Self::Error>> + Send {
        std::future::ready(Ok(DrainResult::Drained))
    }

    fn assess_update(
        &self,
        _context: &ReconcileContext,
        _resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> impl Future<Output = Result<UpdateAssessment, Self::Error>> + Send {
        std::future::ready(
            UpdateAssessment::new(UpdateAssessmentState::Current, Vec::new(), true)
                .map_err(|_| CoreReconcileError),
        )
    }

    fn plan_upgrade(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> impl Future<Output = Result<UpgradePlan, Self::Error>> + Send {
        std::future::ready(
            UpgradePlan::new(
                DisruptionClass::Restart,
                true,
                vec![d2b_controller_toolkit::UpgradeStage::Restart(
                    resource.key().resource_ref().clone(),
                )],
            )
            .map_err(|_| CoreReconcileError),
        )
    }

    fn execute_upgrade(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
        _plan: &UpgradePlan,
    ) -> impl Future<Output = Result<ReconcileResult, Self::Error>> + Send {
        std::future::ready(
            context
                .authorize_effect()
                .map_err(|_| CoreReconcileError)
                .map(|_| ReconcileResult::converged(resource.revision(), resource.generation())),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use d2b_contracts::v3::{
        ConfigurationGeneration, ControllerGeneration, ObservedGeneration, ResourceGeneration,
        ResourceRef, ResourceTypeName, ResourceUid, ZoneId,
    };
    use d2b_controller_toolkit::{
        ControllerExecutionPolicy, ControllerIdentity, ControllerSelector, ControllerVerb,
        ResourceRegistration, ResyncPolicy, Runner, RunnerConfig, SelectorField, TriggerReason,
    };

    use super::*;
    use crate::ChangeField;

    fn key() -> ResourceKey {
        ResourceKey::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse("Process/app").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap(),
        )
    }

    fn descriptor() -> ControllerDescriptor {
        let resource_type = ResourceTypeName::parse("Process").unwrap();
        ControllerDescriptor::new(
            ControllerIdentity::new(
                ZoneId::parse("work").unwrap(),
                ResourceRef::parse("Process/controller").unwrap(),
                ControllerGeneration::new(1).unwrap(),
                ResourceRef::parse("Provider/core").unwrap(),
                ResourceGeneration::new(1).unwrap(),
                ResourceRef::parse("Process/controller").unwrap(),
                ResourceRef::parse("Host/system").unwrap(),
                None,
            )
            .unwrap(),
            vec![ResourceRegistration::new(resource_type.clone(), vec![1], 5_000, 3).unwrap()],
            vec!["resource-api".to_owned()],
            vec!["host".to_owned()],
            vec![ControllerVerb::ReadSpec, ControllerVerb::WriteStatus],
            vec![ControllerSelector::new(resource_type, SelectorField::Spec, None).unwrap()],
            Vec::new(),
            true,
            vec!["d2b.io/core".to_owned()],
            vec!["service.v1".to_owned()],
            vec!["schema.v1".to_owned()],
            ControllerExecutionPolicy::new(
                1,
                1,
                8,
                1,
                4,
                ResyncPolicy::new(Some(100), 5_000).unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn durable_core_change_wakes_toolkit_queue_and_reconciles() {
        let target = key();
        let snapshot = ResourceSnapshot::new(
            target.clone(),
            ZoneRevision::new(2),
            ResourceGeneration::new(2).unwrap(),
            b"{}".to_vec(),
            false,
        );
        let descriptor = descriptor();
        let source = CoreControllerSource::new(
            descriptor.clone(),
            InitialList {
                resources: Vec::new(),
                snapshot_revision: ZoneRevision::new(1),
            },
            BTreeMap::from([(
                target.clone(),
                FreshSnapshot::Present {
                    target: snapshot,
                    dependencies: Vec::new(),
                },
            )]),
        );
        let reconciler = CoreResourceReconciler::new(descriptor);
        let runner = tokio::spawn(
            Runner::new(
                reconciler,
                Arc::clone(&source),
                RunnerConfig {
                    policy_revision: 1,
                    api_revision: 1,
                    configuration_revision: ConfigurationGeneration::new(1).unwrap(),
                    deadline_tick: 5_000,
                    max_attempts: 3,
                },
            )
            .run(),
        );

        tokio::task::yield_now().await;
        assert_eq!(
            source
                .dispatch_change(
                    ControllerLeaseKey::new(
                        ZoneId::parse("work").unwrap(),
                        ResourceRef::parse("Process/controller").unwrap(),
                    )
                    .unwrap(),
                    ChangeRecord {
                        target,
                        revision: ZoneRevision::new(2),
                        generation: ResourceGeneration::new(2).unwrap(),
                        observed_generation: ObservedGeneration::new(1),
                        fields: BTreeSet::from([ChangeField::Spec]),
                        reasons: BTreeSet::from([TriggerReason::SpecGenerationChanged]),
                        type_is_bound: true,
                        relevant_field_changed: true,
                        own_status_only: false,
                        owner_consumer_exists: false,
                        dependency_consumer_exists: false,
                        controller_generation_current: true,
                        conditions_require_work: false,
                        unknown_requires_observation: false,
                    },
                    OperationContext::new("op", "idem", "corr", None).unwrap(),
                )
                .unwrap(),
            SuppressionDecision::Dispatch
        );
        source.wait_for_checkpoint().await;
        assert_eq!(source.checkpoint_count(), 1);
        source.close_watch().unwrap();
        let report = runner.await.unwrap().unwrap();
        assert_eq!(report.dispatched, 1);
        assert_eq!(report.checkpointed, 1);
    }
}
