//! Store-watch driven async controller loop.

use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, mpsc},
    task::{Context, Poll, Wake, Waker},
    thread,
};

use d2b_contracts::v3::{
    ConfigurationGeneration, ResourceGeneration, ResourcePhase, ResourceTypeName, ZoneRevision,
};

use crate::{
    Cancellation, CommittedRevisionProof, ContextError, ControllerHealth, ControllerIdentity,
    DependencySnapshot, DrainResult, FinalizeResult, ObservationResult, OperationContext,
    PendingQueue, PriorityLane, ProjectionDisposition, QueueError, QueueHint, QueuedWork,
    ReconcileContext, ReconcileDisposition, ReconcilePlan, ReconcileProjection, ReconcileResult,
    ResourceKey, ResourceSnapshot, StatusPersistence, TriggerReason, TriggerSet, UpdateAssessment,
    UpdateAssessmentState, UpgradePlan, ValidationResult,
};

/// Signed controller shape accepted by Core registration.
#[derive(Clone, PartialEq, Eq)]
pub struct ControllerDescriptor {
    identity: ControllerIdentity,
    resource_types: Vec<ResourceTypeName>,
    reconcile_concurrency: usize,
    max_pending_resources: usize,
    max_expedited_per_resource: usize,
    initial_watch_credits: u32,
}

impl ControllerDescriptor {
    /// Construct a bounded descriptor.
    pub fn new(
        identity: ControllerIdentity,
        mut resource_types: Vec<ResourceTypeName>,
        reconcile_concurrency: usize,
        max_pending_resources: usize,
        max_expedited_per_resource: usize,
        initial_watch_credits: u32,
    ) -> Result<Self, RunnerError> {
        resource_types.sort();
        let resource_type_count = resource_types.len();
        resource_types.dedup();
        if resource_types.is_empty()
            || resource_types.len() != resource_type_count
            || reconcile_concurrency == 0
            || max_pending_resources == 0
            || max_expedited_per_resource == 0
            || initial_watch_credits == 0
            || reconcile_concurrency > max_pending_resources
        {
            return Err(RunnerError::InvalidDescriptor);
        }
        Ok(Self {
            identity,
            resource_types,
            reconcile_concurrency,
            max_pending_resources,
            max_expedited_per_resource,
            initial_watch_credits,
        })
    }

    /// Borrow the registered identity.
    pub const fn identity(&self) -> &ControllerIdentity {
        &self.identity
    }

    /// Borrow owned ResourceTypes.
    pub fn resource_types(&self) -> &[ResourceTypeName] {
        &self.resource_types
    }

    /// Return the global handler semaphore bound.
    pub const fn reconcile_concurrency(&self) -> usize {
        self.reconcile_concurrency
    }

    /// Return the pending-resource bound.
    pub const fn max_pending_resources(&self) -> usize {
        self.max_pending_resources
    }

    /// Return the expedited per-resource bound.
    pub const fn max_expedited_per_resource(&self) -> usize {
        self.max_expedited_per_resource
    }

    /// Return initial watch credit.
    pub const fn initial_watch_credits(&self) -> u32 {
        self.initial_watch_credits
    }

    fn event_channel_capacity(&self) -> usize {
        self.reconcile_concurrency.saturating_add(1)
    }
}

impl core::fmt::Debug for ControllerDescriptor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ControllerDescriptor")
            .field("identity", &self.identity)
            .field("resource_type_count", &self.resource_types.len())
            .field("reconcile_concurrency", &self.reconcile_concurrency)
            .field("max_pending_resources", &self.max_pending_resources)
            .field(
                "max_expedited_per_resource",
                &self.max_expedited_per_resource,
            )
            .field("initial_watch_credits", &self.initial_watch_credits)
            .finish()
    }
}

/// Initial list entry.
#[derive(Clone, PartialEq, Eq)]
pub struct InitialResource {
    key: ResourceKey,
    revision: ZoneRevision,
}

impl InitialResource {
    /// Construct a listed identity at its snapshot revision.
    pub fn new(key: ResourceKey, revision: ZoneRevision) -> Self {
        Self { key, revision }
    }
}

impl core::fmt::Debug for InitialResource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InitialResource")
            .field("key", &self.key)
            .field("revision", &self.revision)
            .finish()
    }
}

/// Complete initial list plus the durable snapshot revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialList {
    pub resources: Vec<InitialResource>,
    pub snapshot_revision: ZoneRevision,
}

/// Store-contract watch hint.
#[derive(Clone, PartialEq, Eq)]
pub struct WatchHint {
    key: ResourceKey,
    revision: ZoneRevision,
    reasons: TriggerSet,
    lane: PriorityLane,
    operation: OperationContext,
}

impl WatchHint {
    /// Construct a watch hint.
    pub fn new(
        key: ResourceKey,
        revision: ZoneRevision,
        reasons: TriggerSet,
        lane: PriorityLane,
        operation: OperationContext,
    ) -> Self {
        Self {
            key,
            revision,
            reasons,
            lane,
            operation,
        }
    }

    fn into_queue_hint(self) -> Result<QueueHint, QueueError> {
        QueueHint::new(
            self.key,
            self.revision,
            self.reasons,
            self.lane,
            self.operation,
        )
    }
}

impl core::fmt::Debug for WatchHint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WatchHint")
            .field("key", &self.key)
            .field("revision", &self.revision)
            .field("reasons", &self.reasons)
            .field("lane", &self.lane)
            .field("operation", &self.operation)
            .finish()
    }
}

/// One watch receiver event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    Hint(Box<WatchHint>),
    Closed,
}

/// Recoverable or fatal watch failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchFailure {
    Disconnected,
    RevisionExpired,
    Fatal,
}

/// Fresh read after a queue item wins dispatch.
#[derive(Clone, PartialEq, Eq)]
pub enum FreshSnapshot {
    Present {
        target: ResourceSnapshot,
        dependencies: Vec<DependencySnapshot>,
    },
    Deleted {
        key: ResourceKey,
        revision: ZoneRevision,
        generation: ResourceGeneration,
    },
}

impl core::fmt::Debug for FreshSnapshot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Present {
                target,
                dependencies,
            } => f
                .debug_struct("FreshSnapshot::Present")
                .field("target", target)
                .field("dependency_count", &dependencies.len())
                .finish(),
            Self::Deleted {
                key,
                revision,
                generation,
            } => f
                .debug_struct("FreshSnapshot::Deleted")
                .field("key", key)
                .field("revision", revision)
                .field("generation", generation)
                .finish(),
        }
    }
}

/// Expedited admission decision.
#[derive(Debug)]
pub enum CommitDecision {
    Committed {
        resource_uid: d2b_contracts::v3::ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
        operation_id: String,
    },
    Abort,
}

/// Durable commit outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOutcome {
    Committed(ZoneRevision),
    CommittedStatusPending(ZoneRevision),
    Conflict(ZoneRevision),
}

/// Store-contract error with no backend handle or path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceError {
    Unavailable,
    Backpressure,
    Conflict(ZoneRevision),
    Cancelled,
    Timeout,
    Integrity,
}

impl core::fmt::Display for SourceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "resource plane unavailable",
            Self::Backpressure => "resource plane backpressure",
            Self::Conflict(_) => "resource revision conflict",
            Self::Cancelled => "resource operation cancelled",
            Self::Timeout => "resource operation timed out",
            Self::Integrity => "resource plane integrity failure",
        })
    }
}

impl std::error::Error for SourceError {}

/// Capability-limited store/watch seam used by the runner.
///
/// Implementations adapt the registered resource API. No method exposes a
/// database transaction, path, socket, or reusable authorization credential.
pub trait ControllerSource: Send + Sync + 'static {
    fn register(
        &self,
        descriptor: &ControllerDescriptor,
    ) -> impl Future<Output = Result<(), SourceError>> + Send;

    fn list_initial(
        &self,
        descriptor: &ControllerDescriptor,
    ) -> impl Future<Output = Result<InitialList, SourceError>> + Send;

    fn open_watch(
        &self,
        descriptor: &ControllerDescriptor,
        after_revision: ZoneRevision,
    ) -> impl Future<Output = Result<(), SourceError>> + Send;

    fn receive_watch(&self) -> impl Future<Output = Result<WatchEvent, WatchFailure>> + Send;

    fn read_fresh(
        &self,
        key: &ResourceKey,
    ) -> impl Future<Output = Result<FreshSnapshot, SourceError>> + Send;

    fn write_starting(
        &self,
        context: &ReconcileContext,
    ) -> impl Future<Output = Result<(), SourceError>> + Send;

    fn await_expedited_commit(
        &self,
        context: &ReconcileContext,
    ) -> impl Future<Output = Result<CommitDecision, SourceError>> + Send;

    fn commit_result(
        &self,
        context: &ReconcileContext,
        result: &ReconcileResult,
    ) -> impl Future<Output = Result<CommitOutcome, SourceError>> + Send;

    fn complete_expedited(
        &self,
        context: &ReconcileContext,
        projection: &ReconcileProjection,
        status_persistence: StatusPersistence,
    ) -> impl Future<Output = Result<(), SourceError>> + Send;

    fn checkpoint(
        &self,
        context: &ReconcileContext,
        revision: ZoneRevision,
    ) -> impl Future<Output = Result<(), SourceError>> + Send;

    fn schedule_requeue(
        &self,
        key: &ResourceKey,
        at_tick: u64,
    ) -> impl Future<Output = Result<(), SourceError>> + Send;
}

