//! One authoritative Ractor actor per desired resource (U3; spec sections 8,
//! 9, 14, 15, 19, 32).
//!
//! The actor is the exclusive owner of the resource's logical live state
//! (R1). Everything durable lives in the spec store, written only by the
//! manager; everything else - status, internal watchers, retry timers, and
//! effect bookkeeping - lives here, in memory only (R6, R11, R13).
//!
//! ## Scheduling and serialization (R14, spec section 32)
//!
//! `effect_running` / `reconcile_pending` serialize the same resource: while
//! an effect is in flight, further reconcile triggers coalesce into one
//! pending flag instead of entering the driver again. Different resources
//! reconcile concurrently - they are independent actors with independent
//! mailboxes.
//!
//! ## Requeue (R13, spec section 32)
//!
//! Retry state is runtime-only: a retryable failure schedules exactly one
//! ractor timer delivering one `Reconcile`. One pending timer per actor (a
//! new schedule cancels the previous), and the delete path cancels pending
//! schedules. After a restart the actor reconciles immediately instead of
//! restoring timers.
//!
//! ## Internal watches (R12, AE2)
//!
//! Watch registration is evaluated against the current status inside the
//! same mailbox handler: if the condition already holds, the subscriber is
//! notified immediately; otherwise the watcher is inserted. Every status
//! transition re-evaluates the registered watchers in the same handler, so a
//! condition that flips while the registration message is still queued
//! notifies exactly once. No database participates on this path.

pub const MODULE_NAME: &str = "resource";

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use ractor::{Actor, ActorCell, ActorProcessingErr, ActorRef};
use tokio::sync::mpsc;

use crate::context::{
    EffectCompleted, EffectResult, ManagerEndpoint, OperationId, RequeueId, RequeueScheduler,
    ResourceContext, SpecDecoder, WatchCondition, WatchId, WatchSatisfied,
};
use crate::driver::DynResourceDriver;
use crate::error::DriverFailure;
use crate::identity::{ResourceKey, StoredDesiredResource};
use crate::manager::{ManagerActorEndpoint, ResourceManagerMsg};
use crate::provider::ProviderDirectory;

/// Fixed runtime-only reconcile backoff for U3. Per-type backoff policy can
/// land with the provider conversion units; the scheduler shape (one pending
/// timer per actor, delete cancels) is already final here.
pub const DEFAULT_REQUEUE_BACKOFF: Duration = Duration::from_millis(200);

/// In-memory resource status (R11, spec section 19). Closed: the manager's
/// runtime view and the watch hub see this classification and nothing else.
/// Never persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceStatus {
    /// Actor started, no recovery yet.
    Pending,
    /// Discovery/adoption on the realization target is running (F2).
    Recovering,
    /// A reconcile pass is running or an effect is in flight.
    Reconciling,
    /// Desired state satisfied.
    Ready,
    /// The last driver operation failed (closed classification; R13 owns
    /// retry policy from the class alone).
    Failed(DriverFailure),
    /// The durable deleting mark is committed; cleanup runs (R10).
    Deleting,
}

/// The resource actor's message protocol (spec section 8).
///
/// Deviation from the spec sketch: `DependencySatisfied` carries the
/// `WatchId` instead of the `WatchCondition` - the condition lives in the
/// target actor's registration (where it is evaluated atomically, AE2); the
/// subscriber only needs to know that its dependency changed to trigger a
/// reconcile, which re-registers watches as needed (spec section 16).
#[derive(Debug)]
pub enum ResourceMsg {
    /// Start sequence (spec section 9): recover on the target, then
    /// reconcile. Deleting actors resume cleanup instead.
    Start,
    /// The durable desired spec advanced (the manager persisted the new
    /// generation BEFORE sending this, AE1): rebuild the context, reconcile.
    SpecChanged {
        generation: u64,
        spec: Vec<u8>,
        metadata: Vec<u8>,
    },
    /// One reconcile pass. Also the delivery shape of the requeue timer
    /// (spec section 32); deleting actors treat it as a deletion retry.
    Reconcile,
    /// A dependency changed (actor crash, provider failure): reconcile and
    /// re-register watches (spec section 16).
    DependencyChanged { key: ResourceKey },
    /// A watched condition was satisfied (R12). Exactly once per
    /// registration; the target actor's mailbox serialized evaluation.
    DependencySatisfied { key: ResourceKey, watch: WatchId },
    /// Internal watch registration (spec section 15). Evaluated atomically
    /// with the current status in this one mailbox handler.
    Watch {
        id: WatchId,
        condition: WatchCondition,
        subscriber: mpsc::UnboundedSender<WatchSatisfied>,
    },
    /// Drop an internal watch registration.
    Unwatch { id: WatchId },
    /// The durable deleting mark is committed (R10): cancel pending
    /// requeues, run driver delete, report completion, stop.
    Delete,
    /// A spawned long effect completed (R5, spec section 14).
    EffectCompleted {
        operation: OperationId,
        result: EffectResult,
    },
    /// The resource's target went away (R21). Observed state is unavailable;
    /// the desired row, its assignment, its children, and its target-local
    /// realization are untouched, and no requeue is scheduled: the target's
    /// own reconnect drives the retry.
    TargetUnavailable { session_generation: u64 },
    /// The guest target reconnected (F5): target-local discovery, adoption,
    /// and reconcile run again under the new session generation.
    TargetReconnected { session_generation: u64 },
}