/// Official asynchronous controller handler surface.
pub trait ResourceReconciler: Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    fn describe(&self) -> impl Future<Output = Result<ControllerDescriptor, Self::Error>> + Send;

    fn validate_spec(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
    ) -> impl Future<Output = Result<ValidationResult, Self::Error>> + Send;

    fn plan(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
    ) -> impl Future<Output = Result<ReconcilePlan, Self::Error>> + Send;

    fn reconcile(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
        plan: &ReconcilePlan,
    ) -> impl Future<Output = Result<ReconcileResult, Self::Error>> + Send;

    fn observe(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
    ) -> impl Future<Output = Result<ObservationResult, Self::Error>> + Send;

    fn finalize(
        &self,
        context: &ReconcileContext,
        deleting_resource: &ResourceSnapshot,
    ) -> impl Future<Output = Result<FinalizeResult, Self::Error>> + Send;

    fn health(&self) -> impl Future<Output = Result<ControllerHealth, Self::Error>> + Send;

    fn drain(
        &self,
        deadline_tick: u64,
    ) -> impl Future<Output = Result<DrainResult, Self::Error>> + Send;

    fn assess_update(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
    ) -> impl Future<Output = Result<UpdateAssessment, Self::Error>> + Send;

    fn plan_upgrade(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
    ) -> impl Future<Output = Result<UpgradePlan, Self::Error>> + Send;

    fn execute_upgrade(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
        plan: &UpgradePlan,
    ) -> impl Future<Output = Result<ReconcileResult, Self::Error>> + Send;
}

/// Revisions and deadlines fixed by the registered session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunnerConfig {
    pub policy_revision: u64,
    pub api_revision: u64,
    pub configuration_revision: ConfigurationGeneration,
    pub deadline_tick: u64,
    pub max_attempts: u32,
}

/// Successful loop summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunnerReport {
    pub dispatched: usize,
    pub checkpointed: usize,
    pub conflicts_retried: usize,
    pub relists: usize,
    pub handler_retries: usize,
    pub handler_failures: usize,
    pub committed_status_pending: usize,
}

/// Controller-loop failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerError {
    InvalidDescriptor,
    Controller,
    Source(SourceError),
    Queue(QueueError),
    Context(ContextError),
    ReceiverPanicked,
}

impl core::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidDescriptor => "controller descriptor bounds are invalid",
            Self::Controller => "controller handler failed",
            Self::Source(_) => "controller source failed",
            Self::Queue(_) => "controller queue failed",
            Self::Context(_) => "reconcile context failed",
            Self::ReceiverPanicked => "watch receiver panicked",
        })
    }
}

impl std::error::Error for RunnerError {}

impl From<SourceError> for RunnerError {
    fn from(value: SourceError) -> Self {
        Self::Source(value)
    }
}

impl From<QueueError> for RunnerError {
    fn from(value: QueueError) -> Self {
        Self::Queue(value)
    }
}

impl From<ContextError> for RunnerError {
    fn from(value: ContextError) -> Self {
        Self::Context(value)
    }
}

/// Async orchestration loop.
pub struct Runner<R, S> {
    reconciler: Arc<R>,
    source: Arc<S>,
    config: RunnerConfig,
}

impl<R, S> Runner<R, S>
where
    R: ResourceReconciler,
    S: ControllerSource,
{
    /// Bind a reconciler to its capability-limited source.
    pub fn new(reconciler: Arc<R>, source: Arc<S>, config: RunnerConfig) -> Self {
        Self {
            reconciler,
            source,
            config,
        }
    }

    /// Register, list, watch, and reconcile until the watch closes and work drains.
    ///
    /// Blocking store adapters and the internal coordination channel run on a
    /// dedicated orchestration thread. Polling this future never blocks the
    /// caller's async executor.
    pub fn run(&self) -> RunnerFuture {
        let runner = Self {
            reconciler: Arc::clone(&self.reconciler),
            source: Arc::clone(&self.source),
            config: self.config,
        };
        RunnerFuture::spawn(move || block_on(runner.run_inner()))
    }

    async fn run_inner(&self) -> Result<RunnerReport, RunnerError> {
        let descriptor = self
            .reconciler
            .describe()
            .await
            .map_err(|_| RunnerError::Controller)?;
        if self.config.max_attempts == 0 {
            return Err(RunnerError::InvalidDescriptor);
        }
        self.source.register(&descriptor).await?;
        let initial = self.source.list_initial(&descriptor).await?;
        self.source
            .open_watch(&descriptor, initial.snapshot_revision)
            .await?;

        let queue = Arc::new(PendingQueue::new(
            descriptor.max_pending_resources,
            descriptor.max_expedited_per_resource,
        ));
        queue.rebuild(initial_hints(&descriptor, initial.resources)?)?;

        let (sender, receiver) = mpsc::sync_channel(descriptor.event_channel_capacity());
        spawn_receiver(Arc::clone(&self.source), sender.clone());
        let mut report = RunnerReport::default();
        let mut active = 0_usize;
        let mut watch_closed = false;

        loop {
            while active < descriptor.reconcile_concurrency {
                let Some(work) = queue.pop_ready() else {
                    break;
                };
                active += 1;
                report.dispatched += 1;
                spawn_worker(
                    Arc::clone(&self.reconciler),
                    Arc::clone(&self.source),
                    descriptor.identity.clone(),
                    self.config,
                    work,
                    sender.clone(),
                );
            }

            if watch_closed && active == 0 && queue.is_empty() {
                return Ok(report);
            }

            match receiver.recv().map_err(|_| RunnerError::ReceiverPanicked)? {
                RunnerEvent::Watch(Ok(WatchEvent::Hint(hint))) => {
                    if !descriptor_owns_key(&descriptor, &hint.key) {
                        return Err(RunnerError::Source(SourceError::Integrity));
                    }
                    queue.push((*hint).into_queue_hint()?)?;
                }
                RunnerEvent::Watch(Ok(WatchEvent::Closed)) => {
                    watch_closed = true;
                }
                RunnerEvent::Watch(Err(
                    WatchFailure::Disconnected | WatchFailure::RevisionExpired,
                )) => {
                    let relist = self.source.list_initial(&descriptor).await?;
                    self.source
                        .open_watch(&descriptor, relist.snapshot_revision)
                        .await?;
                    queue.rebuild(initial_hints(&descriptor, relist.resources)?)?;
                    report.relists += 1;
                    watch_closed = false;
                    spawn_receiver(Arc::clone(&self.source), sender.clone());
                }
                RunnerEvent::Watch(Err(WatchFailure::Fatal)) => {
                    return Err(RunnerError::Source(SourceError::Integrity));
                }
                RunnerEvent::Worker(completion) => {
                    let completion = *completion;
                    active = active.saturating_sub(1);
                    let key = completion.work.key().clone();
                    match completion.outcome {
                        WorkerOutcome::Done {
                            checkpointed,
                            status_pending,
                        } => {
                            queue.finish(&key)?;
                            report.checkpointed += usize::from(checkpointed);
                            report.committed_status_pending += usize::from(status_pending);
                        }
                        WorkerOutcome::Retry(revision) => {
                            if completion.work.attempt() >= self.config.max_attempts {
                                queue.finish(&key)?;
                                report.handler_failures += 1;
                            } else {
                                queue.retry(completion.work, revision)?;
                                report.conflicts_retried += 1;
                            }
                        }

                        WorkerOutcome::HandlerFailed => {
                            if completion.work.attempt() >= self.config.max_attempts {
                                queue.finish(&key)?;
                                report.handler_failures += 1;
                            } else {
                                let revision = completion.work.high_water_revision();
                                queue.retry(completion.work, revision)?;
                                report.handler_retries += 1;
                            }
                        }
                        WorkerOutcome::SourceFailed(error) => {
                            queue.finish(&key)?;
                            return Err(RunnerError::Source(error));
                        }
                    }
                }
            }
        }
    }
}

struct RunnerFutureState {
    result: Mutex<Option<Result<RunnerReport, RunnerError>>>,
    waker: Mutex<Option<Waker>>,
}

/// Nonblocking future returned by [`Runner::run`].
pub struct RunnerFuture {
    state: Arc<RunnerFutureState>,
}

impl RunnerFuture {
    fn spawn(run: impl FnOnce() -> Result<RunnerReport, RunnerError> + Send + 'static) -> Self {
        let state = Arc::new(RunnerFutureState {
            result: Mutex::new(None),
            waker: Mutex::new(None),
        });
        let worker_state = Arc::clone(&state);
        thread::spawn(move || {
            let result = run();
            *worker_state
                .result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
            if let Some(waker) = worker_state
                .waker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                waker.wake();
            }
        });
        Self { state }
    }
}

impl Future for RunnerFuture {
    type Output = Result<RunnerReport, RunnerError>;

    fn poll(self: std::pin::Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(result) = self
            .state
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            return Poll::Ready(result);
        }
        *self
            .state
            .waker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(context.waker().clone());
        if let Some(result) = self
            .state
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            Poll::Ready(result)
        } else {
            Poll::Pending
        }
    }
}

impl core::fmt::Debug for RunnerFuture {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RunnerFuture(<redacted>)")
    }
}

fn descriptor_owns_key(descriptor: &ControllerDescriptor, key: &ResourceKey) -> bool {
    key.zone() == descriptor.identity().zone()
        && descriptor
            .resource_types()
            .contains(key.resource_ref().resource_type())
}

fn initial_hints(
    descriptor: &ControllerDescriptor,
    resources: Vec<InitialResource>,
) -> Result<Vec<QueueHint>, RunnerError> {
    resources
        .into_iter()
        .map(|resource| {
            if !descriptor_owns_key(descriptor, &resource.key) {
                return Err(RunnerError::Source(SourceError::Integrity));
            }
            let canonical = resource.key.resource_ref().to_canonical_string();
            Ok(QueueHint::new(
                resource.key,
                resource.revision,
                TriggerSet::new([TriggerReason::StartupRelist]),
                PriorityLane::Ordinary,
                OperationContext::new(
                    format!("startup:{canonical}"),
                    format!("startup:{canonical}"),
                    format!("startup:{canonical}"),
                    None,
                )
                .map_err(|_| QueueError::InvalidHint)?,
            )?)
        })
        .collect()
}

enum RunnerEvent {
    Watch(Result<WatchEvent, WatchFailure>),
    Worker(Box<WorkerCompletion>),
}

struct WorkerCompletion {
    work: QueuedWork,
    outcome: WorkerOutcome,
}

enum WorkerOutcome {
    Done {
        checkpointed: bool,
        status_pending: bool,
    },
    Retry(ZoneRevision),
    HandlerFailed,
    SourceFailed(SourceError),
}

fn spawn_receiver<S>(source: Arc<S>, sender: mpsc::SyncSender<RunnerEvent>)
where
    S: ControllerSource,
{
    thread::spawn(move || {
        loop {
            let event = catch_unwind(AssertUnwindSafe(|| block_on(source.receive_watch())))
                .unwrap_or(Err(WatchFailure::Fatal));
            let should_continue = matches!(event, Ok(WatchEvent::Hint(_)));
            if sender.send(RunnerEvent::Watch(event)).is_err() || !should_continue {
                break;
            }
        }
    });
}

fn spawn_worker<R, S>(
    reconciler: Arc<R>,
    source: Arc<S>,
    identity: ControllerIdentity,
    config: RunnerConfig,
    work: QueuedWork,
    sender: mpsc::SyncSender<RunnerEvent>,
) where
    R: ResourceReconciler,
    S: ControllerSource,
{
    thread::spawn(move || {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            block_on(execute_work(reconciler, source, identity, config, &work))
        }))
        .unwrap_or(WorkerOutcome::HandlerFailed);
        let _ = sender.send(RunnerEvent::Worker(Box::new(WorkerCompletion {
            work,
            outcome,
        })));
    });
}

async fn execute_work<R, S>(
    reconciler: Arc<R>,
    source: Arc<S>,
    identity: ControllerIdentity,
    config: RunnerConfig,
    work: &QueuedWork,
) -> WorkerOutcome
where
    R: ResourceReconciler,
    S: ControllerSource,
{
    let fresh = match source.read_fresh(work.key()).await {
        Ok(fresh) => fresh,
        Err(SourceError::Conflict(revision)) => return WorkerOutcome::Retry(revision),
        Err(error) => return WorkerOutcome::SourceFailed(error),
    };
    let (target, dependencies, event_only) = match fresh {
        FreshSnapshot::Present {
            target,
            dependencies,
        } => {
            if target.key() != work.key() {
                return WorkerOutcome::SourceFailed(SourceError::Integrity);
            }
            (target, dependencies, false)
        }
        FreshSnapshot::Deleted {
            key,
            revision,
            generation,
        } => {
            if &key != work.key() {
                return WorkerOutcome::SourceFailed(SourceError::Integrity);
            }
            (
                ResourceSnapshot::new(key, revision, generation, Vec::new(), true),
                Vec::new(),
                true,
            )
        }
    };

    let context_result = if work.lane() == PriorityLane::Expedited {
        ReconcileContext::expedited_pending(
            identity,
            &target,
            &dependencies,
            work.reasons().clone(),
            work.high_water_revision().max(target.revision()),
            work.operation().clone(),
            work.attempt(),
            config.deadline_tick,
            Cancellation::default(),
            config.policy_revision,
            config.api_revision,
            config.configuration_revision,
        )
    } else {
        ReconcileContext::ordinary(
            identity,
            &target,
            &dependencies,
            work.reasons().clone(),
            work.high_water_revision().max(target.revision()),
            work.operation().clone(),
            work.attempt(),
            config.deadline_tick,
            Cancellation::default(),
            config.policy_revision,
            config.api_revision,
            config.configuration_revision,
        )
    };
    let mut context = match context_result {
        Ok(context) => context,
        Err(_) => return WorkerOutcome::HandlerFailed,
    };

    let deleting = target.deleting()
        || work.reasons().contains(TriggerReason::DeletionRequested)
        || work.reasons().contains(TriggerReason::FinalizerRequired);
    let validation = if deleting {
        ValidationResult::Valid
    } else {
        match reconciler.validate_spec(&context, &target).await {
            Ok(validation) => validation,
            Err(_) => return WorkerOutcome::HandlerFailed,
        }
    };
    let expedited_plan = if work.lane() == PriorityLane::Expedited
        && !deleting
        && matches!(validation, ValidationResult::Valid)
    {
        match reconciler.plan(&context, &target, &dependencies).await {
            Ok(plan) => Some(plan),
            Err(_) => return WorkerOutcome::HandlerFailed,
        }
    } else {
        None
    };

    if work.lane() == PriorityLane::Expedited {
        match source.await_expedited_commit(&context).await {
            Ok(CommitDecision::Committed {
                resource_uid,
                generation,
                revision,
                operation_id,
            }) => {
                let proof =
                    CommittedRevisionProof::issue(resource_uid, generation, revision, operation_id);
                context = match context.bind_committed_proof(proof) {
                    Ok(context) => context,
                    Err(_) => return WorkerOutcome::HandlerFailed,
                };
            }
            Ok(CommitDecision::Abort) => {
                return WorkerOutcome::Done {
                    checkpointed: false,
                    status_pending: false,
                };
            }
            Err(error) => return WorkerOutcome::SourceFailed(error),
        }
    }

    if let ValidationResult::Invalid { reason_code } = validation {
        let projection = if work.lane() == PriorityLane::Expedited {
            match ReconcileProjection::new(
                target.key().clone(),
                target.revision(),
                ResourcePhase::Failed,
                ProjectionDisposition::Failed,
                reason_code,
                false,
            ) {
                Ok(projection) => Some(projection),
                Err(_) => return WorkerOutcome::HandlerFailed,
            }
        } else {
            None
        };
        return persist_result(
            source.as_ref(),
            &context,
            ReconcileResult::failed_terminal(target.revision(), target.generation(), projection),
        )
        .await;
    }

    if !event_only {
        match source.write_starting(&context).await {
            Ok(()) => {}
            Err(SourceError::Conflict(revision)) => return WorkerOutcome::Retry(revision),
            Err(error) => return WorkerOutcome::SourceFailed(error),
        }
    }

    if deleting {
        let result = match reconciler.finalize(&context, &target).await {
            Ok(result) => result.into_result(),
            Err(_) => return WorkerOutcome::HandlerFailed,
        };
        return persist_result(source.as_ref(), &context, result).await;
    }

    if work.reasons().contains(TriggerReason::UpgradeRequested) {
        let plan = match reconciler
            .plan_upgrade(&context, &target, &dependencies)
            .await
        {
            Ok(plan) => plan,
            Err(_) => return WorkerOutcome::HandlerFailed,
        };
        let result = match reconciler
            .execute_upgrade(&context, &target, &dependencies, &plan)
            .await
        {
            Ok(result) => result,
            Err(_) => return WorkerOutcome::HandlerFailed,
        };
        return persist_result(source.as_ref(), &context, result).await;
    }

    if work.reasons().contains(TriggerReason::ScheduledObserve) {
        let result = match reconciler.observe(&context, &target).await {
            Ok(result) => result.into_result(),
            Err(_) => return WorkerOutcome::HandlerFailed,
        };
        return persist_result(source.as_ref(), &context, result).await;
    }

    if work.reasons().requires_update_assessment() {
        let assessment = match reconciler
            .assess_update(&context, &target, &dependencies)
            .await
        {
            Ok(assessment) => assessment,
            Err(_) => return WorkerOutcome::HandlerFailed,
        };
        if assessment.state() == UpdateAssessmentState::UpgradeRequired {
            let projection = if work.lane() == PriorityLane::Expedited {
                ReconcileProjection::new(
                    target.key().clone(),
                    target.revision(),
                    ResourcePhase::Pending,
                    ProjectionDisposition::UpgradeRequired,
                    "upgrade-required",
                    false,
                )
                .ok()
            } else {
                None
            };
            return persist_result(
                source.as_ref(),
                &context,
                ReconcileResult::upgrade_required(
                    target.revision(),
                    target.generation(),
                    projection,
                ),
            )
            .await;
        }
    }

    let plan = if let Some(plan) = expedited_plan {
        plan
    } else {
        match reconciler.plan(&context, &target, &dependencies).await {
            Ok(plan) => plan,
            Err(_) => return WorkerOutcome::HandlerFailed,
        }
    };

    if plan.is_no_op() {
        return persist_result(
            source.as_ref(),
            &context,
            ReconcileResult::converged(target.revision(), target.generation()),
        )
        .await;
    }

    let result = match reconciler
        .reconcile(&context, &target, &dependencies, &plan)
        .await
    {
        Ok(result) => result,
        Err(_) => return WorkerOutcome::HandlerFailed,
    };
    persist_result(source.as_ref(), &context, result).await
}