/// Evaluate a watch condition against the actor's in-memory status (R12).
///
/// `Custom` predicates are driver-supplied; the contract hook for named
/// custom predicates lands with the provider conversion units (U6+). U3
/// treats unknown custom ids as unsatisfied - registered, never silently
/// satisfied by the wrong predicate.
fn condition_matches(condition: &WatchCondition, status: &ResourceStatus) -> bool {
    match condition {
        WatchCondition::Ready => *status == ResourceStatus::Ready,
        WatchCondition::Custom(_) => false,
    }
}

/// One registered internal watcher (spec section 15). Ephemeral: never
/// persisted; dependents re-register after a dependency change.
struct Watcher {
    condition: WatchCondition,
    subscriber: mpsc::UnboundedSender<WatchSatisfied>,
}

/// Requeue scheduler over ractor timers (R13, spec section 32): one pending
/// timer per actor, new schedules cancel the previous, cancellation is the
/// delete path's job, nothing is persisted.
struct ActorTimers {
    cell: ActorCell,
    next: AtomicU64,
    pending: Mutex<
        Option<(
            RequeueId,
            ractor::concurrency::JoinHandle<Result<(), ractor::MessagingErr<ResourceMsg>>>,
        )>,
    >,
}

impl ActorTimers {
    fn new(cell: ActorCell) -> Self {
        Self { cell, next: AtomicU64::new(1), pending: Mutex::new(None) }
    }

    /// Cancel every pending schedule (the delete path).
    fn cancel_all(&self) {
        if let Some((_, handle)) = self.pending.lock().take() {
            handle.abort();
        }
    }
}

impl RequeueScheduler for ActorTimers {
    fn schedule(&self, _key: ResourceKey, after: Duration) -> RequeueId {
        let id = RequeueId(self.next.fetch_add(1, Ordering::SeqCst));
        let handle = ractor::time::send_after(after, self.cell.clone(), || ResourceMsg::Reconcile);
        let mut pending = self.pending.lock();
        if let Some((_, old)) = pending.take() {
            old.abort();
        }
        *pending = Some((id, handle));
        id
    }

    fn cancel(&self, id: RequeueId) {
        let mut pending = self.pending.lock();
        if pending.as_ref().is_some_and(|(scheduled, _)| *scheduled == id) {
            if let Some((_, handle)) = pending.take() {
                handle.abort();
            }
        }
    }
}

/// Arguments assembled by the manager at the commit-then-spawn boundary (F1).
pub struct ResourceActorArgs {
    /// Committed durable row: the manager persisted it before spawning.
    pub row: StoredDesiredResource,
    /// Execution target for effects (R19); the coarse handle derived from
    /// `target_binding` when the manager resolved one.
    pub target: crate::target::TargetHandle,
    /// Directory-backed target binding (U13), resolved by the manager from
    /// the row's declared execution reference. `None` when the manager runs
    /// without a target directory (scaffold and unit fixtures).
    pub target_binding: Option<crate::target::TargetBinding>,
    /// Provider directory resolved in `pre_start`. A lookup failure fails
    /// the spawn AFTER the row committed (F1): the row stays durable and a
    /// restart or Ensure recovers it.
    pub providers: Arc<ProviderDirectory>,
    /// Manager mailbox: status publication and deletion completion.
    pub manager: ActorRef<crate::manager::ResourceManagerMsg>,
    /// Per-type spec decode hook (manager-wired).
    pub decoder: Arc<dyn SpecDecoder>,
    /// Runtime-only retryable-failure backoff (R13, manager-configured).
    pub backoff: Duration,
    /// The owning resource's key, resolved by the manager from the row's
    /// owner uid (R8). Drivers select their launch shape from it (the
    /// provider-controller and binding-worker ticket intents both key on the
    /// owner reference), so a child row without it cannot bind a ticket.
    pub owner_key: Option<crate::identity::ResourceKey>,
}

/// The resource actor's live state (spec section 8).
pub struct ResourceActorState {
    /// Manager mailbox for `RuntimeChanged` / `DeletionComplete`.
    manager: ActorRef<crate::manager::ResourceManagerMsg>,
    /// Manager endpoint behind the driver context (R2).
    manager_endpoint: Arc<dyn ManagerEndpoint>,
    decoder: Arc<dyn SpecDecoder>,
    target: crate::target::TargetHandle,
    /// Directory-backed target binding (U13); rebuilt with the context.
    target_binding: Option<crate::target::TargetBinding>,
    /// Runtime-only retryable-failure backoff (R13).
    backoff: Duration,
    /// The owning resource's key (see [`ResourceActorArgs::owner_key`]).
    owner_key: Option<crate::identity::ResourceKey>,
    /// Requeue timers (ractor timers, runtime-only, R13).
    timers: Arc<ActorTimers>,

    /// Durable desired row mirror; the context rebuilds on `SpecChanged`.
    row: StoredDesiredResource,
    /// In-memory status (R11). Never persisted.
    status: ResourceStatus,
    /// The durable deleting mark was committed before this flag was set.
    deleting: bool,

    /// Erased driver; typed errors stop at the classification boundary.
    driver: Box<dyn DynResourceDriver>,

    /// Driver capability surface; rebuilt on `SpecChanged`.
    ctx: ResourceContext,

    /// Registered internal watchers (R12), evaluated on every transition.
    watchers: HashMap<WatchId, Watcher>,

    /// Same-resource reconcile serialization (R14).
    effect_running: bool,
    reconcile_pending: bool,
    pending_operation: Option<OperationId>,

    /// The one pending requeue timer (R13, spec section 32).
    pending_requeue: Option<RequeueId>,

    // Effect/watch pump receivers, held until post_start spawns the pumps.
    effect_rx: Option<mpsc::UnboundedReceiver<EffectCompleted>>,
    watch_rx: Option<mpsc::UnboundedReceiver<WatchSatisfied>>,
    // Senders kept for context rebuilds on `SpecChanged`.
    effect_tx: mpsc::UnboundedSender<EffectCompleted>,
    watch_tx: mpsc::UnboundedSender<WatchSatisfied>,
}

impl ResourceActorState {
    /// Publish one status transition (R11, spec section 19): evaluate the
    /// internal watchers in the same handler and notify the manager, which
    /// feeds the watch hub. Zero persistent writes on this path (AE6). The
    /// message carries the row generation this actor holds, so a status that
    /// crosses a spec commit is never recorded as state of the newer row.
    fn transition(&mut self, status: ResourceStatus) {
        self.status = status;
        self.evaluate_watchers();
        let _ = self.manager.send_message(ResourceManagerMsg::RuntimeChanged {
            key: self.row.key.clone(),
            generation: self.row.generation,
            status,
        });
    }

    /// AE2, spec section 15: satisfy matching watchers in the transition
    /// handler and remove them (exactly once per registration).
    fn evaluate_watchers(&mut self) {
        let target = self.row.key.clone();
        let satisfied: Vec<WatchId> = self
            .watchers
            .iter()
            .filter(|(_, watcher)| condition_matches(&watcher.condition, &self.status))
            .map(|(id, _)| *id)
            .collect();
        for id in satisfied {
            if let Some(watcher) = self.watchers.remove(&id) {
                let _ =
                    watcher.subscriber.send(WatchSatisfied { watch: id, target: target.clone() });
            }
        }
    }

    /// Schedule the single runtime-only requeue (R13, spec section 32).
    fn schedule_requeue(&mut self) {
        let id = self.ctx.requeue_after(self.backoff);
        self.pending_requeue = Some(id);
    }

    /// Handle a driver failure with the actor's retry policy (R13).
    fn handle_driver_failure(&mut self, failure: DriverFailure) {
        self.transition(ResourceStatus::Failed(failure));
        if failure.class() == crate::error::FailureClass::Retryable {
            self.schedule_requeue();
        }
    }

    /// Spec section 9 start sequence: recover (discovery/adoption) then
    /// reconcile. A deleting actor resumes cleanup instead (R10, F2).
    async fn start_pass(&mut self, myself: ActorRef<ResourceMsg>) -> Result<(), ActorProcessingErr> {
        if self.deleting {
            self.transition(ResourceStatus::Deleting);
            self.delete_pass(myself).await;
            return Ok(());
        }
        self.transition(ResourceStatus::Recovering);
        // Spec section 13: `validate_spec()` folds in before recovery.
        if let Err(failure) = self.driver.validate(&mut self.ctx).await {
            self.handle_driver_failure(failure);
            return Ok(());
        }
        // Recovery reconstructs observed state from reality (R15); the
        // outcome is runtime status, never persisted.
        match self.driver.recover(&mut self.ctx).await {
            Ok(_recovered) => {
                self.transition(ResourceStatus::Reconciling);
                self.reconcile_pass().await;
            }
            Err(failure) => self.handle_driver_failure(failure),
        }
        Ok(())
    }

    /// `Reconcile` message: a reconcile pass, or (while deleting) a requeue
    /// tick driving the deletion retry.
    async fn reconcile_msg(&mut self, myself: ActorRef<ResourceMsg>) -> Result<(), ActorProcessingErr> {
        if self.deleting {
            self.delete_pass(myself).await;
            return Ok(());
        }
        if self.effect_running {
            // Same-resource serialization (R14): coalesce, never re-enter.
            self.reconcile_pending = true;
            return Ok(());
        }
        self.transition(ResourceStatus::Reconciling);
        self.reconcile_pass().await;
        Ok(())
    }

    /// Dependency change or satisfaction (spec sections 15-16): reconcile.
    async fn dependency_triggered(&mut self) -> Result<(), ActorProcessingErr> {
        if self.deleting || self.effect_running {
            self.reconcile_pending = true;
            return Ok(());
        }
        self.transition(ResourceStatus::Reconciling);
        self.reconcile_pass().await;
        Ok(())
    }

    async fn apply_spec_changed(
        &mut self,
        generation: u64,
        spec: Vec<u8>,
        metadata: Vec<u8>,
    ) -> Result<(), ActorProcessingErr> {
        if self.deleting {
            // Deletion in progress: spec churn does not resurrect the
            // resource (R10; a late Ensure on a deleting row is rejected by
            // the store anyway).
            return Ok(());
        }
        self.row.generation = generation;
        self.row.spec = spec;
        self.row.metadata = metadata;
        self.rebuild_context();
        if self.effect_running {
            self.reconcile_pending = true;
            return Ok(());
        }
        self.transition(ResourceStatus::Reconciling);
        self.reconcile_pass().await;
        Ok(())
    }

    /// Rebuild the driver context after a spec change (generation moves).
    fn rebuild_context(&mut self) {
        let ctx = ResourceContext::new(
            self.row.clone(),
            self.target,
            self.decoder.clone(),
            self.manager_endpoint.clone(),
            self.timers.clone(),
            self.effect_tx.clone(),
            self.watch_tx.clone(),
        )
        .with_owner_key(self.owner_key.clone());
        self.ctx = match self.target_binding.clone() {
            Some(binding) => ctx.with_target_binding(binding),
            None => ctx,
        };
    }