async fn persist_result<S>(
    source: &S,
    context: &ReconcileContext,
    mut result: ReconcileResult,
) -> WorkerOutcome
where
    S: ControllerSource,
{
    if result.processed_revision() != context.revision()
        || result.processed_generation() != context.generation()
        || result.projection().is_some_and(|projection| {
            projection.target() != context.target() || projection.revision() != context.revision()
        })
    {
        return WorkerOutcome::HandlerFailed;
    }
    if context.is_expedited() && result.projection().is_none() {
        let (phase, disposition) = match result.disposition() {
            ReconcileDisposition::Converged => {
                (ResourcePhase::Ready, ProjectionDisposition::Converged)
            }
            ReconcileDisposition::Pending | ReconcileDisposition::RequeueAt => {
                (ResourcePhase::Pending, ProjectionDisposition::Progressing)
            }
            ReconcileDisposition::Degraded => {
                (ResourcePhase::Degraded, ProjectionDisposition::Blocked)
            }
            ReconcileDisposition::FailedRetryable | ReconcileDisposition::FailedTerminal => {
                (ResourcePhase::Failed, ProjectionDisposition::Failed)
            }
            ReconcileDisposition::Finalized => {
                (ResourcePhase::Deleted, ProjectionDisposition::Converged)
            }
        };
        let projection = match ReconcileProjection::new(
            context.target().clone(),
            context.revision(),
            phase,
            disposition,
            "reconcile-pass",
            false,
        ) {
            Ok(projection) => projection,
            Err(_) => return WorkerOutcome::HandlerFailed,
        };
        if result.attach_projection(projection).is_err() {
            return WorkerOutcome::HandlerFailed;
        }
    }

    if result.requires_commit() {
        match source.commit_result(context, &result).await {
            Ok(CommitOutcome::Committed(revision)) => {
                if revision < context.revision() {
                    return WorkerOutcome::SourceFailed(SourceError::Integrity);
                }
                if let Err(error) = complete_expedited(source, context, &result, None).await {
                    return WorkerOutcome::SourceFailed(error);
                }
                if let Err(error) = source.checkpoint(context, revision).await {
                    return WorkerOutcome::SourceFailed(error);
                }
                return WorkerOutcome::Done {
                    checkpointed: true,
                    status_pending: false,
                };
            }
            Ok(CommitOutcome::CommittedStatusPending(revision)) => {
                if revision < context.revision() {
                    return WorkerOutcome::SourceFailed(SourceError::Integrity);
                }
                if let Err(error) =
                    complete_expedited(source, context, &result, Some(StatusPersistence::Pending))
                        .await
                {
                    return WorkerOutcome::SourceFailed(error);
                }
                if let Err(error) = source.checkpoint(context, revision).await {
                    return WorkerOutcome::SourceFailed(error);
                }
                return WorkerOutcome::Done {
                    checkpointed: true,
                    status_pending: true,
                };
            }
            Ok(CommitOutcome::Conflict(revision)) | Err(SourceError::Conflict(revision)) => {
                return WorkerOutcome::Retry(revision);
            }
            Err(error) => return WorkerOutcome::SourceFailed(error),
        }
    }

    if let Err(error) = complete_expedited(source, context, &result, None).await {
        return WorkerOutcome::SourceFailed(error);
    }
    if let Some(next_tick) = result.next_tick()
        && let Err(error) = source.schedule_requeue(context.target(), next_tick).await
    {
        return WorkerOutcome::SourceFailed(error);
    }
    if result.disposition().is_terminal()
        || result.next_tick().is_some()
        || result.disposition() == ReconcileDisposition::Pending
    {
        if let Err(error) = source
            .checkpoint(context, result.processed_revision())
            .await
        {
            return WorkerOutcome::SourceFailed(error);
        }
        return WorkerOutcome::Done {
            checkpointed: true,
            status_pending: false,
        };
    }
    if result.disposition() == ReconcileDisposition::FailedRetryable {
        return WorkerOutcome::Retry(result.processed_revision());
    }
    WorkerOutcome::Done {
        checkpointed: false,
        status_pending: false,
    }
}

async fn complete_expedited<S>(
    source: &S,
    context: &ReconcileContext,
    result: &ReconcileResult,
    status_override: Option<StatusPersistence>,
) -> Result<(), SourceError>
where
    S: ControllerSource,
{
    if !context.is_expedited() {
        return Ok(());
    }
    let projection = result.projection().ok_or(SourceError::Integrity)?;
    source
        .complete_expedited(
            context,
            projection,
            status_override.unwrap_or(result.status_persistence()),
        )
        .await
}

struct ThreadNotify {
    ready: Mutex<bool>,
    signal: Condvar,
}

impl Wake for ThreadNotify {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let mut ready = self
            .ready
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *ready = true;
        self.signal.notify_one();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let notify = Arc::new(ThreadNotify {
        ready: Mutex::new(false),
        signal: Condvar::new(),
    });
    let waker = Waker::from(Arc::clone(&notify));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => {
                let mut ready = notify
                    .ready
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                while !*ready {
                    ready = notify
                        .signal
                        .wait(ready)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                *ready = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, VecDeque},
        convert::Infallible,
        sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        time::Duration,
    };

    use d2b_contracts::v3::{ControllerGeneration, ResourceRef, ResourceUid, ZoneId};

    use super::*;
    use crate::{DisruptionClass, StatusPersistence, UpgradeStage};