    /// One reconcile pass. Callers guarantee `!effect_running` (R14: the
    /// same resource never reconciles concurrently).
    async fn reconcile_pass(&mut self) {
        self.effect_running = true;
        self.reconcile_pending = false;
        match self.driver.reconcile(&mut self.ctx).await {
            Ok(crate::driver::ReconcileOutcome::Satisfied) => {
                self.effect_running = false;
                self.transition(ResourceStatus::Ready);
            }
            Ok(crate::driver::ReconcileOutcome::InProgress { operation }) => {
                // Long effect in flight (R5): the mailbox stays responsive;
                // completion arrives as `EffectCompleted`.
                self.pending_operation = Some(operation);
            }
            Err(failure) => {
                self.effect_running = false;
                self.handle_driver_failure(failure);
            }
        }
    }

    async fn delete_msg(&mut self, myself: ActorRef<ResourceMsg>) -> Result<(), ActorProcessingErr> {
        // Delete cancels the pending requeue (spec section 32; R10).
        self.timers.cancel_all();
        self.pending_requeue = None;
        self.deleting = true;
        self.transition(ResourceStatus::Deleting);
        self.delete_pass(myself).await;
        Ok(())
    }

    /// Teardown (R10, F3): the durable deleting mark is already committed.
    /// The drain step runs first - on every resource, not only the types the
    /// old plane gave a finalizer - and a retryable failure from either step
    /// requeues another pass. Success reports completion to the manager
    /// (which removes the row and stops this actor).
    async fn delete_pass(&mut self, myself: ActorRef<ResourceMsg>) {
        if let Err(failure) = self.driver.finalize(&mut self.ctx).await {
            self.handle_driver_failure(failure);
            return;
        }
        match self.driver.delete(&mut self.ctx).await {
            Ok(()) => {
                let _ = self.manager.send_message(ResourceManagerMsg::DeletionComplete {
                    key: self.row.key.clone(),
                });
                myself.get_cell().stop(None);
            }
            Err(failure) => self.handle_driver_failure(failure),
        }
    }

    async fn effect_completed(
        &mut self,
        operation: OperationId,
        result: crate::context::EffectResult,
        myself: ActorRef<ResourceMsg>,
    ) -> Result<(), ActorProcessingErr> {
        if self.pending_operation != Some(operation) {
            // Stale effect (spec changed mid-flight): drop it.
            return Ok(());
        }
        self.pending_operation = None;
        match result {
            crate::context::EffectResult::Completed => {
                self.effect_running = false;
                // Continue reconcile (spec section 14); the pending flag
                // coalesces any triggers that arrived while the effect ran.
                let _ = myself.send_message(ResourceMsg::Reconcile);
            }
            crate::context::EffectResult::Failed(failure) => {
                self.effect_running = false;
                self.handle_driver_failure(failure);
            }
        }
        Ok(())
    }
}

/// One authoritative actor per desired resource (R1).
///
/// Implements the plain [`ractor::Actor`] trait: U1 enabled ractor's default
/// features only (no `actor-macros`), and the trait's RPITIT methods accept
/// plain `async fn` implementations.
pub struct ResourceActor;

impl ResourceActor {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for ResourceActor {
    fn default() -> Self {
        Self::new()
    }
}

impl Actor for ResourceActor {
    type Msg = ResourceMsg;
    type State = ResourceActorState;
    type Arguments = ResourceActorArgs;

    async fn pre_start(
        &self,
        myself: ActorRef<ResourceMsg>,
        args: ResourceActorArgs,
    ) -> Result<ResourceActorState, ActorProcessingErr> {
        // Driver production runs inside the spawn boundary: a missing
        // provider fails the spawn AFTER the manager committed the row (F1
        // durability boundary) - the row stays durable and a restart or a
        // later Ensure recovers it.
        let driver = args.providers.create_driver(&args.row.key).await.map_err(|error| {
            ActorProcessingErr::from(format!("driver creation failed: {error}"))
        })?;
        let timers = Arc::new(ActorTimers::new(myself.get_cell()));
        let manager_endpoint: Arc<dyn ManagerEndpoint> =
            Arc::new(ManagerActorEndpoint::new(args.manager.clone()));
        let (effect_tx, effect_rx) = mpsc::unbounded_channel();
        let (watch_tx, watch_rx) = mpsc::unbounded_channel();
        let ctx = ResourceContext::new(
            args.row.clone(),
            args.target,
            args.decoder.clone(),
            manager_endpoint.clone(),
            timers.clone(),
            effect_tx.clone(),
            watch_tx.clone(),
        )
        .with_owner_key(args.owner_key.clone());
        let ctx = match args.target_binding.clone() {
            Some(binding) => ctx.with_target_binding(binding),
            None => ctx,
        };
        Ok(ResourceActorState {
            manager: args.manager,
            manager_endpoint,
            decoder: args.decoder,
            target: args.target,
            target_binding: args.target_binding,
            backoff: args.backoff,
            owner_key: args.owner_key,
            timers,
            row: args.row.clone(),
            status: ResourceStatus::Pending,
            deleting: args.row.deleting,
            driver,
            ctx,
            watchers: HashMap::new(),
            effect_running: false,
            reconcile_pending: false,
            pending_operation: None,
            pending_requeue: None,
            effect_rx: Some(effect_rx),
            watch_rx: Some(watch_rx),
            effect_tx,
            watch_tx,
        })
    }

    async fn post_start(
        &self,
        myself: ActorRef<ResourceMsg>,
        state: &mut ResourceActorState,
    ) -> Result<(), ActorProcessingErr> {
        // Long effects and watch satisfactions arrive on unbounded channels
        // and are forwarded into the mailbox as typed messages (R5, R12);
        // the mailbox never blocks on external work (KTD12).
        let effects = state.effect_rx.take().expect("effect receiver held for the pump");
        spawn_pump(myself.clone(), effects, |effect| ResourceMsg::EffectCompleted {
            operation: effect.operation,
            result: effect.result,
        });
        let watches = state.watch_rx.take().expect("watch receiver held for the pump");
        spawn_pump(myself.clone(), watches, |satisfied| {
            ResourceMsg::DependencySatisfied { key: satisfied.target, watch: satisfied.watch }
        });
        // Spec section 9: the start sequence runs through the mailbox so a
        // supervisor sees start failures like any other failure.
        let _ = myself.send_message(ResourceMsg::Start);
        Ok(())
    }

    async fn handle(
        &self,
        myself: ActorRef<ResourceMsg>,
        message: ResourceMsg,
        state: &mut ResourceActorState,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            ResourceMsg::Start => state.start_pass(myself).await,
            ResourceMsg::SpecChanged { generation, spec, metadata } => {
                state.apply_spec_changed(generation, spec, metadata).await
            }
            ResourceMsg::Reconcile => state.reconcile_msg(myself).await,
            ResourceMsg::DependencyChanged { .. } | ResourceMsg::DependencySatisfied { .. } => {
                state.dependency_triggered().await
            }
            ResourceMsg::Watch { id, condition, subscriber } => {
                // ATOMICITY INVARIANT (AE2, R12): evaluate and register in
                // this one handler. Condition already true: notify now.
                // Otherwise insert; transitions re-evaluate in their handler.
                if condition_matches(&condition, &state.status) {
                    let _ = subscriber.send(WatchSatisfied {
                        watch: id,
                        target: state.row.key.clone(),
                    });
                } else {
                    state.watchers.insert(id, Watcher { condition, subscriber });
                }
                Ok(())
            }
            ResourceMsg::Unwatch { id } => {
                state.watchers.remove(&id);
                Ok(())
            }
            ResourceMsg::Delete => state.delete_msg(myself).await,
            ResourceMsg::EffectCompleted { operation, result } => {
                state.effect_completed(operation, result, myself).await
            }
            ResourceMsg::TargetUnavailable { .. } => {
                // Target failure is not Zone failure (R21): only the
                // target-dependent observed state changes.
                if !state.deleting {
                    state.transition(ResourceStatus::Failed(crate::error::DriverFailure::retryable(
                        crate::error::DriverOp::Recover,
                    )));
                }
                Ok(())
            }
            ResourceMsg::TargetReconnected { .. } => {
                if state.deleting {
                    Ok(())
                } else {
                    state.dependency_triggered().await
                }
            }
        }
    }
}