    type FreshMap = BTreeMap<ResourceKey, FreshSnapshot>;
    type CommitGate = (Mutex<bool>, Condvar);
    type Harness = (
        Arc<FakeReconciler>,
        mpsc::Receiver<(ResourceKey, &'static str)>,
        Arc<FakeSource>,
        mpsc::Sender<Result<WatchEvent, WatchFailure>>,
    );

    struct FakeSource {
        initial: Mutex<VecDeque<InitialList>>,
        fresh: Mutex<FreshMap>,
        watch_rx: Mutex<mpsc::Receiver<Result<WatchEvent, WatchFailure>>>,
        commit_gate: CommitGate,
        abort_expedited: AtomicBool,
        expedited_gate_error: AtomicBool,
        conflicts_remaining: AtomicUsize,
        commit_status_pending: AtomicBool,
        commit_revision: AtomicU64,
        commits: AtomicUsize,
        expedited_completions: AtomicUsize,
        pending_completions: AtomicUsize,
        checkpoints: AtomicUsize,
        starting: AtomicUsize,
        requeues: AtomicUsize,
        watch_opens: AtomicUsize,
    }

    impl FakeSource {
        fn new(
            initial: Vec<InitialList>,
            fresh: FreshMap,
        ) -> (Arc<Self>, mpsc::Sender<Result<WatchEvent, WatchFailure>>) {
            let (watch_tx, watch_rx) = mpsc::channel();
            (
                Arc::new(Self {
                    initial: Mutex::new(initial.into()),
                    fresh: Mutex::new(fresh),
                    watch_rx: Mutex::new(watch_rx),
                    commit_gate: (Mutex::new(false), Condvar::new()),
                    abort_expedited: AtomicBool::new(false),
                    expedited_gate_error: AtomicBool::new(false),
                    conflicts_remaining: AtomicUsize::new(0),
                    commit_status_pending: AtomicBool::new(false),
                    commit_revision: AtomicU64::new(10),
                    commits: AtomicUsize::new(0),
                    expedited_completions: AtomicUsize::new(0),
                    pending_completions: AtomicUsize::new(0),
                    checkpoints: AtomicUsize::new(0),
                    starting: AtomicUsize::new(0),
                    requeues: AtomicUsize::new(0),
                    watch_opens: AtomicUsize::new(0),
                }),
                watch_tx,
            )
        }

        fn release_commit_gate(&self) {
            *self
                .commit_gate
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
            self.commit_gate.1.notify_all();
        }
    }

    impl ControllerSource for FakeSource {
        fn register(
            &self,
            _descriptor: &ControllerDescriptor,
        ) -> impl Future<Output = Result<(), SourceError>> + Send {
            std::future::ready(Ok(()))
        }

        fn list_initial(
            &self,
            _descriptor: &ControllerDescriptor,
        ) -> impl Future<Output = Result<InitialList, SourceError>> + Send {
            std::future::ready(
                self.initial
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .pop_front()
                    .ok_or(SourceError::Unavailable),
            )
        }

        fn open_watch(
            &self,
            _descriptor: &ControllerDescriptor,
            _after_revision: ZoneRevision,
        ) -> impl Future<Output = Result<(), SourceError>> + Send {
            self.watch_opens.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(()))
        }

        fn receive_watch(&self) -> impl Future<Output = Result<WatchEvent, WatchFailure>> + Send {
            std::future::ready(
                self.watch_rx
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .recv()
                    .unwrap_or(Ok(WatchEvent::Closed)),
            )
        }

        fn read_fresh(
            &self,
            key: &ResourceKey,
        ) -> impl Future<Output = Result<FreshSnapshot, SourceError>> + Send {
            std::future::ready(
                self.fresh
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
            self.starting.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(()))
        }

        fn await_expedited_commit(
            &self,
            context: &ReconcileContext,
        ) -> impl Future<Output = Result<CommitDecision, SourceError>> + Send {
            let mut released = self
                .commit_gate
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while !*released {
                released = self
                    .commit_gate
                    .1
                    .wait(released)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            if self.expedited_gate_error.load(Ordering::SeqCst) {
                return std::future::ready(Err(SourceError::Unavailable));
            }
            let decision = if self.abort_expedited.load(Ordering::SeqCst) {
                CommitDecision::Abort
            } else {
                CommitDecision::Committed {
                    resource_uid: context.target().uid().clone(),
                    generation: context.generation(),
                    revision: context.revision(),
                    operation_id: context.operation().operation_id().to_owned(),
                }
            };
            std::future::ready(Ok(decision))
        }

        fn commit_result(
            &self,
            _context: &ReconcileContext,
            result: &ReconcileResult,
        ) -> impl Future<Output = Result<CommitOutcome, SourceError>> + Send {
            self.commits.fetch_add(1, Ordering::SeqCst);
            let remaining = self.conflicts_remaining.load(Ordering::SeqCst);
            let outcome = if remaining > 0 {
                self.conflicts_remaining.fetch_sub(1, Ordering::SeqCst);
                CommitOutcome::Conflict(ZoneRevision::new(9))
            } else if self.commit_status_pending.load(Ordering::SeqCst)
                || result.status_persistence() == StatusPersistence::Pending
            {
                CommitOutcome::CommittedStatusPending(ZoneRevision::new(
                    self.commit_revision.load(Ordering::SeqCst),
                ))
            } else {
                CommitOutcome::Committed(ZoneRevision::new(
                    self.commit_revision.load(Ordering::SeqCst),
                ))
            };
            std::future::ready(Ok(outcome))
        }

        fn complete_expedited(
            &self,
            _context: &ReconcileContext,
            _projection: &ReconcileProjection,
            status_persistence: StatusPersistence,
        ) -> impl Future<Output = Result<(), SourceError>> + Send {
            self.expedited_completions.fetch_add(1, Ordering::SeqCst);
            if status_persistence == StatusPersistence::Pending {
                self.pending_completions.fetch_add(1, Ordering::SeqCst);
            }
            std::future::ready(Ok(()))
        }

        fn checkpoint(
            &self,
            _context: &ReconcileContext,
            _revision: ZoneRevision,
        ) -> impl Future<Output = Result<(), SourceError>> + Send {
            self.checkpoints.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(()))
        }

        fn schedule_requeue(
            &self,
            _key: &ResourceKey,
            _at_tick: u64,
        ) -> impl Future<Output = Result<(), SourceError>> + Send {
            self.requeues.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(()))
        }
    }

    #[derive(Debug)]
    struct FakeError;

    impl core::fmt::Display for FakeError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("fake handler failed")
        }
    }

    impl std::error::Error for FakeError {}

    struct FakeReconciler {
        descriptor: ControllerDescriptor,
        entered_tx: mpsc::Sender<(ResourceKey, &'static str)>,
        release: Arc<(Mutex<usize>, Condvar)>,
        active: AtomicUsize,
        max_active: AtomicUsize,
        plan_count: AtomicUsize,
        assess_count: AtomicUsize,
        reconcile_count: AtomicUsize,
        observe_count: AtomicUsize,
        upgrade_count: AtomicUsize,
        finalizer_count: AtomicUsize,
        validation_valid: AtomicBool,
        handler_failures_remaining: AtomicUsize,
        block_handlers: AtomicBool,
        no_op_after_first: AtomicBool,
        assessment_state: Mutex<UpdateAssessmentState>,
        requeue_at: Mutex<Option<u64>>,
    }

    impl FakeReconciler {
        fn enter(&self, key: &ResourceKey, action: &'static str) {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            self.entered_tx.send((key.clone(), action)).unwrap();
            if self.block_handlers.load(Ordering::SeqCst) {
                let mut permits = self
                    .release
                    .0
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                while *permits == 0 {
                    permits = self
                        .release
                        .1
                        .wait(permits)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                *permits -= 1;
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
        }

        fn release(&self, count: usize) {
            *self
                .release
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) += count;
            self.release.1.notify_all();
        }
    }

    impl ResourceReconciler for FakeReconciler {
        type Error = FakeError;

        fn describe(
            &self,
        ) -> impl Future<Output = Result<ControllerDescriptor, Self::Error>> + Send {
            std::future::ready(Ok(self.descriptor.clone()))
        }

        fn validate_spec(
            &self,
            _context: &ReconcileContext,
            _resource: &ResourceSnapshot,
        ) -> impl Future<Output = Result<ValidationResult, Self::Error>> + Send {
            std::future::ready(Ok(if self.validation_valid.load(Ordering::SeqCst) {
                ValidationResult::Valid
            } else {
                ValidationResult::Invalid {
                    reason_code: "invalid-spec",
                }
            }))
        }

        fn plan(
            &self,
            _context: &ReconcileContext,
            _resource: &ResourceSnapshot,
            _dependencies: &[DependencySnapshot],
        ) -> impl Future<Output = Result<ReconcilePlan, Self::Error>> + Send {
            let failures = self.handler_failures_remaining.load(Ordering::SeqCst);
            if failures > 0 {
                self.handler_failures_remaining
                    .fetch_sub(1, Ordering::SeqCst);
                return std::future::ready(Err(FakeError));
            }
            let count = self.plan_count.fetch_add(1, Ordering::SeqCst);
            std::future::ready(
                ReconcilePlan::new(
                    if count > 0 && self.no_op_after_first.load(Ordering::SeqCst) {
                        Vec::new()
                    } else {
                        vec!["effect".to_owned()]
                    },
                    count > 0 && self.no_op_after_first.load(Ordering::SeqCst),
                )
                .map_err(|_| FakeError),
            )
        }

        fn reconcile(
            &self,
            context: &ReconcileContext,
            resource: &ResourceSnapshot,
            _dependencies: &[DependencySnapshot],
            _plan: &ReconcilePlan,
        ) -> impl Future<Output = Result<ReconcileResult, Self::Error>> + Send {
            let permit = context.authorize_effect().map_err(|_| FakeError);
            if permit.is_ok() {
                self.enter(resource.key(), "reconcile");
            }
            self.reconcile_count.fetch_add(1, Ordering::SeqCst);
            let requeue_at = *self
                .requeue_at
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::future::ready(permit.and_then(|_| {
                if let Some(next_tick) = requeue_at {
                    ReconcileResult::new(
                        resource.revision(),
                        resource.generation(),
                        None,
                        None,
                        ReconcileDisposition::RequeueAt,
                        Some(next_tick),
                        None,
                        StatusPersistence::NotRequested,
                    )
                } else {
                    ReconcileResult::new(
                        resource.revision(),
                        resource.generation(),
                        None,
                        Some(b"{}".to_vec()),
                        ReconcileDisposition::Pending,
                        None,
                        None,
                        StatusPersistence::Pending,
                    )
                }
                .map_err(|_| FakeError)
            }))
        }

        fn observe(
            &self,
            context: &ReconcileContext,
            resource: &ResourceSnapshot,
        ) -> impl Future<Output = Result<ObservationResult, Self::Error>> + Send {
            self.observe_count.fetch_add(1, Ordering::SeqCst);
            self.enter(resource.key(), "observe");
            std::future::ready(context.authorize_effect().map_err(|_| FakeError).map(|_| {
                ObservationResult::new(ReconcileResult::converged(
                    resource.revision(),
                    resource.generation(),
                ))
            }))
        }

        fn finalize(
            &self,
            context: &ReconcileContext,
            resource: &ResourceSnapshot,
        ) -> impl Future<Output = Result<FinalizeResult, Self::Error>> + Send {
            if context.authorize_effect().is_err() {
                return std::future::ready(Err(FakeError));
            }
            self.finalizer_count.fetch_add(1, Ordering::SeqCst);
            let projection = ReconcileProjection::new(
                resource.key().clone(),
                resource.revision(),
                ResourcePhase::Deleted,
                ProjectionDisposition::Converged,
                "deleted",
                resource.canonical_json().is_empty(),
            )
            .map_err(|_| FakeError)
            .and_then(|projection| {
                ReconcileResult::new(
                    resource.revision(),
                    resource.generation(),
                    None,
                    None,
                    ReconcileDisposition::Finalized,
                    None,
                    Some(projection),
                    StatusPersistence::NotRequested,
                )
                .map_err(|_| FakeError)
            })
            .map(FinalizeResult::new);
            std::future::ready(projection)
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
            self.assess_count.fetch_add(1, Ordering::SeqCst);
            let state = *self
                .assessment_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::future::ready(
                UpdateAssessment::new(state, Vec::new(), true).map_err(|_| FakeError),
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
                    DisruptionClass::Recycle,
                    true,
                    vec![
                        UpgradeStage::Drain(resource.key().resource_ref().clone()),
                        UpgradeStage::Recycle(resource.key().resource_ref().clone()),
                        UpgradeStage::Restart(resource.key().resource_ref().clone()),
                    ],
                )
                .map_err(|_| FakeError),
            )
        }

        fn execute_upgrade(
            &self,
            context: &ReconcileContext,
            resource: &ResourceSnapshot,
            _dependencies: &[DependencySnapshot],
            _plan: &UpgradePlan,
        ) -> impl Future<Output = Result<ReconcileResult, Self::Error>> + Send {
            let permitted = context.authorize_effect().map_err(|_| FakeError);
            if permitted.is_ok() {
                self.enter(resource.key(), "upgrade");
            }
            self.upgrade_count.fetch_add(1, Ordering::SeqCst);
            std::future::ready(
                permitted.map(|_| {
                    ReconcileResult::converged(resource.revision(), resource.generation())
                }),
            )
        }
    }

    fn key(name: &str, suffix: u8) -> ResourceKey {
        ResourceKey::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse(&format!("Process/{name}")).unwrap(),
            ResourceUid::parse(format!("123e4567-e89b-42d3-a456-4266141740{suffix:02}")).unwrap(),
        )
    }

    fn identity() -> ControllerIdentity {
        ControllerIdentity::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse("Process/controller").unwrap(),
            ControllerGeneration::new(1).unwrap(),
            ResourceRef::parse("Provider/runtime").unwrap(),
            ResourceGeneration::new(1).unwrap(),
            ResourceRef::parse("Process/controller").unwrap(),
            ResourceRef::parse("Host/system").unwrap(),
            None,
        )
        .unwrap()
    }

    fn resource(key: ResourceKey, revision: u64) -> FreshSnapshot {
        FreshSnapshot::Present {
            target: ResourceSnapshot::new(
                key,
                ZoneRevision::new(revision),
                ResourceGeneration::new(1).unwrap(),
                b"{}".to_vec(),
                false,
            ),
            dependencies: Vec::new(),
        }
    }

    fn initial(keys: &[ResourceKey]) -> InitialList {
        InitialList {
            resources: keys
                .iter()
                .cloned()
                .map(|key| InitialResource::new(key, ZoneRevision::new(1)))
                .collect(),
            snapshot_revision: ZoneRevision::new(1),
        }
    }

    fn harness(keys: Vec<ResourceKey>, concurrency: usize) -> Harness {
        let fresh = keys
            .iter()
            .cloned()
            .map(|key| (key.clone(), resource(key, 1)))
            .collect();
        let (source, watch_tx) = FakeSource::new(vec![initial(&keys)], fresh);
        let (entered_tx, entered_rx) = mpsc::channel();
        let reconciler = Arc::new(FakeReconciler {
            descriptor: ControllerDescriptor::new(
                identity(),
                vec![ResourceTypeName::parse("Process").unwrap()],
                concurrency,
                32,
                2,
                16,
            )
            .unwrap(),
            entered_tx,
            release: Arc::new((Mutex::new(0), Condvar::new())),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            plan_count: AtomicUsize::new(0),
            assess_count: AtomicUsize::new(0),
            reconcile_count: AtomicUsize::new(0),
            observe_count: AtomicUsize::new(0),
            upgrade_count: AtomicUsize::new(0),
            finalizer_count: AtomicUsize::new(0),
            validation_valid: AtomicBool::new(true),
            handler_failures_remaining: AtomicUsize::new(0),
            block_handlers: AtomicBool::new(true),
            no_op_after_first: AtomicBool::new(false),
            assessment_state: Mutex::new(UpdateAssessmentState::Current),
            requeue_at: Mutex::new(None),
        });
        (reconciler, entered_rx, source, watch_tx)
    }

    fn config() -> RunnerConfig {
        RunnerConfig {
            policy_revision: 1,
            api_revision: 2,
            configuration_revision: ConfigurationGeneration::new(3).unwrap(),
            deadline_tick: 100,
            max_attempts: 3,
        }
    }

    fn run_in_thread(
        reconciler: Arc<FakeReconciler>,
        source: Arc<FakeSource>,
    ) -> thread::JoinHandle<Result<RunnerReport, RunnerError>> {
        thread::spawn(move || block_on(Runner::new(reconciler, source, config()).run()))
    }

    fn wait_for(counter: &AtomicUsize, expected: usize) {
        for _ in 0..400 {
            if counter.load(Ordering::SeqCst) >= expected {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("counter did not reach {expected}");
    }

    #[test]
    fn runner_future_does_not_block_the_calling_executor() {
        let (reconciler, entered, source, watch_tx) = harness(vec![key("app", 1)], 1);
        let mut future =
            std::pin::pin!(Runner::new(Arc::clone(&reconciler), source, config()).run());

        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut context = Context::from_waker(Waker::noop());
        assert_eq!(future.as_mut().poll(&mut context), Poll::Pending);

        reconciler.release(1);
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();
        assert_eq!(block_on(future).unwrap().checkpointed, 1);
    }

    #[test]
    fn initial_list_rejects_keys_outside_the_registered_zone() {
        let (reconciler, _entered, _source, _watch_tx) = harness(Vec::new(), 1);
        let foreign = ResourceKey::new(
            ZoneId::parse("personal").unwrap(),
            ResourceRef::parse("Process/app").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap(),
        );

        assert_eq!(
            initial_hints(
                &reconciler.descriptor,
                vec![InitialResource::new(foreign, ZoneRevision::new(1))]
            )
            .unwrap_err(),
            RunnerError::Source(SourceError::Integrity)
        );
    }

    #[test]
    fn watch_rejects_keys_outside_registered_ownership() {
        let (reconciler, _entered, source, watch_tx) = harness(Vec::new(), 1);
        let runner = Runner::new(reconciler, source, config()).run();
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                ResourceKey::new(
                    ZoneId::parse("personal").unwrap(),
                    ResourceRef::parse("Process/app").unwrap(),
                    ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").unwrap(),
                ),
                ZoneRevision::new(1),
                TriggerSet::new([TriggerReason::DependencyChanged]),
                PriorityLane::Ordinary,
                OperationContext::new("watch", "watch", "watch", None).unwrap(),
            )))))
            .unwrap();

        assert_eq!(
            block_on(runner).unwrap_err(),
            RunnerError::Source(SourceError::Integrity)
        );
    }

    #[test]
    fn fresh_read_rejects_a_different_resource_key() {
        let requested = key("requested", 1);
        let (reconciler, _entered, source, watch_tx) = harness(vec![requested.clone()], 1);
        source
            .fresh
            .lock()
            .unwrap()
            .insert(requested, resource(key("other", 2), 1));
        let runner = Runner::new(reconciler, source, config()).run();
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        assert_eq!(
            block_on(runner).unwrap_err(),
            RunnerError::Source(SourceError::Integrity)
        );
    }

    #[test]
    fn committed_revision_cannot_move_checkpoint_backwards() {
        let (reconciler, entered, source, watch_tx) = harness(vec![key("app", 1)], 1);
        source.commit_revision.store(0, Ordering::SeqCst);
        let runner = run_in_thread(Arc::clone(&reconciler), source);

        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        reconciler.release(1);
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        assert_eq!(
            runner.join().unwrap().unwrap_err(),
            RunnerError::Source(SourceError::Integrity)
        );
    }

    #[test]
    fn independent_resources_contend_on_the_configured_semaphore() {
        let keys = vec![key("one", 1), key("two", 2), key("three", 3)];
        let (reconciler, entered, source, watch_tx) = harness(keys, 2);
        let runner = run_in_thread(Arc::clone(&reconciler), source);

        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            entered.recv_timeout(Duration::from_millis(100)).is_err(),
            "a third handler bypassed the semaphore"
        );
        assert_eq!(reconciler.max_active.load(Ordering::SeqCst), 2);
        reconciler.release(2);
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        reconciler.release(1);
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.dispatched, 3);
        assert_eq!(report.checkpointed, 3);
    }

    #[test]
    fn duplicate_hint_contends_with_running_handler_and_stays_single_flight() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(vec![target.clone()], 2);
        let runner = run_in_thread(Arc::clone(&reconciler), source);

        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(2),
                TriggerSet::new([TriggerReason::DependencyChanged]),
                PriorityLane::Ordinary,
                OperationContext::new("watch", "watch", "watch", None).unwrap(),
            )))))
            .unwrap();
        assert!(
            entered.recv_timeout(Duration::from_millis(100)).is_err(),
            "the same resource ran concurrently"
        );
        reconciler.release(1);
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(reconciler.max_active.load(Ordering::SeqCst), 1);
        reconciler.release(1);
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.dispatched, 2);
    }

    #[test]
    fn expedited_plan_finishes_but_effect_waits_for_commit_proof() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(Vec::new(), 1);
        let mut fresh = source
            .fresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        fresh.insert(target.clone(), resource(target.clone(), 4));
        drop(fresh);
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(4),
                TriggerSet::new([TriggerReason::ExpeditedMutation]),
                PriorityLane::Expedited,
                OperationContext::new("expedite", "expedite", "expedite", None).unwrap(),
            )))))
            .unwrap();

        wait_for(&reconciler.plan_count, 1);
        assert!(
            entered.recv_timeout(Duration::from_millis(100)).is_err(),
            "effect started before durable commit proof"
        );
        assert_eq!(source.expedited_completions.load(Ordering::SeqCst), 0);
        source.release_commit_gate();
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        reconciler.release(1);
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.dispatched, 1);
        assert_eq!(source.commits.load(Ordering::SeqCst), 1);
        assert_eq!(source.expedited_completions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn expedited_abort_produces_no_effect_or_status_commit() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(Vec::new(), 1);
        source
            .fresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(target.clone(), resource(target.clone(), 4));
        source.abort_expedited.store(true, Ordering::SeqCst);
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(4),
                TriggerSet::new([TriggerReason::ExpeditedMutation]),
                PriorityLane::Expedited,
                OperationContext::new("abort", "abort", "abort", None).unwrap(),
            )))))
            .unwrap();
        wait_for(&reconciler.plan_count, 1);
        source.release_commit_gate();
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert!(entered.try_recv().is_err());
        assert_eq!(reconciler.reconcile_count.load(Ordering::SeqCst), 0);
        assert_eq!(source.commits.load(Ordering::SeqCst), 0);
        assert_eq!(source.starting.load(Ordering::SeqCst), 0);
        assert_eq!(report.checkpointed, 0);
    }

    #[test]
    fn expedited_commit_failure_produces_no_effect() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(Vec::new(), 1);
        source
            .fresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(target.clone(), resource(target.clone(), 4));
        source.expedited_gate_error.store(true, Ordering::SeqCst);
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(4),
                TriggerSet::new([TriggerReason::ExpeditedMutation]),
                PriorityLane::Expedited,
                OperationContext::new("failed", "failed", "failed", None).unwrap(),
            )))))
            .unwrap();
        wait_for(&reconciler.plan_count, 1);
        source.release_commit_gate();

        assert_eq!(
            runner.join().unwrap().unwrap_err(),
            RunnerError::Source(SourceError::Unavailable)
        );
        assert!(entered.try_recv().is_err());
        assert_eq!(reconciler.reconcile_count.load(Ordering::SeqCst), 0);
        assert_eq!(source.expedited_completions.load(Ordering::SeqCst), 0);
        drop(watch_tx);
    }

    #[test]
    fn invalid_spec_finishes_terminally_without_planning_or_effects() {
        let (reconciler, _entered, source, watch_tx) = harness(vec![key("app", 1)], 1);
        reconciler.validation_valid.store(false, Ordering::SeqCst);
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.checkpointed, 1);
        assert_eq!(reconciler.plan_count.load(Ordering::SeqCst), 0);
        assert_eq!(reconciler.reconcile_count.load(Ordering::SeqCst), 0);
        assert_eq!(source.commits.load(Ordering::SeqCst), 0);
        assert_eq!(source.starting.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn expedited_invalid_spec_returns_a_failed_projection_after_proof() {
        let target = key("app", 1);
        let (reconciler, _entered, source, watch_tx) = harness(Vec::new(), 1);
        reconciler.validation_valid.store(false, Ordering::SeqCst);
        source
            .fresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(target.clone(), resource(target.clone(), 2));
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(2),
                TriggerSet::new([TriggerReason::ExpeditedMutation]),
                PriorityLane::Expedited,
                OperationContext::new("invalid", "invalid", "invalid", None).unwrap(),
            )))))
            .unwrap();
        source.release_commit_gate();
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        runner.join().unwrap().unwrap();
        assert_eq!(reconciler.plan_count.load(Ordering::SeqCst), 0);
        assert_eq!(reconciler.reconcile_count.load(Ordering::SeqCst), 0);
        assert_eq!(source.expedited_completions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn ordinary_reentry_after_expedited_effect_no_ops_without_duplicate() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(Vec::new(), 1);
        reconciler.no_op_after_first.store(true, Ordering::SeqCst);
        source
            .fresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(target.clone(), resource(target.clone(), 4));
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target.clone(),
                ZoneRevision::new(4),
                TriggerSet::new([TriggerReason::ExpeditedMutation]),
                PriorityLane::Expedited,
                OperationContext::new("expedited", "expedited", "expedited", None).unwrap(),
            )))))
            .unwrap();
        wait_for(&reconciler.plan_count, 1);
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(4),
                TriggerSet::new([TriggerReason::ManualReconcile]),
                PriorityLane::Ordinary,
                OperationContext::new("ordinary", "ordinary", "ordinary", None).unwrap(),
            )))))
            .unwrap();
        source.release_commit_gate();
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        reconciler.release(1);
        wait_for(&reconciler.plan_count, 2);
        assert!(
            entered.recv_timeout(Duration::from_millis(100)).is_err(),
            "ordinary reentry duplicated the effect"
        );
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.dispatched, 2);
        assert_eq!(reconciler.reconcile_count.load(Ordering::SeqCst), 1);
        assert_eq!(source.commits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stale_commit_reloads_and_retries_without_checkpointing_stale_output() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(vec![target], 1);
        source.conflicts_remaining.store(1, Ordering::SeqCst);
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));

        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        reconciler.release(1);
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        reconciler.release(1);
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.conflicts_retried, 1);
        assert_eq!(source.commits.load(Ordering::SeqCst), 2);
        assert_eq!(source.checkpoints.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn conflict_reentry_reuses_observed_effect_and_does_not_duplicate() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(vec![target], 1);
        source.conflicts_remaining.store(1, Ordering::SeqCst);
        reconciler.no_op_after_first.store(true, Ordering::SeqCst);
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));

        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        reconciler.release(1);
        wait_for(&reconciler.plan_count, 2);
        assert!(
            entered.recv_timeout(Duration::from_millis(100)).is_err(),
            "retry duplicated an already-started effect"
        );
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.conflicts_retried, 1);
        assert_eq!(reconciler.reconcile_count.load(Ordering::SeqCst), 1);
        assert_eq!(source.commits.load(Ordering::SeqCst), 1);
        assert_eq!(source.checkpoints.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn repeated_conflicts_stop_at_the_attempt_bound() {
        let target = key("app", 1);
        let (reconciler, _entered, source, watch_tx) = harness(vec![target], 1);
        reconciler.block_handlers.store(false, Ordering::SeqCst);
        source.conflicts_remaining.store(8, Ordering::SeqCst);
        let runner = run_in_thread(reconciler, Arc::clone(&source));
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.conflicts_retried, 2);
        assert_eq!(report.handler_failures, 1);
        assert_eq!(source.commits.load(Ordering::SeqCst), 3);
        assert_eq!(source.checkpoints.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn transient_handler_failure_retries_from_a_fresh_read() {
        let (reconciler, entered, source, watch_tx) = harness(vec![key("app", 1)], 1);
        reconciler
            .handler_failures_remaining
            .store(1, Ordering::SeqCst);
        reconciler.block_handlers.store(false, Ordering::SeqCst);
        let runner = run_in_thread(Arc::clone(&reconciler), source);

        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.dispatched, 2);
        assert_eq!(report.handler_retries, 1);
        assert_eq!(report.handler_failures, 0);
        assert_eq!(reconciler.plan_count.load(Ordering::SeqCst), 1);
        assert_eq!(reconciler.reconcile_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn requeue_at_uses_source_scheduler_and_terminal_checkpoint() {
        let target = key("app", 1);
        let (reconciler, _entered, source, watch_tx) = harness(vec![target], 1);
        reconciler.block_handlers.store(false, Ordering::SeqCst);
        *reconciler
            .requeue_at
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(42);
        let runner = run_in_thread(reconciler, Arc::clone(&source));
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(source.requeues.load(Ordering::SeqCst), 1);
        assert_eq!(source.checkpoints.load(Ordering::SeqCst), 1);
        assert_eq!(report.checkpointed, 1);
    }

    #[test]
    fn reconcile_and_upgrade_for_one_resource_are_serialized() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(vec![target.clone()], 2);
        let runner = run_in_thread(Arc::clone(&reconciler), source);

        let (_, action) = entered.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(action, "reconcile");
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(2),
                TriggerSet::new([TriggerReason::UpgradeRequested]),
                PriorityLane::Ordinary,
                OperationContext::new("upgrade", "upgrade", "upgrade", None).unwrap(),
            )))))
            .unwrap();
        assert!(
            entered.recv_timeout(Duration::from_millis(100)).is_err(),
            "upgrade overlapped reconcile"
        );
        reconciler.release(1);
        let (_, action) = entered.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(action, "upgrade");
        reconciler.release(1);
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        runner.join().unwrap().unwrap();
        assert_eq!(reconciler.upgrade_count.load(Ordering::SeqCst), 1);
        assert_eq!(reconciler.max_active.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn deletion_event_without_body_executes_event_only_finalizer_projection() {
        let target = key("gone", 1);
        let (reconciler, _entered, source, watch_tx) = harness(Vec::new(), 1);
        source
            .fresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                target.clone(),
                FreshSnapshot::Deleted {
                    key: target.clone(),
                    revision: ZoneRevision::new(7),
                    generation: ResourceGeneration::new(2).unwrap(),
                },
            );
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(7),
                TriggerSet::new([TriggerReason::DeletionRequested]),
                PriorityLane::Ordinary,
                OperationContext::new("delete", "delete", "delete", None).unwrap(),
            )))))
            .unwrap();
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        runner.join().unwrap().unwrap();
        assert_eq!(reconciler.finalizer_count.load(Ordering::SeqCst), 1);
        assert_eq!(source.starting.load(Ordering::SeqCst), 0);
        assert_eq!(source.checkpoints.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn expedited_delete_event_cannot_finalize_before_commit_proof() {
        let target = key("gone", 1);
        let (reconciler, _entered, source, watch_tx) = harness(Vec::new(), 1);
        source
            .fresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                target.clone(),
                FreshSnapshot::Deleted {
                    key: target.clone(),
                    revision: ZoneRevision::new(7),
                    generation: ResourceGeneration::new(2).unwrap(),
                },
            );
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(7),
                TriggerSet::new([
                    TriggerReason::DeletionRequested,
                    TriggerReason::ExpeditedMutation,
                ]),
                PriorityLane::Expedited,
                OperationContext::new("delete", "delete", "delete", None).unwrap(),
            )))))
            .unwrap();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(reconciler.finalizer_count.load(Ordering::SeqCst), 0);
        source.release_commit_gate();
        wait_for(&reconciler.finalizer_count, 1);
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        runner.join().unwrap().unwrap();
        assert_eq!(source.starting.load(Ordering::SeqCst), 0);
        assert_eq!(source.checkpoints.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn expedited_committed_but_pending_status_keeps_ordinary_reentry() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(Vec::new(), 1);
        source
            .fresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(target.clone(), resource(target.clone(), 2));
        source.commit_status_pending.store(true, Ordering::SeqCst);
        reconciler.no_op_after_first.store(true, Ordering::SeqCst);
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target.clone(),
                ZoneRevision::new(2),
                TriggerSet::new([TriggerReason::ExpeditedMutation]),
                PriorityLane::Expedited,
                OperationContext::new("fast", "fast", "fast", None).unwrap(),
            )))))
            .unwrap();
        wait_for(&reconciler.plan_count, 1);
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(2),
                TriggerSet::new([TriggerReason::ExecutionStatusChanged]),
                PriorityLane::Ordinary,
                OperationContext::new("rejoin", "rejoin", "rejoin", None).unwrap(),
            )))))
            .unwrap();
        source.release_commit_gate();
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        reconciler.release(1);
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.committed_status_pending, 1);
        assert_eq!(report.checkpointed, 2);
        assert_eq!(reconciler.reconcile_count.load(Ordering::SeqCst), 1);
        assert_eq!(source.expedited_completions.load(Ordering::SeqCst), 1);
        assert_eq!(source.pending_completions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn non_disruptive_assessment_continues_through_ordinary_reconcile() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(Vec::new(), 1);
        *reconciler
            .assessment_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            UpdateAssessmentState::NonDisruptive;
        source
            .fresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(target.clone(), resource(target.clone(), 2));
        reconciler.block_handlers.store(false, Ordering::SeqCst);
        let runner = run_in_thread(Arc::clone(&reconciler), source);
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(2),
                TriggerSet::new([TriggerReason::ArtifactOrImageChanged]),
                PriorityLane::Ordinary,
                OperationContext::new("assess", "assess", "assess", None).unwrap(),
            )))))
            .unwrap();
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let (_, action) = entered.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(action, "reconcile");
        runner.join().unwrap().unwrap();
        assert_eq!(reconciler.assess_count.load(Ordering::SeqCst), 1);
        assert_eq!(reconciler.reconcile_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn scheduled_observe_executes_observer_without_reconcile() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(Vec::new(), 1);
        reconciler.block_handlers.store(false, Ordering::SeqCst);
        source
            .fresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(target.clone(), resource(target.clone(), 2));
        let runner = run_in_thread(Arc::clone(&reconciler), source);
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(2),
                TriggerSet::new([TriggerReason::ScheduledObserve]),
                PriorityLane::Ordinary,
                OperationContext::new("observe", "observe", "observe", None).unwrap(),
            )))))
            .unwrap();
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let (_, action) = entered.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(action, "observe");
        runner.join().unwrap().unwrap();
        assert_eq!(reconciler.observe_count.load(Ordering::SeqCst), 1);
        assert_eq!(reconciler.reconcile_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn upgrade_required_assessment_never_applies_change_in_place() {
        let target = key("app", 1);
        let (reconciler, entered, source, watch_tx) = harness(Vec::new(), 1);
        *reconciler
            .assessment_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            UpdateAssessmentState::UpgradeRequired;
        source
            .fresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(target.clone(), resource(target.clone(), 2));
        let runner = run_in_thread(Arc::clone(&reconciler), Arc::clone(&source));
        watch_tx
            .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                target,
                ZoneRevision::new(2),
                TriggerSet::new([TriggerReason::AssessUpdateDue]),
                PriorityLane::Ordinary,
                OperationContext::new("assess", "assess", "assess", None).unwrap(),
            )))))
            .unwrap();
        wait_for(&reconciler.assess_count, 1);
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        runner.join().unwrap().unwrap();
        assert!(entered.try_recv().is_err());
        assert_eq!(reconciler.reconcile_count.load(Ordering::SeqCst), 0);
        assert_eq!(reconciler.upgrade_count.load(Ordering::SeqCst), 0);
        assert_eq!(source.checkpoints.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn every_currency_trigger_executes_assessment_in_the_runner() {
        let triggers = [
            TriggerReason::SpecGenerationChanged,
            TriggerReason::ControllerGenerationChanged,
            TriggerReason::ProviderGenerationChanged,
            TriggerReason::SecurityPolicyChanged,
            TriggerReason::ArtifactOrImageChanged,
            TriggerReason::DependencyChanged,
            TriggerReason::AssessUpdateDue,
        ];
        let (reconciler, _entered, source, watch_tx) = harness(Vec::new(), 4);
        reconciler.block_handlers.store(false, Ordering::SeqCst);
        for (index, trigger) in triggers.into_iter().enumerate() {
            let target = key(&format!("assess-{index}"), u8::try_from(index + 1).unwrap());
            source
                .fresh
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(target.clone(), resource(target.clone(), 3));
            watch_tx
                .send(Ok(WatchEvent::Hint(Box::new(WatchHint::new(
                    target,
                    ZoneRevision::new(3),
                    TriggerSet::new([trigger]),
                    PriorityLane::Ordinary,
                    OperationContext::new(
                        format!("assess-{index}"),
                        format!("assess-{index}"),
                        format!("assess-{index}"),
                        None,
                    )
                    .unwrap(),
                )))))
                .unwrap();
        }
        let runner = run_in_thread(Arc::clone(&reconciler), source);
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.dispatched, triggers.len());
        assert_eq!(
            reconciler.assess_count.load(Ordering::SeqCst),
            triggers.len()
        );
    }

    #[test]
    fn watch_revision_expiry_relists_and_reopens_after_new_snapshot() {
        let target = key("app", 1);
        let fresh = BTreeMap::from([(target.clone(), resource(target.clone(), 2))]);
        let (source, watch_tx) = FakeSource::new(
            vec![
                initial(&[]),
                InitialList {
                    resources: vec![InitialResource::new(target, ZoneRevision::new(2))],
                    snapshot_revision: ZoneRevision::new(2),
                },
            ],
            fresh,
        );
        let (entered_tx, entered) = mpsc::channel();
        let reconciler = Arc::new(FakeReconciler {
            descriptor: ControllerDescriptor::new(
                identity(),
                vec![ResourceTypeName::parse("Process").unwrap()],
                1,
                4,
                1,
                4,
            )
            .unwrap(),
            entered_tx,
            release: Arc::new((Mutex::new(1), Condvar::new())),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            plan_count: AtomicUsize::new(0),
            assess_count: AtomicUsize::new(0),
            reconcile_count: AtomicUsize::new(0),
            observe_count: AtomicUsize::new(0),
            upgrade_count: AtomicUsize::new(0),
            finalizer_count: AtomicUsize::new(0),
            validation_valid: AtomicBool::new(true),
            handler_failures_remaining: AtomicUsize::new(0),
            block_handlers: AtomicBool::new(false),
            no_op_after_first: AtomicBool::new(false),
            assessment_state: Mutex::new(UpdateAssessmentState::Current),
            requeue_at: Mutex::new(None),
        });
        let runner = run_in_thread(reconciler, Arc::clone(&source));
        watch_tx.send(Err(WatchFailure::RevisionExpired)).unwrap();
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        watch_tx.send(Ok(WatchEvent::Closed)).unwrap();

        let report = runner.join().unwrap().unwrap();
        assert_eq!(report.relists, 1);
        assert_eq!(source.watch_opens.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn every_currency_trigger_reaches_update_assessment() {
        let triggers = [
            TriggerReason::SpecGenerationChanged,
            TriggerReason::ControllerGenerationChanged,
            TriggerReason::ProviderGenerationChanged,
            TriggerReason::SecurityPolicyChanged,
            TriggerReason::ArtifactOrImageChanged,
            TriggerReason::DependencyChanged,
            TriggerReason::AssessUpdateDue,
        ];
        for trigger in triggers {
            assert!(trigger.requires_update_assessment(), "{trigger:?}");
        }
        assert!(!TriggerReason::ManualReconcile.requires_update_assessment());
    }

    #[test]
    fn custom_block_on_executes_a_pending_future_after_wake() {
        struct WakeOnce {
            polled: bool,
        }
        impl Future for WakeOnce {
            type Output = usize;

            fn poll(
                mut self: std::pin::Pin<&mut Self>,
                context: &mut Context<'_>,
            ) -> Poll<Self::Output> {
                if self.polled {
                    Poll::Ready(1)
                } else {
                    self.polled = true;
                    context.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        }

        assert_eq!(block_on(WakeOnce { polled: false }), 1);
        let _: Result<(), Infallible> = Ok(());
    }

    #[test]
    fn health_and_drain_async_contracts_execute() {
        let (reconciler, _entered, _source, _watch_tx) = harness(Vec::new(), 1);
        assert_eq!(
            block_on(reconciler.health()).unwrap(),
            ControllerHealth::Healthy
        );
        assert_eq!(
            block_on(reconciler.drain(100)).unwrap(),
            DrainResult::Drained
        );
    }

    #[test]
    fn descriptor_rejects_unbounded_or_empty_execution_shapes() {
        assert_eq!(
            ControllerDescriptor::new(identity(), Vec::new(), 1, 1, 1, 1,).unwrap_err(),
            RunnerError::InvalidDescriptor
        );
        assert_eq!(
            ControllerDescriptor::new(
                identity(),
                vec![ResourceTypeName::parse("Process").unwrap()],
                2,
                1,
                1,
                1,
            )
            .unwrap_err(),
            RunnerError::InvalidDescriptor
        );
        let duplicate = ResourceTypeName::parse("Process").unwrap();
        assert_eq!(
            ControllerDescriptor::new(identity(), vec![duplicate.clone(), duplicate], 1, 2, 1, 1,)
                .unwrap_err(),
            RunnerError::InvalidDescriptor
        );
    }

    #[test]
    fn internal_event_budget_tracks_workers_not_pending_resources() {
        let descriptor = ControllerDescriptor::new(
            identity(),
            vec![ResourceTypeName::parse("Process").unwrap()],
            4,
            4_096,
            2,
            8,
        )
        .unwrap();

        assert_eq!(descriptor.event_channel_capacity(), 5);
    }
}