/// Forwarding pump: typed context-surface values (long-effect completions,
/// internal-watch satisfactions) become mailbox messages (R5, R12). Ends
/// when the actor terminates and the sender side drops.
fn spawn_pump<T>(
    myself: ActorRef<ResourceMsg>,
    mut receiver: mpsc::UnboundedReceiver<T>,
    translate: impl Fn(T) -> ResourceMsg + Send + 'static,
) where
    T: Send + 'static,
{
    tokio::spawn(async move {
        while let Some(value) = receiver.recv().await {
            if myself.send_message(translate(value)).is_err() {
                break;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Tests: shared fake driver/factory plus manager harness. The manager-level
// invariant tests live in manager.rs and exercise this actor end to end.
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use parking_lot::Mutex;

    use crate::context::{EffectCompleted, EffectResult, ResourceContext, WatchCondition};
    use crate::driver::{DynResourceDriver, ResourceDriver, ResourceDriverFactory};
    use crate::driver::{ReconcileOutcome, RecoveryOutcome};
    use crate::error::{DriverFailure, DriverOp};
    use crate::identity::{ResourceKey, ResourceTypeName};

    /// Scripted driver error; classification is always retryable (the actor
    /// owns the retry decision).
    #[derive(Debug, thiserror::Error)]
    #[error("fake driver failure")]
    pub(crate) struct FakeDriverError;

    /// How the fake driver's `reconcile` behaves.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum ReconcileMode {
        /// Desired state satisfied immediately.
        Satisfied,
        /// `reconcile` awaits the gate inside the call (blocks the mailbox;
        /// proves cross-resource concurrency).
        BlockedInline,
        /// First pass spawns a gated long effect and returns `InProgress`
        /// (mailbox free); every later pass is satisfied (the driver
        /// re-assesses after the effect completes). Proves same-resource
        /// serialization through the mailbox without looping on an open
        /// gate.
        GatedEffectOnce,
        /// Panic on the first invocation, then `Satisfied` (actor crash).
        PanicOnce,
        /// Fail retryably (requeue path, R13).
        FailRetryable,
    }

    /// One live read a fake driver performed through its context
    /// (`ResourceContext::get_view`), carrying the manager's answer verbatim.
    #[derive(Debug, Clone)]
    pub(crate) struct FakeViewRead {
        pub(crate) key: ResourceKey,
        /// `None` = the manager reported no row for the key.
        pub(crate) view: Option<crate::manager::ResourceView>,
    }

    /// Shared, test-drivable state of one fake driver (per resource key).
    pub(crate) struct FakeDriverShared {
        pub(crate) validate_calls: AtomicU64,
        pub(crate) recover_calls: AtomicU64,
        pub(crate) reconcile_calls: AtomicU64,
        pub(crate) watch_calls: AtomicU64,
        pub(crate) delete_calls: AtomicU64,
        /// Driver-body finalize (drain) invocations: the erased boundary's
        /// owned-children step runs first, so this only advances when the
        /// pass reaches the driver's own drain.
        pub(crate) finalize_calls: AtomicU64,
        pub(crate) active_reconcile: AtomicU64,
        pub(crate) max_concurrent_reconcile: AtomicU64,
        pub(crate) reconcile_mode: Mutex<ReconcileMode>,
        pub(crate) gate_open: AtomicBool,
        pub(crate) gate: tokio::sync::Notify,
        pub(crate) delete_blocked: AtomicBool,
        /// When set, every reconcile registers an internal watch on this
        /// target first (models a dependent resource).
        pub(crate) watch_target: Mutex<Option<ResourceKey>>,
        /// Keys every reconcile reads live through its context
        /// (`ResourceContext::get_view`), in order (models a parent proving a
        /// child, or a dependent proving a dependency).
        pub(crate) view_targets: Mutex<Vec<ResourceKey>>,
        /// Live reads performed through the context, in order.
        pub(crate) view_reads: Mutex<Vec<FakeViewRead>>,
        /// Generations observed by `reconcile` (spec change delivery).
        pub(crate) generations_seen: Mutex<Vec<u64>>,
    }

    impl FakeDriverShared {
        pub(crate) fn new() -> Self {
            Self {
                validate_calls: AtomicU64::new(0),
                recover_calls: AtomicU64::new(0),
                reconcile_calls: AtomicU64::new(0),
                watch_calls: AtomicU64::new(0),
                delete_calls: AtomicU64::new(0),
                finalize_calls: AtomicU64::new(0),
                active_reconcile: AtomicU64::new(0),
                max_concurrent_reconcile: AtomicU64::new(0),
                reconcile_mode: Mutex::new(ReconcileMode::Satisfied),
                gate_open: AtomicBool::new(false),
                gate: tokio::sync::Notify::new(),
                delete_blocked: AtomicBool::new(false),
                watch_target: Mutex::new(None),
                view_targets: Mutex::new(Vec::new()),
                view_reads: Mutex::new(Vec::new()),
                generations_seen: Mutex::new(Vec::new()),
            }
        }

        /// Release a gated reconcile/delete.
        pub(crate) fn open_gate(&self) {
            self.gate_open.store(true, Ordering::Release);
            // notify_one stores a permit when no waiter is registered yet,
            // so a waiter that starts after the open still wakes (the
            // loop re-checks the flag on wake).
            self.gate.notify_one();
        }

        async fn wait_gate(&self) {
            loop {
                if self.gate_open.load(Ordering::Acquire) {
                    return;
                }
                self.gate.notified().await;
            }
        }

        fn enter_reconcile(&self) {
            let entered = self.active_reconcile.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_concurrent_reconcile.fetch_max(entered, Ordering::SeqCst);
        }

        fn exit_reconcile(&self) {
            self.active_reconcile.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Fake driver per resource key, sharing its [`FakeDriverShared`].
    pub(crate) struct FakeDriver {
        shared: Arc<FakeDriverShared>,
    }

    #[async_trait]
    impl ResourceDriver for FakeDriver {
        type Error = FakeDriverError;

        fn classify_error(&self, _error: &Self::Error) -> DriverFailure {
            DriverFailure::retryable(DriverOp::Reconcile)
        }

        async fn validate(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
            self.shared.validate_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn recover(
            &mut self,
            _ctx: &mut ResourceContext,
        ) -> Result<RecoveryOutcome, Self::Error> {
            self.shared.recover_calls.fetch_add(1, Ordering::SeqCst);
            Ok(RecoveryOutcome::Adopted)
        }

        async fn reconcile(
            &mut self,
            ctx: &mut ResourceContext,
        ) -> Result<ReconcileOutcome, Self::Error> {
            self.shared.reconcile_calls.fetch_add(1, Ordering::SeqCst);
            self.shared.generations_seen.lock().push(ctx.generation());
            // A dependent registers its internal watch in every reconcile
            // (spec sections 15-16): exactly-once delivery is the target
            // actor's contract, re-registration is the dependent's.
            let watch_target = self.shared.watch_target.lock().clone();
            if let Some(target) = watch_target {
                self.shared.watch_calls.fetch_add(1, Ordering::SeqCst);
                let _watch = ctx.watch(target, WatchCondition::Ready).await;
            }
            // Live state of other resources (KTD3): each reconcile reads the
            // configured keys through the context and records the answer.
            let view_targets = self.shared.view_targets.lock().clone();
            for target in view_targets {
                let view = ctx.get_view(&target).await.expect("live view read");
                self.shared.view_reads.lock().push(FakeViewRead { key: target, view });
            }
            let mode = *self.shared.reconcile_mode.lock();
            match mode {
                ReconcileMode::Satisfied => Ok(ReconcileOutcome::Satisfied),
                ReconcileMode::PanicOnce => {
                    *self.shared.reconcile_mode.lock() = ReconcileMode::Satisfied;
                    panic!("fake reconcile crash");
                }
                ReconcileMode::FailRetryable => Err(FakeDriverError),
                ReconcileMode::BlockedInline => {
                    self.shared.enter_reconcile();
                    self.shared.wait_gate().await;
                    self.shared.exit_reconcile();
                    Ok(ReconcileOutcome::Satisfied)
                }
                ReconcileMode::GatedEffectOnce => {
                    self.shared.enter_reconcile();
                    if self.shared.reconcile_calls.load(Ordering::SeqCst) > 1 {
                        // The driver re-assessed after the effect: satisfied.
                        self.shared.exit_reconcile();
                        return Ok(ReconcileOutcome::Satisfied);
                    }
                    let operation = ctx.begin_operation();
                    let sender = ctx.effect_sender();
                    let shared = self.shared.clone();
                    tokio::spawn(async move {
                        shared.wait_gate().await;
                        shared.exit_reconcile();
                        let _ = sender.send(EffectCompleted {
                            operation,
                            result: EffectResult::Completed,
                        });
                    });
                    Ok(ReconcileOutcome::InProgress { operation })
                }
            }
        }

        async fn finalize(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
            self.shared.finalize_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
            self.shared.delete_calls.fetch_add(1, Ordering::SeqCst);
            if self.shared.delete_blocked.load(Ordering::SeqCst) {
                self.shared.wait_gate().await;
            }
            Ok(())
        }
    }

    /// Factory producing [`FakeDriver`]s; the registry exposes each key's
    /// shared state so tests can configure and observe behavior.
    pub(crate) struct FakeFactory {
        types: Vec<ResourceTypeName>,
        registry: Mutex<HashMap<ResourceKey, Arc<FakeDriverShared>>>,
    }

    impl FakeFactory {
        pub(crate) fn new(types: &[&str]) -> Self {
            Self {
                types: types.iter().map(|t| ResourceTypeName::new(*t)).collect(),
                registry: Mutex::new(HashMap::new()),
            }
        }

        /// The shared driver state for a key (created on first use).
        pub(crate) fn shared(&self, key: &ResourceKey) -> Arc<FakeDriverShared> {
            self.registry.lock().entry(key.clone()).or_insert_with(|| Arc::new(FakeDriverShared::new())).clone()
        }

    }

    #[async_trait]
    impl ResourceDriverFactory for FakeFactory {
        fn resource_types(&self) -> &[ResourceTypeName] {
            &self.types
        }

        async fn create(&self, key: &ResourceKey) -> Box<dyn DynResourceDriver> {
            Box::new(FakeDriver { shared: self.shared(key) })
        }
    }

    /// Spec decode hook treating the envelope as opaque bytes: tests pass
    /// spec payloads as raw byte strings and drivers see them unchanged.
    pub(crate) struct PassthroughDecoder;

    impl crate::context::SpecDecoder for PassthroughDecoder {
        fn decode(
            &self,
            envelope: &[u8],
        ) -> Result<Box<dyn std::any::Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(Box::new(envelope.to_vec()))
        }
    }

    /// Running manager plus its handles for assertions.
    pub(crate) struct TestHarness {
        pub(crate) client: crate::manager::ResourceManagerClient,
        pub(crate) factory: Arc<FakeFactory>,
        pub(crate) store: Arc<crate::spec_store::SpecStore>,
        pub(crate) hub: Arc<crate::watch::WatchHub>,
        /// Owns the store's directory for the lifetime of the harness.
        _tmp: Option<tempfile::TempDir>,
    }

    impl Drop for TestHarness {
        fn drop(&mut self) {
            let _ = self.client.actor().get_cell().stop(None);
        }
    }

    /// Spawn a manager over a fresh store with a fake factory covering
    /// `types`.
    pub(crate) async fn harness(types: &[&str]) -> TestHarness {
        harness_with(types, crate::resource::DEFAULT_REQUEUE_BACKOFF).await
    }

    /// Spawn a manager with an explicit retry backoff (timer tests).
    pub(crate) async fn harness_with(types: &[&str], backoff: Duration) -> TestHarness {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            crate::spec_store::SpecStore::open(tmp.path().join("specs.sqlite")).expect("store"),
        );
        spawn_manager(store, "test", types, backoff, Some(tmp)).await
    }

    /// Spawn a manager with a scripted target resolver and directory (U13
    /// wiring tests): the caller owns the directory and can drive guest
    /// session loss and reconnect on it.
    pub(crate) async fn harness_targeted(
        types: &[&str],
        resolver: Arc<dyn crate::target::TargetResolver>,
        targets: Arc<crate::target::TargetDirectory>,
    ) -> TestHarness {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            crate::spec_store::SpecStore::open(tmp.path().join("specs.sqlite")).expect("store"),
        );
        spawn_manager_targeted(
            store,
            "test",
            types,
            crate::resource::DEFAULT_REQUEUE_BACKOFF,
            Some(tmp),
            resolver,
            targets,
        )
        .await
    }

    /// Spawn a manager over an existing store (restart semantics: the new
    /// manager loads the durable rows and spawns actors for them).
    pub(crate) async fn harness_over(
        store: Arc<crate::spec_store::SpecStore>,
        zone: &str,
        types: &[&str],
        backoff: Duration,
    ) -> TestHarness {
        spawn_manager(store, zone, types, backoff, None).await
    }

    /// Restart semantics with the caller's factory: restart tests can pin
    /// driver behavior (e.g. blocked deletes) on keys before the manager
    /// loads rows and spawns their actors.
    pub(crate) async fn harness_over_with_factory(
        store: Arc<crate::spec_store::SpecStore>,
        zone: &str,
        factory: Arc<FakeFactory>,
        backoff: Duration,
    ) -> TestHarness {
        spawn_manager_with_factory(store, zone, factory, backoff, None).await
    }

    async fn spawn_manager(
        store: Arc<crate::spec_store::SpecStore>,
        zone: &str,
        types: &[&str],
        backoff: Duration,
        tmp: Option<tempfile::TempDir>,
    ) -> TestHarness {
        let factory = Arc::new(FakeFactory::new(types));
        spawn_manager_with_factory(store, zone, factory, backoff, tmp).await
    }

    /// Manager spawn with the target layer under test control (U13).
    async fn spawn_manager_targeted(
        store: Arc<crate::spec_store::SpecStore>,
        zone: &str,
        types: &[&str],
        backoff: Duration,
        tmp: Option<tempfile::TempDir>,
        target_resolver: Arc<dyn crate::target::TargetResolver>,
        targets: Arc<crate::target::TargetDirectory>,
    ) -> TestHarness {
        let factory = Arc::new(FakeFactory::new(types));
        spawn_manager_with(store, zone, factory, backoff, tmp, target_resolver, targets).await
    }

    async fn spawn_manager_with_factory(
        store: Arc<crate::spec_store::SpecStore>,
        zone: &str,
        factory: Arc<FakeFactory>,
        backoff: Duration,
        tmp: Option<tempfile::TempDir>,
    ) -> TestHarness {
        spawn_manager_with(
            store,
            zone,
            factory,
            backoff,
            tmp,
            Arc::new(HostOnlyResolver),
            Arc::new(crate::target::TargetDirectory::new()),
        )
        .await
    }

    /// One manager spawn over one target directory and resolver.
    async fn spawn_manager_with(
        store: Arc<crate::spec_store::SpecStore>,
        zone: &str,
        factory: Arc<FakeFactory>,
        backoff: Duration,
        tmp: Option<tempfile::TempDir>,
        target_resolver: Arc<dyn crate::target::TargetResolver>,
        targets: Arc<crate::target::TargetDirectory>,
    ) -> TestHarness {
        let mut providers = crate::provider::ProviderDirectory::new();
        providers
            .register(factory.clone() as Arc<dyn ResourceDriverFactory>)
            .expect("factory registration");
        let hub = Arc::new(crate::watch::WatchHub::new(
            &crate::revision::SystemClock,
            crate::watch::DEFAULT_RING_CAPACITY,
        ));
        let args = crate::manager::ResourceManagerArgs {
            zone: zone.to_string(),
            store: store.clone(),
            providers,
            hub: hub.clone(),
            admission: Arc::new(crate::manager::AllowAll),
            decoders: HashMap::new(),
            default_decoder: Arc::new(PassthroughDecoder),
            targets,
            host_target: crate::target::TargetRef::host("test-host").expect("host target"),
            target_resolver,
            backoff,
        };
        let (actor, _join) =
            ractor::Actor::spawn(None, crate::manager::ResourceManager::new(), args)
                .await
                .expect("manager spawn");
        TestHarness {
            client: crate::manager::ResourceManagerClient::new(actor),
            factory,
            store,
            hub,
            _tmp: tmp,
        }
    }

    /// Test resolver: no fixture row declares an execution reference, so
    /// every test resource realizes on the Host target.
    pub(crate) struct HostOnlyResolver;

    impl crate::target::TargetResolver for HostOnlyResolver {
        fn execution_ref(&self, _key: &ResourceKey, _spec: &[u8]) -> Option<String> {
            None
        }
    }

    /// Test resolver marking the `guest-worker` fixture row as guest-targeted.
    pub(crate) struct ScriptedResolver;

    impl crate::target::TargetResolver for ScriptedResolver {
        fn execution_ref(&self, key: &ResourceKey, _spec: &[u8]) -> Option<String> {
            (key.name == "guest-worker").then(|| "Guest/test-vm".to_owned())
        }
    }

    /// Wait until `check` holds, polling on the test runtime.
    pub(crate) async fn until(mut check: impl FnMut() -> bool) {
        for _ in 0..500 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("condition not reached within the test timeout");
    }

    /// Wait until the durable row for `key` is gone.
    pub(crate) async fn wait_row_gone(
        client: &crate::manager::ResourceManagerClient,
        key: &crate::identity::ResourceKey,
    ) {
        for _ in 0..500 {
            if client.get_row(key.clone()).await.expect("get_row").is_none() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("row not removed within the test timeout");
    }

    /// Wait until the runtime view reports `status` for `key`.
    pub(crate) async fn wait_status(
        client: &crate::manager::ResourceManagerClient,
        key: &crate::identity::ResourceKey,
        status: crate::resource::ResourceStatus,
    ) {
        for _ in 0..500 {
            if let Ok(Some(view)) = client.get(key.clone()).await {
                if view.status == Some(status) {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("status not reached within the test timeout");
    }

    pub(crate) fn key(zone: &str, type_name: &str, name: &str) -> ResourceKey {
        ResourceKey::new(zone, type_name, name)
    }

    pub(crate) fn desired(
        type_name: &str,
        name: &str,
        spec: &[u8],
    ) -> crate::manager::DesiredResource {
        crate::manager::DesiredResource {
            key: key("test", type_name, name),
            spec: spec.to_vec(),
            metadata: Vec::new(),
            provenance: crate::identity::ResourceProvenance::Api,
        }
    }

    pub(crate) fn subject() -> crate::manager::MutationSubject {
        crate::manager::MutationSubject {
            principal: "test".to_string(),
            origin: crate::identity::ResourceProvenance::Api,
        }
    }
}