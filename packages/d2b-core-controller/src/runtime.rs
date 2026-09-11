//! Core-to-toolkit source and reconciler adapters.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use d2b_contracts_resource::v3::ZoneRevision;
use d2b_contracts_zone_session::v3::ZoneStatusResource;
use d2b_controller_toolkit::{
    CommitOutcome, ControllerDescriptor, ControllerSource, DependencySnapshot, FreshSnapshot,
    InitialList, OperationContext, ReconcileContext, ReconcilePlan, ReconcileProjection,
    ReconcileResult, ResourceKey, ResourceSnapshot, SourceError, StatusPersistence, WatchEvent,
    WatchFailure,
};

use crate::{
    ChangeRecord, ControllerHint, ControllerLeaseKey, FairAdmission, HintAdmissionError,
    HintAdmissionOutcome, SuppressionDecision, WatchPlan,
};
use crate::providers::ProviderObservation;

fn resource_field(key: &ResourceKey) -> String {
    key.resource_ref().to_canonical_string()
}

fn controller_field(controller: &ControllerLeaseKey) -> String {
    controller.controller_ref().to_canonical_string()
}

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

/// Closed result of dispatching a store-watch change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreDispatchOutcome {
    Suppressed(SuppressionDecision),
    Admitted,
    Coalesced,
}

/// Cardinality-safe Core admission counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoreAdmissionCounts {
    pub admitted: usize,
    pub coalesced: usize,
    pub backpressure: usize,
}

fn validate_watch_plan(descriptor: &ControllerDescriptor) -> bool {
    WatchPlan::new(
        descriptor.resource_types().cloned().collect(),
        descriptor.watch_selectors().to_vec(),
        descriptor.consumes_owner_triggers(),
    )
    .is_ok()
}

/// Registered resource/store-watch operations available to one controller.
///
/// Implementations are trusted adapters over the production resource plane.
/// Outcome and checkpoint writes must be durable and revision-idempotent.
pub trait RegisteredControllerApi: Send + Sync + 'static {
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

    /// Stop a live adapter watch without dropping an admitted Core hint.
    fn stop_watch(&self) {}

    /// Whether `receive_watch_change` is backed by a live store stream.
    ///
    /// Test-only adapters can leave this disabled and inject changes through
    /// [`CoreControllerSource::dispatch_change`].
    fn has_watch_stream(&self) -> bool {
        false
    }

    /// Receive one raw store change for Core-owned validation and admission.
    ///
    /// The Core source applies suppression, lease, coalescing, and fair queue
    /// policy. `None` is a clean stream close; recoverable stream failures are
    /// returned as the toolkit's typed watch failure.
    fn receive_watch_change(
        &self,
    ) -> impl Future<Output = Result<Option<(ChangeRecord, OperationContext)>, WatchFailure>> + Send
    {
        std::future::ready(Err(WatchFailure::Fatal))
    }

    fn read_fresh(
        &self,
        key: &ResourceKey,
    ) -> impl Future<Output = Result<FreshSnapshot, SourceError>> + Send;

    fn write_starting(
        &self,
        context: &ReconcileContext,
    ) -> impl Future<Output = Result<(), SourceError>> + Send;

    fn accept_effect(
        &self,
        _context: &ReconcileContext,
        _plan: &ReconcilePlan,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(Ok(()))
    }

    /// Return the durable effect operation accepted for one pass.
    ///
    /// Core forwards this identity so Runner persistence retries never use a
    /// transient watch operation as ledger authority.
    fn accepted_effect_operation(
        &self,
        _context: &ReconcileContext,
    ) -> impl Future<Output = Result<Option<OperationContext>, SourceError>> + Send {
        std::future::ready(Ok(None))
    }

    fn complete_effect(
        &self,
        _context: &ReconcileContext,
        _result: &ReconcileResult,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        std::future::ready(Ok(()))
    }

    fn verify_expedited_commit(
        &self,
        _context: &ReconcileContext,
    ) -> impl Future<Output = Result<bool, SourceError>> + Send {
        std::future::ready(Ok(false))
    }

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

    fn persist_outcome(
        &self,
        projection: &ReconcileProjection,
    ) -> impl Future<Output = Result<(), SourceError>> + Send;

    fn persist_outcome_with_operation(
        &self,
        projection: &ReconcileProjection,
        _operation: &OperationContext,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.persist_outcome(projection)
    }

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

struct WatchState {
    admission: FairAdmission,
    operations: BTreeMap<(ControllerLeaseKey, ResourceKey), (ZoneRevision, OperationContext)>,
    closed: bool,
}

/// Core adapter over a registered resource API and bounded store-watch queue.
pub struct CoreControllerSource<A> {
    descriptor: ControllerDescriptor,
    controller: ControllerLeaseKey,
    api: Arc<A>,
    watch: Mutex<WatchState>,
    watch_signal_tx: tokio::sync::mpsc::Sender<()>,
    watch_signal_rx: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<()>>,
    pending_change: Mutex<Option<(ChangeRecord, OperationContext)>>,
    watch_stream_enabled: AtomicBool,
    admitted: AtomicUsize,
    coalesced: AtomicUsize,
    backpressure: AtomicUsize,
}

impl<A> CoreControllerSource<A>
where
    A: RegisteredControllerApi,
{
    /// Bind one complete descriptor to its registered resource API.
    pub fn new(descriptor: ControllerDescriptor, api: Arc<A>) -> Arc<Self> {
        let controller = ControllerLeaseKey::new(
            descriptor.identity().zone().clone(),
            descriptor.identity().controller_ref().clone(),
        )
        .expect("validated descriptor has a valid controller lease key");
        let pending_bound = descriptor.max_pending_resources();
        let (watch_signal_tx, watch_signal_rx) = tokio::sync::mpsc::channel(1);
        Arc::new(Self {
            descriptor,
            controller,
            api,
            watch: Mutex::new(WatchState {
                admission: FairAdmission::new(pending_bound, pending_bound),
                operations: BTreeMap::new(),
                closed: false,
            }),
            watch_signal_tx,
            watch_signal_rx: tokio::sync::Mutex::new(watch_signal_rx),
            pending_change: Mutex::new(None),
            watch_stream_enabled: AtomicBool::new(false),
            admitted: AtomicUsize::new(0),
            coalesced: AtomicUsize::new(0),
            backpressure: AtomicUsize::new(0),
        })
    }

    /// Apply suppression and admit one canonical toolkit hint.
    pub fn dispatch_change(
        &self,
        controller: ControllerLeaseKey,
        change: ChangeRecord,
        operation: OperationContext,
    ) -> Result<CoreDispatchOutcome, CoreSourceError> {
        let decision = change.suppression();
        if decision != SuppressionDecision::Dispatch {
            tracing::debug!(
                zone = change.target.zone().as_str(),
                resource = resource_field(&change.target),
                decision = ?decision,
                "change suppressed by convergence policy",
            );
            return Ok(CoreDispatchOutcome::Suppressed(decision));
        }
        if controller != self.controller {
            tracing::warn!(
                zone = controller.zone().as_str(),
                controller = controller_field(&controller),
                resource = resource_field(&change.target),
                "hint rejected: change routed to a foreign controller",
            );
            return Err(CoreSourceError::Hint(HintAdmissionError::InvalidHint));
        }
        if !self
            .descriptor
            .resource_types()
            .any(|resource_type| resource_type == change.target.resource_ref().resource_type())
        {
            tracing::warn!(
                zone = controller.zone().as_str(),
                resource = resource_field(&change.target),
                "hint rejected: target ResourceType is not owned by this controller",
            );
            return Err(CoreSourceError::Hint(HintAdmissionError::InvalidHint));
        }
        let target = change.target.clone();
        let revision = change.revision;
        let hint = ControllerHint::new(controller, change.target, change.revision, change.reasons)
            .map_err(|error| {
                tracing::warn!(
                    zone = target.zone().as_str(),
                    resource = resource_field(&target),
                    reason = %error,
                    "hint rejected: hint construction failed",
                );
                CoreSourceError::Hint(error)
            })?;
        self.admit_hint(hint, target, revision, operation)
    }

    /// Wake one exact resource for a declared liveness observation.
    ///
    /// This path is not a store mutation and therefore bypasses convergence
    /// suppression while still using the same bounded Core admission queue.
    pub fn dispatch_observation(
        &self,
        target: ResourceKey,
        revision: ZoneRevision,
    ) -> Result<CoreDispatchOutcome, CoreSourceError> {
        if revision.get() == 0
            || target.zone() != self.controller.zone()
            || !self
                .descriptor
                .resource_types()
                .any(|resource_type| resource_type == target.resource_ref().resource_type())
        {
            tracing::warn!(
                zone = target.zone().as_str(),
                resource = resource_field(&target),
                "observation wakeup rejected: revision, zone, or type invalid",
            );
            return Err(CoreSourceError::Hint(HintAdmissionError::InvalidHint));
        }
        let operation_suffix = format!(
            "{}:{}:{}",
            target.zone().as_str(),
            target.resource_ref().to_canonical_string(),
            target.uid().as_str(),
        );
        let operation = OperationContext::new(
            format!("process-observe:{operation_suffix}"),
            format!("process-observe:{operation_suffix}"),
            format!("process-observe:{operation_suffix}"),
            None,
        )
        .map_err(|error| {
            tracing::warn!(
                zone = target.zone().as_str(),
                resource = resource_field(&target),
                reason = %error,
                "observation wakeup rejected: operation context construction failed",
            );
            CoreSourceError::Hint(HintAdmissionError::InvalidHint)
        })?;
        let hint = ControllerHint::new(
            self.controller.clone(),
            target.clone(),
            revision,
            BTreeSet::from([d2b_controller_toolkit::TriggerReason::ScheduledObserve]),
        )
        .map_err(|error| {
            tracing::warn!(
                zone = target.zone().as_str(),
                resource = resource_field(&target),
                reason = %error,
                "observation wakeup rejected: hint construction failed",
            );
            CoreSourceError::Hint(error)
        })?;
        self.admit_hint(hint, target, revision, operation)
    }

    fn admit_hint(
        &self,
        hint: ControllerHint,
        target: ResourceKey,
        revision: ZoneRevision,
        operation: OperationContext,
    ) -> Result<CoreDispatchOutcome, CoreSourceError> {
        let key = (self.controller.clone(), target);
        let outcome = {
            let mut watch = self
                .watch
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if watch.closed {
                tracing::warn!(
                    zone = self.controller.zone().as_str(),
                    controller = controller_field(&self.controller),
                    resource = resource_field(&key.1),
                    "hint dropped: controller watch is closed",
                );
                return Err(CoreSourceError::WatchClosed);
            }
            match watch.admission.push(hint) {
                Ok(HintAdmissionOutcome::Admitted) => {
                    watch.operations.insert(key, (revision, operation));
                    HintAdmissionOutcome::Admitted
                }
                Ok(HintAdmissionOutcome::Coalesced) => {
                    let entry = watch
                        .operations
                        .get_mut(&key)
                        .expect("coalesced hint has matching operation state");
                    if revision >= entry.0 {
                        *entry = (revision, operation);
                    }
                    HintAdmissionOutcome::Coalesced
                }
                Err(error) => {
                    if error == HintAdmissionError::Backpressure {
                        self.backpressure.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            zone = self.controller.zone().as_str(),
                            controller = controller_field(&self.controller),
                            resource = resource_field(&key.1),
                            "hint dropped under fair-queue backpressure",
                        );
                    }
                    return Err(CoreSourceError::Hint(error));
                }
            }
        };
        let dispatch = match outcome {
            HintAdmissionOutcome::Admitted => {
                self.admitted.fetch_add(1, Ordering::Relaxed);
                CoreDispatchOutcome::Admitted
            }
            HintAdmissionOutcome::Coalesced => {
                self.coalesced.fetch_add(1, Ordering::Relaxed);
                CoreDispatchOutcome::Coalesced
            }
        };
        match self.watch_signal_tx.try_send(()) {
            Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(())) => Ok(dispatch),
            Err(tokio::sync::mpsc::error::TrySendError::Closed(())) => {
                tracing::error!(
                    zone = self.controller.zone().as_str(),
                    controller = controller_field(&self.controller),
                    "controller watch signal channel closed; watch is dead",
                );
                Err(CoreSourceError::WatchClosed)
            }
        }
    }

    /// Close the watch after all bounded admitted changes drain.
    pub fn close_watch(&self) -> Result<(), CoreSourceError> {
        self.api.stop_watch();
        self.watch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .closed = true;
        self.watch_stream_enabled.store(false, Ordering::Release);
        match self.watch_signal_tx.try_send(()) {
            Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(())) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Closed(())) => {
                tracing::warn!(
                    zone = self.controller.zone().as_str(),
                    controller = controller_field(&self.controller),
                    "watch close signal undeliverable; signal channel already closed",
                );
                Err(CoreSourceError::WatchClosed)
            }
        }
    }

    /// Snapshot admission, coalescing, and backpressure counters.
    pub fn admission_counts(&self) -> CoreAdmissionCounts {
        CoreAdmissionCounts {
            admitted: self.admitted.load(Ordering::Relaxed),
            coalesced: self.coalesced.load(Ordering::Relaxed),
            backpressure: self.backpressure.load(Ordering::Relaxed),
        }
    }

    /// Borrow the Zone owned by this source.
    pub const fn zone(&self) -> &d2b_contracts_resource::v3::ZoneId {
        self.controller.zone()
    }
}

impl<A> ControllerSource for CoreControllerSource<A>
where
    A: RegisteredControllerApi,
{
    fn register(
        &self,
        descriptor: &ControllerDescriptor,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        let valid = descriptor.same_routing(&self.descriptor) && validate_watch_plan(descriptor);
        async move {
            if !valid {
                return Err(SourceError::Integrity);
            }
            self.api.register(descriptor).await
        }
    }

    fn list_initial(
        &self,
        descriptor: &ControllerDescriptor,
    ) -> impl Future<Output = Result<InitialList, SourceError>> + Send {
        let valid = descriptor.same_routing(&self.descriptor) && validate_watch_plan(descriptor);
        let future = self.api.list_initial(descriptor);
        async move {
            if !valid {
                return Err(SourceError::Integrity);
            }
            future.await
        }
    }

    fn open_watch(
        &self,
        descriptor: &ControllerDescriptor,
        after_revision: ZoneRevision,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        let valid = descriptor.same_routing(&self.descriptor) && validate_watch_plan(descriptor);
        let future = self.api.open_watch(descriptor, after_revision);
        async move {
            if !valid {
                return Err(SourceError::Integrity);
            }
            let result = future.await;
            if result.is_ok() && self.api.has_watch_stream() {
                self.watch_stream_enabled.store(true, Ordering::Release);
            }
            result
        }
    }

    async fn receive_watch(&self) -> Result<WatchEvent, WatchFailure> {
        loop {
            let event = {
                let mut watch = self
                    .watch
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(hint) = watch.admission.pop() {
                    let key = (hint.controller().clone(), hint.target().clone());
                    let Some((_, operation)) = watch.operations.remove(&key) else {
                        tracing::error!(
                            zone = hint.controller().zone().as_str(),
                            controller = controller_field(hint.controller()),
                            resource = resource_field(hint.target()),
                            "admitted hint has no operation state; escalating to fatal watch failure",
                        );
                        return Err(WatchFailure::Fatal);
                    };
                    Some(WatchEvent::Hint(Box::new(hint.into_watch_hint(operation))))
                } else if watch.closed {
                    Some(WatchEvent::Closed)
                } else {
                    None
                }
            };
            if let Some(event) = event {
                return Ok(event);
            }
            if let Some((change, operation)) = self
                .pending_change
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                match self.dispatch_change(self.controller.clone(), change.clone(), operation.clone()) {
                    Ok(CoreDispatchOutcome::Suppressed(_))
                    | Ok(CoreDispatchOutcome::Admitted)
                    | Ok(CoreDispatchOutcome::Coalesced) => continue,
                    Err(CoreSourceError::WatchClosed) => return Ok(WatchEvent::Closed),
                    Err(CoreSourceError::Hint(HintAdmissionError::Backpressure)) => {
                        *self
                            .pending_change
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                            Some((change, operation));
                        return Err(WatchFailure::Backpressure);
                    }
                    Err(error) => {
                        tracing::error!(
                            zone = self.controller.zone().as_str(),
                            controller = controller_field(&self.controller),
                            reason = ?error,
                            "pending watch change escalated to fatal watch failure",
                        );
                        return Err(WatchFailure::Fatal);
                    }
                }
            }
            if self.watch_stream_enabled.load(Ordering::Acquire) {
                let mut signal_rx = self.watch_signal_rx.lock().await;
                tokio::select! {
                    biased;
                    change = self.api.receive_watch_change() => {
                        match change? {
                            Some((change, operation)) => {
                                match self.dispatch_change(
                                    self.controller.clone(),
                                    change.clone(),
                                    operation.clone(),
                                ) {
                                    Ok(CoreDispatchOutcome::Suppressed(_))
                                    | Ok(CoreDispatchOutcome::Admitted)
                                    | Ok(CoreDispatchOutcome::Coalesced) => continue,
                                    Err(CoreSourceError::WatchClosed) => return Ok(WatchEvent::Closed),
                                    Err(CoreSourceError::Hint(HintAdmissionError::Backpressure)) => {
                                        *self
                                            .pending_change
                                            .lock()
                                            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                                            Some((change, operation));
                                        return Err(WatchFailure::Backpressure);
                                    }
                                    Err(error) => {
                                        tracing::error!(
                                            zone = self.controller.zone().as_str(),
                                            controller = controller_field(&self.controller),
                                            reason = ?error,
                                            "stream watch change escalated to fatal watch failure",
                                        );
                                        return Err(WatchFailure::Fatal);
                                    }
                                }
                            }
                            None => {
                                tracing::warn!(
                                    zone = self.controller.zone().as_str(),
                                    controller = controller_field(&self.controller),
                                    "controller watch stream closed by source; disabling stream",
                                );
                                self.watch_stream_enabled.store(false, Ordering::Release);
                                self.watch
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .closed = true;
                                continue;
                            }
                        }
                    }
                    signal = signal_rx.recv() => {
                        if signal.is_none() {
                            return Ok(WatchEvent::Closed);
                        }
                        continue;
                    }
                }
            }
            if self.watch_signal_rx.lock().await.recv().await.is_none() {
                return Ok(WatchEvent::Closed);
            }
        }
    }

    fn read_fresh(
        &self,
        key: &ResourceKey,
    ) -> impl Future<Output = Result<FreshSnapshot, SourceError>> + Send {
        self.api.read_fresh(key)
    }

    fn write_starting(
        &self,
        context: &ReconcileContext,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.api.write_starting(context)
    }

    fn accept_effect(
        &self,
        context: &ReconcileContext,
        plan: &ReconcilePlan,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.api.accept_effect(context, plan)
    }

    fn accepted_effect_operation(
        &self,
        context: &ReconcileContext,
    ) -> impl Future<Output = Result<Option<OperationContext>, SourceError>> + Send {
        self.api.accepted_effect_operation(context)
    }

    fn complete_effect(
        &self,
        context: &ReconcileContext,
        result: &ReconcileResult,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.api.complete_effect(context, result)
    }

    fn verify_expedited_commit(
        &self,
        context: &ReconcileContext,
    ) -> impl Future<Output = Result<bool, SourceError>> + Send {
        self.api.verify_expedited_commit(context)
    }

    fn commit_result(
        &self,
        context: &ReconcileContext,
        result: &ReconcileResult,
    ) -> impl Future<Output = Result<CommitOutcome, SourceError>> + Send {
        self.api.commit_result(context, result)
    }

    fn complete_expedited(
        &self,
        context: &ReconcileContext,
        projection: &ReconcileProjection,
        status_persistence: StatusPersistence,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.api
            .complete_expedited(context, projection, status_persistence)
    }

    fn persist_outcome(
        &self,
        projection: &ReconcileProjection,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.api.persist_outcome(projection)
    }

    fn persist_outcome_with_operation(
        &self,
        projection: &ReconcileProjection,
        operation: &OperationContext,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.api
            .persist_outcome_with_operation(projection, operation)
    }

    fn checkpoint(
        &self,
        context: &ReconcileContext,
        revision: ZoneRevision,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.api.checkpoint(context, revision)
    }

    fn schedule_requeue(
        &self,
        key: &ResourceKey,
        at_tick: u64,
    ) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.api.schedule_requeue(key, at_tick)
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

fn provider_process_session_ready(
    provider_ref: &str,
    provider_uid: &str,
    provider_generation: u64,
    process: &ResourceSnapshot,
) -> bool {
    let process_value =
        match serde_json::from_slice::<serde_json::Value>(process.canonical_json()) {
            Ok(value) => value,
            Err(error) => {
                tracing::debug!(
                    resource = resource_field(process.key()),
                    reason = %error,
                    "controller session treated as not ready: process canonical JSON did not parse",
                );
                return false;
            }
        };
    let session = process_value
        .pointer("/status/resource/controllerSession")
        .and_then(serde_json::Value::as_object);
    let Some(session) = session else {
        return false;
    };
    let expected_process_ref = process.key().resource_ref().to_canonical_string();
    let process_uid = process.key().uid().as_str();
    session.get("ready").and_then(serde_json::Value::as_bool) == Some(true)
        && session.get("providerRef").and_then(serde_json::Value::as_str)
            == Some(provider_ref)
        && session.get("providerUid").and_then(serde_json::Value::as_str)
            == Some(provider_uid)
        && session.get("processRef").and_then(serde_json::Value::as_str)
            == Some(expected_process_ref.as_str())
        && session.get("processUid").and_then(serde_json::Value::as_str) == Some(process_uid)
        && session
            .get("processGeneration")
            .and_then(serde_json::Value::as_u64)
            == Some(process.generation().get())
        && session
            .get("providerGeneration")
            .and_then(serde_json::Value::as_u64)
            == Some(provider_generation)
        && session
            .get("controllerGeneration")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|generation| generation > 0)
        && session
            .get("sessionGeneration")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|generation| generation > 0)
        && session
            .get("artifactReady")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        && session
            .get("descriptorReady")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        && session
            .get("registrationReady")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
}

const SYSTEM_CORE_PROVIDER_REF: &str = "Provider/system-core";
const SYSTEM_MINIJAIL_PROVIDER_REF: &str = "Provider/system-minijail";
const SYSTEM_CORE_HOST_REF: &str = "Host/host-system";

/// Whether the fixed system-core handlers are ready, read from the `Zone`
/// dependency of the `Provider/system-core` row.
///
/// Public with `provider_observation`: the v3 Core driver (U12) applies the
/// same predicate over manager-served dependency rows, so the policy has one
/// home.
pub fn fixed_system_core_handlers_ready(dependencies: &[DependencySnapshot]) -> bool {
    dependencies.iter().any(|dependency| {
        let resource = dependency.resource();
        if resource.key().resource_ref().resource_type().as_str() != "Zone" {
            return false;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(resource.canonical_json())
        else {
            tracing::debug!(
                resource = resource_field(resource.key()),
                "mandatory-handler readiness treated as false: Zone canonical JSON did not parse",
            );
            return false;
        };
        let Some(status) = value.pointer("/status/resource").cloned() else {
            return false;
        };
        serde_json::from_value::<ZoneStatusResource>(status)
            .is_ok_and(|status| status.mandatory_handlers_ready())
    })
}

fn fixed_provider_host_ready(dependencies: &[DependencySnapshot]) -> bool {
    dependencies.iter().any(|dependency| {
        let resource = dependency.resource();
        if resource.key().resource_ref().to_canonical_string() != SYSTEM_CORE_HOST_REF {
            return false;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(resource.canonical_json())
        else {
            tracing::debug!(
                resource = resource_field(resource.key()),
                "host readiness treated as false: Host canonical JSON did not parse",
            );
            return false;
        };
        value
            .pointer("/spec/providerRef")
            .and_then(serde_json::Value::as_str)
            == Some(SYSTEM_CORE_PROVIDER_REF)
            && value
                .pointer("/status/phase")
                .and_then(serde_json::Value::as_str)
                == Some("Ready")
            && value
                .pointer("/status/observedGeneration")
                .and_then(serde_json::Value::as_u64)
                == Some(resource.generation().get())
    })
}

fn expected_provider_volume_refs(provider: &serde_json::Value) -> BTreeSet<String> {
    ["/status/resource/owned/refs", "/status/update/owned/refs"]
        .into_iter()
        .filter_map(|path| provider.pointer(path))
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|resource_ref| resource_ref.starts_with("Volume/"))
        .map(str::to_owned)
        .collect()
}

/// The pure observation the Core `Provider` handler applies to its
/// dependency list. Public so the bridge that merges manager-served
/// dependency rows (G5) is pinned against the exact consumer of that list.
pub fn provider_observation(
    resource: &ResourceSnapshot,
    dependencies: &[DependencySnapshot],
) -> Result<ProviderObservation, CoreReconcileError> {
    let provider = serde_json::from_slice::<serde_json::Value>(resource.canonical_json())
        .map_err(|error| {
            tracing::warn!(
                resource = resource_field(resource.key()),
                reason = %error,
                "provider observation failed: canonical JSON did not parse",
            );
            CoreReconcileError
        })?;
    let provider_ref = resource.key().resource_ref().to_canonical_string();
    let spec = provider
        .get("spec")
        .and_then(serde_json::Value::as_object)
        .ok_or(CoreReconcileError)?;
    let package_present = spec
        .get("artifactId")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|artifact| !artifact.is_empty());
    let config_valid = spec
        .get("config")
        .is_some_and(serde_json::Value::is_object);
    let fixed_host_ready = match provider_ref.as_str() {
        SYSTEM_CORE_PROVIDER_REF => Some(fixed_system_core_handlers_ready(dependencies)),
        SYSTEM_MINIJAIL_PROVIDER_REF => Some(fixed_provider_host_ready(dependencies)),
        _ => None,
    };
    if let Some(required_ready) = fixed_host_ready {
        return Ok(ProviderObservation {
            package_present: provider_ref == SYSTEM_CORE_PROVIDER_REF || package_present,
            config_valid,
            graph_valid: true,
            conformance_valid: true,
            required_dependencies_ready: required_ready,
            required_components_ready: required_ready,
            optional_components_degraded: false,
            components_drained: true,
        });
    }
    let mut process_count = 0_usize;
    let mut graph_valid = true;
    let mut conformance_valid = true;
    let mut required_components_ready = true;
    let mut required_dependencies_ready = true;
    let mut optional_components_degraded = false;
    let expected_volume_refs = expected_provider_volume_refs(&provider);
    let mut observed_volume_refs = BTreeSet::new();

    for dependency in dependencies {
        let dependency_resource = dependency.resource();
        let value = serde_json::from_slice::<serde_json::Value>(
            dependency_resource.canonical_json(),
        )
        .map_err(|error| {
            tracing::warn!(
                resource = resource_field(dependency_resource.key()),
                reason = %error,
                "provider observation failed: dependency canonical JSON did not parse",
            );
            CoreReconcileError
        })?;
        let owner_ref = value
            .pointer("/metadata/ownerRef")
            .and_then(serde_json::Value::as_str);
        if owner_ref != Some(provider_ref.as_str()) {
            continue;
        }
        match dependency_resource
            .key()
            .resource_ref()
            .resource_type()
            .as_str()
        {
            "Process" => {
                if dependency_resource.owner_uid() != Some(resource.key().uid()) {
                    graph_valid = false;
                    conformance_valid = false;
                    required_components_ready = false;
                    required_dependencies_ready = false;
                    continue;
                }
                process_count += 1;
                let process_spec = value
                    .get("spec")
                    .and_then(serde_json::Value::as_object);
                let process_valid = process_spec
                    .and_then(|spec| spec.get("processClass"))
                    .and_then(serde_json::Value::as_str)
                    == Some("controller")
                    && process_spec
                        .and_then(|spec| spec.get("providerRef"))
                        .and_then(serde_json::Value::as_str)
                            .is_some_and(|provider| {
                                matches!(
                                    provider,
                                    "Provider/system-minijail" | "Provider/system-systemd"
                                )
                            });
                let phase = value
                    .pointer("/status/phase")
                    .and_then(serde_json::Value::as_str);
                let process_ready = phase == Some("Ready")
                    && value
                        .pointer("/status/observedGeneration")
                        .and_then(serde_json::Value::as_u64)
                        == Some(dependency_resource.generation().get());
                let provider_uid = resource.key().uid().as_str();
                let session_ready = provider_process_session_ready(
                    provider_ref.as_str(),
                    provider_uid,
                    resource.generation().get(),
                    dependency_resource,
                );
                graph_valid &= process_valid;
                conformance_valid &= process_valid && session_ready;
                required_components_ready &= process_ready && session_ready;
                required_dependencies_ready &= process_ready;
                optional_components_degraded |= phase == Some("Degraded");
            }
            "Volume" => {
                let volume_ref = dependency_resource
                    .key()
                    .resource_ref()
                    .to_canonical_string();
                if !expected_volume_refs.contains(&volume_ref) {
                    continue;
                }
                observed_volume_refs.insert(volume_ref);
                let owner_uid_matches =
                    dependency_resource.owner_uid() == Some(resource.key().uid());
                if !owner_uid_matches {
                    required_dependencies_ready = false;
                    continue;
                }
                let volume_ready = value
                    .pointer("/status/phase")
                    .and_then(serde_json::Value::as_str)
                    == Some("Ready")
                    && value
                        .pointer("/status/observedGeneration")
                        .and_then(serde_json::Value::as_u64)
                        == Some(dependency_resource.generation().get());
                required_dependencies_ready &= volume_ready;
            }
            _ => {}
        }
    }
    if !expected_volume_refs.is_empty()
        && !expected_volume_refs.is_subset(&observed_volume_refs)
    {
        required_dependencies_ready = false;
    }

    let has_processes = process_count > 0;
    Ok(ProviderObservation {
        package_present: package_present && has_processes,
        config_valid,
        graph_valid: graph_valid && has_processes,
        conformance_valid: conformance_valid && has_processes,
        required_dependencies_ready: required_dependencies_ready && has_processes,
        required_components_ready: required_components_ready && has_processes,
        optional_components_degraded,
        components_drained: !has_processes,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};

    use d2b_contracts_resource::v3::{
        ControllerGeneration, ObservedGeneration, ResourceGeneration, ResourcePhase, ResourceRef,
        ResourceTypeName, ResourceUid, ZoneId,
    };
    use d2b_controller_toolkit::{
        ControllerExecutionPolicy, ControllerIdentity, ControllerSelector, ControllerVerb,
        ProjectionDisposition, ReconcileReason, ResourceRegistration, ResyncPolicy, SelectorField,
        TriggerReason,
    };

    use super::*;
    use crate::providers::{ProviderHandler, ProviderIntent, ProviderPhase};
    use crate::{ChangeField, CoreTriggerReason};

    const OUTCOME_RETENTION: usize = 2;

    struct TestRegisteredApi {
        initial: InitialList,
        snapshots: Mutex<BTreeMap<ResourceKey, FreshSnapshot>>,
        starting: Mutex<BTreeSet<(ResourceKey, ZoneRevision)>>,
        commits: Mutex<BTreeSet<(ResourceKey, ZoneRevision)>>,
        committed_results: Mutex<Vec<ReconcileResult>>,
        checkpoints: Mutex<BTreeSet<(ResourceKey, ZoneRevision)>>,
        checkpoint_calls: AtomicUsize,
        checkpoint_notify: tokio::sync::Notify,
        outcomes: Mutex<VecDeque<(ResourceKey, ZoneRevision, ReconcileReason)>>,
        watch_changes:
            Mutex<VecDeque<Result<Option<(ChangeRecord, OperationContext)>, WatchFailure>>>,
        watch_stream_enabled: AtomicBool,
        watch_change_notify: tokio::sync::Notify,
        watch_receive_started: tokio::sync::Notify,
    }

    impl TestRegisteredApi {
        fn new(initial: InitialList, snapshots: BTreeMap<ResourceKey, FreshSnapshot>) -> Arc<Self> {
            Arc::new(Self {
                initial,
                snapshots: Mutex::new(snapshots),
                starting: Mutex::new(BTreeSet::new()),
                commits: Mutex::new(BTreeSet::new()),
                committed_results: Mutex::new(Vec::new()),
                checkpoints: Mutex::new(BTreeSet::new()),
                checkpoint_calls: AtomicUsize::new(0),
                checkpoint_notify: tokio::sync::Notify::new(),
                outcomes: Mutex::new(VecDeque::new()),
                watch_changes: Mutex::new(VecDeque::new()),
                watch_stream_enabled: AtomicBool::new(false),
                watch_change_notify: tokio::sync::Notify::new(),
                watch_receive_started: tokio::sync::Notify::new(),
            })
        }

        fn enable_watch_stream(
            &self,
            changes: Vec<Result<Option<(ChangeRecord, OperationContext)>, WatchFailure>>,
        ) {
            self.watch_changes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(changes);
            self.watch_stream_enabled.store(true, Ordering::Release);
        }

        fn push_watch_change(
            &self,
            change: Result<Option<(ChangeRecord, OperationContext)>, WatchFailure>,
        ) {
            self.watch_changes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push_back(change);
            self.watch_change_notify.notify_one();
        }

        fn record_outcome(&self, projection: &ReconcileProjection) {
            let mut outcomes = self
                .outcomes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let key = (projection.target().clone(), projection.revision());
            if outcomes
                .iter()
                .any(|(target, revision, _)| (target, *revision) == (&key.0, key.1))
            {
                return;
            }
            if outcomes.len() == OUTCOME_RETENTION {
                outcomes.pop_front();
            }
            outcomes.push_back((key.0, key.1, projection.reason()));
        }
    }

    impl RegisteredControllerApi for TestRegisteredApi {
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
            std::future::ready(Ok(self.initial.clone()))
        }

        fn open_watch(
            &self,
            _descriptor: &ControllerDescriptor,
            _after_revision: ZoneRevision,
        ) -> impl Future<Output = Result<(), SourceError>> + Send {
            std::future::ready(Ok(()))
        }

        fn has_watch_stream(&self) -> bool {
            self.watch_stream_enabled.load(Ordering::Acquire)
        }

        fn receive_watch_change(
            &self,
        ) -> impl Future<Output = Result<Option<(ChangeRecord, OperationContext)>, WatchFailure>>
        + Send {
            async move {
                loop {
                    if let Some(change) = self
                        .watch_changes
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .pop_front()
                    {
                        return change;
                    }
                    self.watch_receive_started.notify_one();
                    self.watch_change_notify.notified().await;
                }
            }
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
            context: &ReconcileContext,
        ) -> impl Future<Output = Result<(), SourceError>> + Send {
            self.starting
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert((context.target().clone(), context.revision()));
            std::future::ready(Ok(()))
        }

        fn commit_result(
            &self,
            context: &ReconcileContext,
            result: &ReconcileResult,
        ) -> impl Future<Output = Result<CommitOutcome, SourceError>> + Send {
            self.commits
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert((context.target().clone(), context.revision()));
            self.committed_results
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(result.clone());
            std::future::ready(Ok(CommitOutcome::Committed(context.revision())))
        }

        fn complete_expedited(
            &self,
            _context: &ReconcileContext,
            projection: &ReconcileProjection,
            _status_persistence: StatusPersistence,
        ) -> impl Future<Output = Result<(), SourceError>> + Send {
            self.record_outcome(projection);
            std::future::ready(Ok(()))
        }

        fn persist_outcome(
            &self,
            projection: &ReconcileProjection,
        ) -> impl Future<Output = Result<(), SourceError>> + Send {
            self.record_outcome(projection);
            std::future::ready(Ok(()))
        }

        fn checkpoint(
            &self,
            context: &ReconcileContext,
            revision: ZoneRevision,
        ) -> impl Future<Output = Result<(), SourceError>> + Send {
            self.checkpoints
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert((context.target().clone(), revision));
            self.checkpoint_calls.fetch_add(1, Ordering::Release);
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

    fn key(name: &str, suffix: u16) -> ResourceKey {
        ResourceKey::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse(&format!("Process/{name}")).unwrap(),
            ResourceUid::parse(format!("123e4567-e89b-42d3-a456-{suffix:012}")).unwrap(),
        )
    }

    fn controller_key() -> ControllerLeaseKey {
        ControllerLeaseKey::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse("Process/controller").unwrap(),
        )
        .unwrap()
    }

    fn descriptor(max_pending: usize) -> ControllerDescriptor {
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
            Vec::new(),
            vec!["service.v1".to_owned()],
            vec!["schema.v1".to_owned()],
            ControllerExecutionPolicy::new(
                1,
                1,
                max_pending,
                1,
                4,
                ResyncPolicy::new(Some(100), 5_000).unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn snapshot(target: ResourceKey, revision: u64) -> ResourceSnapshot {
        ResourceSnapshot::new(
            target,
            ZoneRevision::new(revision),
            ResourceGeneration::new(revision).unwrap(),
            b"{}".to_vec(),
            false,
        )
    }

    fn change(
        target: ResourceKey,
        revision: u64,
        reasons: BTreeSet<CoreTriggerReason>,
    ) -> ChangeRecord {
        ChangeRecord {
            target,
            revision: ZoneRevision::new(revision),
            generation: ResourceGeneration::new(revision).unwrap(),
            observed_generation: ObservedGeneration::new(revision.saturating_sub(1)),
            fields: BTreeSet::from([ChangeField::Spec]),
            reasons,
            type_is_bound: true,
            relevant_field_changed: true,
            own_status_only: false,
            owner_consumer_exists: false,
            dependency_consumer_exists: false,
            controller_generation_current: true,
            conditions_require_work: false,
            unknown_requires_observation: false,
        }
    }

    fn operation(id: &str) -> OperationContext {
        OperationContext::new(id, format!("idem-{id}"), format!("corr-{id}"), None).unwrap()
    }

    fn source_with_snapshot(
        descriptor: &ControllerDescriptor,
        target: &ResourceKey,
        revision: u64,
    ) -> (
        Arc<TestRegisteredApi>,
        Arc<CoreControllerSource<TestRegisteredApi>>,
    ) {
        let target_snapshot = snapshot(target.clone(), revision);
        let api = TestRegisteredApi::new(
            InitialList {
                resources: Vec::new(),
                snapshot_revision: ZoneRevision::new(1),
            },
            BTreeMap::from([(
                target.clone(),
                FreshSnapshot::Present {
                    target: target_snapshot,
                    dependencies: Vec::new(),
                },
            )]),
        );
        let source = CoreControllerSource::new(descriptor.clone(), Arc::clone(&api));
        (api, source)
    }

    fn provider_fixture(session_ready: bool) -> (ResourceSnapshot, DependencySnapshot) {
        provider_fixture_with_process_owner_generation(session_ready, 2, Some(2))
    }

    fn provider_fixture_with_session_provider_generation(
        session_ready: bool,
        session_provider_generation: u64,
    ) -> (ResourceSnapshot, DependencySnapshot) {
        provider_fixture_with_process_owner_generation(
            session_ready,
            session_provider_generation,
            Some(2),
        )
    }

    fn provider_fixture_with_process_owner_generation(
        session_ready: bool,
        session_provider_generation: u64,
        process_owner_generation: Option<u64>,
    ) -> (ResourceSnapshot, DependencySnapshot) {
        let zone = ZoneId::parse("work").unwrap();
        let provider_ref = ResourceRef::parse("Provider/runtime").unwrap();
        let provider_uid =
            ResourceUid::parse("11111111-1111-4111-8111-111111111111").unwrap();
        let process_ref = ResourceRef::parse("Process/runtime-controller").unwrap();
        let process_uid =
            ResourceUid::parse("22222222-2222-4222-8222-222222222222").unwrap();
        let provider = ResourceSnapshot::new(
            ResourceKey::new(zone.clone(), provider_ref, provider_uid.clone()),
            ZoneRevision::new(4),
            ResourceGeneration::new(2).unwrap(),
            serde_json::to_vec(&serde_json::json!({
                "metadata": {
                    "uid": provider_uid.as_str(),
                    "generation": 2,
                    "name": "runtime",
                    "finalizers": ["core.provider-api-binding"]
                },
                "spec": {
                    "artifactId": "runtime",
                    "config": {}
                },
                "status": {
                    "phase": "Pending",
                    "observedGeneration": 0,
                    "resource": {}
                }
            }))
            .unwrap(),
            false,
        );
        let process = ResourceSnapshot::new(
            ResourceKey::new(zone, process_ref, process_uid.clone()),
            ZoneRevision::new(4),
            ResourceGeneration::new(3).unwrap(),
            serde_json::to_vec(&serde_json::json!({
                "metadata": {
                    "uid": process_uid.as_str(),
                    "generation": 3,
                    "ownerRef": "Provider/runtime"
                },
                "spec": {
                    "processClass": "controller",
                    "providerRef": "Provider/system-minijail"
                },
                "status": {
                    "phase": "Ready",
                    "observedGeneration": 3,
                    "resource": {
                        "controllerSession": {
                            "ready": session_ready,
                            "providerRef": "Provider/runtime",
                            "providerUid": provider_uid.as_str(),
                            "providerGeneration": session_provider_generation,
                            "processRef": "Process/runtime-controller",
                            "processUid": process_uid.as_str(),
                            "processGeneration": 3,
                            "controllerGeneration": 7,
                            "sessionGeneration": 9,
                            "artifactReady": session_ready,
                            "descriptorReady": session_ready,
                            "registrationReady": session_ready
                        }
                    }
                }
            }))
            .unwrap(),
            false,
        )
        .with_owner_identity(
            Some(provider_uid.clone()),
            process_owner_generation
                .map(|generation| ResourceGeneration::new(generation).unwrap()),
        );
        (provider, DependencySnapshot::new(process))
    }

    fn provider_volume_fixture(
        owner_uid: ResourceUid,
        owner_generation: Option<u64>,
    ) -> DependencySnapshot {
        let zone = ZoneId::parse("work").unwrap();
        let volume_ref = ResourceRef::parse("Volume/runtime-state").unwrap();
        let volume_uid =
            ResourceUid::parse("33333333-3333-4333-8333-333333333333").unwrap();
        DependencySnapshot::new(
            ResourceSnapshot::new(
                ResourceKey::new(zone, volume_ref, volume_uid),
                ZoneRevision::new(4),
                ResourceGeneration::new(1).unwrap(),
                serde_json::to_vec(&serde_json::json!({
                    "metadata": {
                        "uid": "33333333-3333-4333-8333-333333333333",
                        "generation": 1,
                        "ownerRef": "Provider/runtime"
                    },
                    "status": {
                        "phase": "Ready",
                        "observedGeneration": 1
                    }
                }))
                .unwrap(),
                false,
            )
            .with_owner_identity(
                Some(owner_uid),
                owner_generation.map(|generation| ResourceGeneration::new(generation).unwrap()),
            ),
        )
    }

    fn fixed_provider_fixture(name: &str) -> ResourceSnapshot {
        let uid = match name {
            "system-core" => "55555555-5555-4555-8555-555555555555",
            "system-minijail" => "66666666-6666-4666-8666-666666666666",
            _ => "77777777-7777-4777-8777-777777777777",
        };
        ResourceSnapshot::new(
            ResourceKey::new(
                ZoneId::parse("work").unwrap(),
                ResourceRef::parse(&format!("Provider/{name}")).unwrap(),
                ResourceUid::parse(uid).unwrap(),
            ),
            ZoneRevision::new(1),
            ResourceGeneration::new(1).unwrap(),
            serde_json::to_vec(&serde_json::json!({
                "spec": {
                    "artifactId": name,
                    "config": {}
                },
                "status": {
                    "phase": "Pending",
                    "observedGeneration": 0,
                    "resource": {}
                }
            }))
            .unwrap(),
            false,
        )
    }

    fn fixed_zone_dependency(ready: bool) -> DependencySnapshot {
        use d2b_contracts_zone_session::v3::{
            ZoneHandlerName, ZoneHandlerPhase, ZoneHandlerStatus, ZoneStatusResource,
        };

        let phase = if ready {
            d2b_contracts_resource::v3::ResourcePhase::Ready
        } else {
            d2b_contracts_resource::v3::ResourcePhase::Pending
        };
        let handler_phase = if ready {
            ZoneHandlerPhase::Ready
        } else {
            ZoneHandlerPhase::Pending
        };
        let status = ZoneStatusResource::new(
            1,
            1,
            1,
            phase,
            vec![
                ZoneHandlerStatus::new(ZoneHandlerName::SystemCoreHost, handler_phase, None),
                ZoneHandlerStatus::new(ZoneHandlerName::SystemCoreUser, handler_phase, None),
            ],
            1,
            1,
            1,
            1,
            false,
            0,
        )
        .unwrap();
        DependencySnapshot::new(ResourceSnapshot::new(
            ResourceKey::new(
                ZoneId::parse("work").unwrap(),
                ResourceRef::parse("Zone/work").unwrap(),
                ResourceUid::parse("88888888-8888-4888-8888-888888888888").unwrap(),
            ),
            ZoneRevision::new(1),
            ResourceGeneration::new(1).unwrap(),
            serde_json::to_vec(&serde_json::json!({
                "status": {
                    "resource": serde_json::to_value(status).unwrap()
                }
            }))
            .unwrap(),
            false,
        ))
    }

    fn fixed_host_dependency(ready: bool) -> DependencySnapshot {
        DependencySnapshot::new(ResourceSnapshot::new(
            ResourceKey::new(
                ZoneId::parse("work").unwrap(),
                ResourceRef::parse("Host/host-system").unwrap(),
                ResourceUid::parse("99999999-9999-4999-8999-999999999999").unwrap(),
            ),
            ZoneRevision::new(1),
            ResourceGeneration::new(1).unwrap(),
            serde_json::to_vec(&serde_json::json!({
                "spec": {
                    "providerRef": "Provider/system-core"
                },
                "status": {
                    "phase": if ready { "Ready" } else { "Pending" },
                    "observedGeneration": if ready { 1 } else { 0 }
                }
            }))
            .unwrap(),
            false,
        ))
    }

    fn provider_with_expected_volume(provider: ResourceSnapshot) -> ResourceSnapshot {
        let mut value: serde_json::Value =
            serde_json::from_slice(provider.canonical_json()).unwrap();
        value["status"]["update"]["owned"] =
            serde_json::json!({"count": 1, "refs": ["Volume/runtime-state"]});
        ResourceSnapshot::new(
            provider.key().clone(),
            provider.revision(),
            provider.generation(),
            serde_json::to_vec(&value).unwrap(),
            false,
        )
    }

    #[test]
    fn provider_readiness_requires_durable_session_evidence() {
        for session_ready in [false, true] {
            let (provider, process) = provider_fixture(session_ready);
            let observation = provider_observation(&provider, &[process]).unwrap();
            let phase = ProviderHandler::plan_observed(
                provider.key().resource_ref(),
                ProviderIntent::Enable,
                observation,
            )
            .map(|plan| plan.phase())
            .unwrap_or(ProviderPhase::Pending);
            assert_eq!(
                phase,
                if session_ready {
                    ProviderPhase::Ready
                } else {
                    ProviderPhase::Pending
                }
            );
            if session_ready {
                // The durable status candidate and its damping are gone with
                // the old plane (R11: status is in memory); the driver
                // publishes this same observation through
                // `CoreDriverStatus::Provider`.
                assert!(observation.package_present);
                assert!(observation.conformance_valid);
            }
        }
    }

    #[test]
    fn provider_readiness_rejects_session_evidence_from_an_older_provider_generation() {
        let (provider, process) = provider_fixture_with_session_provider_generation(true, 1);
        let observation = provider_observation(&provider, &[process]).unwrap();
        let phase = ProviderHandler::plan_observed(
            provider.key().resource_ref(),
            ProviderIntent::Enable,
            observation,
        )
        .map(|plan| plan.phase())
        .unwrap_or(ProviderPhase::Pending);
        assert_eq!(phase, ProviderPhase::Pending);
    }

    #[test]
    fn provider_readiness_accepts_legacy_process_without_owner_generation() {
        let (provider, process) =
            provider_fixture_with_process_owner_generation(true, 2, None);
        let observation = provider_observation(&provider, &[process]).unwrap();
        let phase = ProviderHandler::plan_observed(
            provider.key().resource_ref(),
            ProviderIntent::Enable,
            observation,
        )
        .map(|plan| plan.phase())
        .unwrap_or(ProviderPhase::Pending);
        assert_eq!(phase, ProviderPhase::Ready);
    }

    #[test]
    fn fixed_system_core_readiness_uses_mandatory_handler_evidence_without_process() {
        let provider = fixed_provider_fixture("system-core");
        for ready in [false, true] {
            let dependencies = vec![fixed_zone_dependency(ready)];
            let observation = provider_observation(&provider, &dependencies).unwrap();
            let phase = ProviderHandler::plan_system_core(
                fixed_system_core_handlers_ready(&dependencies),
            )
            .phase();
            assert_eq!(
                phase,
                if ready {
                    ProviderPhase::Ready
                } else {
                    ProviderPhase::Pending
                }
            );
            assert_eq!(observation.required_components_ready, ready);
        }
    }

    #[test]
    fn fixed_system_minijail_readiness_uses_host_evidence_without_process() {
        let provider = fixed_provider_fixture("system-minijail");
        for ready in [false, true] {
            let dependencies = vec![fixed_host_dependency(ready)];
            let observation = provider_observation(&provider, &dependencies).unwrap();
            let phase = ProviderHandler::plan_observed(
                provider.key().resource_ref(),
                ProviderIntent::Enable,
                observation,
            )
            .unwrap()
            .phase();
            assert_eq!(
                phase,
                if ready {
                    ProviderPhase::Ready
                } else {
                    ProviderPhase::Pending
                }
            );
        }
    }

    #[test]
    fn fixed_provider_readiness_does_not_use_controller_session_log_alone() {
        let (_, process) = provider_fixture(true);
        let system_minijail = fixed_provider_fixture("system-minijail");
        let observation = provider_observation(&system_minijail, &[process]).unwrap();
        let phase = ProviderHandler::plan_observed(
            system_minijail.key().resource_ref(),
            ProviderIntent::Enable,
            observation,
        )
        .unwrap()
        .phase();
        assert_eq!(phase, ProviderPhase::Pending);

        let system_core = fixed_provider_fixture("system-core");
        let observation = provider_observation(&system_core, &[provider_fixture(true).1]).unwrap();
        let phase = ProviderHandler::plan_system_core(
            fixed_system_core_handlers_ready(&[provider_fixture(true).1]),
        )
        .phase();
        assert_eq!(phase, ProviderPhase::Pending);
        assert!(!observation.required_components_ready);
    }

    #[test]
    fn provider_readiness_requires_matching_volume_owner_uid_and_observed_state() {
        let (provider, process) = provider_fixture(true);
        let expected_volume_provider = provider_with_expected_volume(provider.clone());
        let stale_owner_uid =
            ResourceUid::parse("44444444-4444-4444-8444-444444444444").unwrap();
        let stale_volume = provider_volume_fixture(stale_owner_uid, Some(2));
        let observation = provider_observation(
            &expected_volume_provider,
            &[process.clone(), stale_volume],
        )
        .unwrap();
        let phase = ProviderHandler::plan_observed(
            expected_volume_provider.key().resource_ref(),
            ProviderIntent::Enable,
            observation,
        )
        .map(|plan| plan.phase())
        .unwrap_or(ProviderPhase::Pending);
        assert_eq!(phase, ProviderPhase::Pending);

        let old_owner_generation_volume =
            provider_volume_fixture(provider.key().uid().clone(), Some(1));
        let observation = provider_observation(
            &expected_volume_provider,
            &[provider_fixture(true).1, old_owner_generation_volume],
        )
        .unwrap();
        let phase = ProviderHandler::plan_observed(
            expected_volume_provider.key().resource_ref(),
            ProviderIntent::Enable,
            observation,
        )
        .map(|plan| plan.phase())
        .unwrap_or(ProviderPhase::Pending);
        assert_eq!(phase, ProviderPhase::Ready);

        let absent_owner_generation_volume =
            provider_volume_fixture(provider.key().uid().clone(), None);
        let observation = provider_observation(
            &expected_volume_provider,
            &[provider_fixture(true).1, absent_owner_generation_volume],
        )
        .unwrap();
        let phase = ProviderHandler::plan_observed(
            expected_volume_provider.key().resource_ref(),
            ProviderIntent::Enable,
            observation,
        )
        .map(|plan| plan.phase())
        .unwrap_or(ProviderPhase::Pending);
        assert_eq!(phase, ProviderPhase::Ready);

        let observation =
            provider_observation(&expected_volume_provider, &[process.clone()]).unwrap();
        let phase = ProviderHandler::plan_observed(
            expected_volume_provider.key().resource_ref(),
            ProviderIntent::Enable,
            observation,
        )
        .map(|plan| plan.phase())
        .unwrap_or(ProviderPhase::Pending);
        assert_eq!(phase, ProviderPhase::Pending);

        let optional_volume = provider_volume_fixture(
            ResourceUid::parse("44444444-4444-4444-8444-444444444444").unwrap(),
            Some(2),
        );
        let observation =
            provider_observation(&provider, &[process.clone(), optional_volume]).unwrap();
        let phase = ProviderHandler::plan_observed(
            provider.key().resource_ref(),
            ProviderIntent::Enable,
            observation,
        )
        .map(|plan| plan.phase())
        .unwrap_or(ProviderPhase::Pending);
        assert_eq!(phase, ProviderPhase::Ready);

        let observation = provider_observation(&provider, &[process]).unwrap();
        let phase = ProviderHandler::plan_observed(
            provider.key().resource_ref(),
            ProviderIntent::Enable,
            observation,
        )
        .map(|plan| plan.phase())
        .unwrap_or(ProviderPhase::Pending);
        assert_eq!(phase, ProviderPhase::Ready);
    }

    #[tokio::test]
    async fn declared_observation_wakeup_bypasses_convergence_suppression() {
        let target = key("observed", 9);
        let descriptor = descriptor(2);
        let (_api, source) = source_with_snapshot(&descriptor, &target, 2);

        assert_eq!(
            source
                .dispatch_observation(target, ZoneRevision::new(2))
                .unwrap(),
            CoreDispatchOutcome::Admitted
        );
        let WatchEvent::Hint(hint) = source.receive_watch().await.unwrap() else {
            panic!("observation wakeup returned an unexpected event");
        };
        assert!(hint.reasons().contains(TriggerReason::ScheduledObserve));
    }

    #[tokio::test]
    async fn core_watch_admission_is_bounded_coalesced_and_counted() {
        let descriptor = descriptor(1);
        let first = key("first", 4);
        let api = TestRegisteredApi::new(
            InitialList {
                resources: Vec::new(),
                snapshot_revision: ZoneRevision::new(1),
            },
            BTreeMap::new(),
        );
        let source = CoreControllerSource::new(descriptor, api);

        assert_eq!(
            source
                .dispatch_change(
                    controller_key(),
                    change(
                        first.clone(),
                        2,
                        BTreeSet::from([CoreTriggerReason::SpecGenerationChanged]),
                    ),
                    operation("first"),
                )
                .unwrap(),
            CoreDispatchOutcome::Admitted
        );
        assert_eq!(
            source
                .dispatch_change(
                    controller_key(),
                    change(
                        first,
                        3,
                        BTreeSet::from([CoreTriggerReason::DeletionRequested]),
                    ),
                    operation("coalesced"),
                )
                .unwrap(),
            CoreDispatchOutcome::Coalesced
        );
        assert_eq!(
            source
                .dispatch_change(
                    controller_key(),
                    change(
                        key("second", 5),
                        2,
                        BTreeSet::from([CoreTriggerReason::SpecGenerationChanged]),
                    ),
                    operation("rejected"),
                )
                .unwrap_err(),
            CoreSourceError::Hint(HintAdmissionError::Backpressure)
        );
        assert_eq!(
            source.admission_counts(),
            CoreAdmissionCounts {
                admitted: 1,
                coalesced: 1,
                backpressure: 1,
            }
        );

        let WatchEvent::Hint(hint) = source.receive_watch().await.unwrap() else {
            panic!("bounded watch returned an unexpected event");
        };
        assert_eq!(hint.revision(), ZoneRevision::new(3));
        assert!(
            hint.reasons()
                .contains(TriggerReason::SpecGenerationChanged)
        );
        assert!(hint.reasons().contains(TriggerReason::DeletionRequested));
    }

    #[tokio::test]
    async fn core_watch_backpressure_retains_the_unadmitted_change() {
        let descriptor = descriptor(1);
        let first = key("stream-first", 6);
        let second = key("stream-second", 7);
        let api = TestRegisteredApi::new(
            InitialList {
                resources: Vec::new(),
                snapshot_revision: ZoneRevision::new(1),
            },
            BTreeMap::new(),
        );
        api.enable_watch_stream(Vec::new());
        let source = CoreControllerSource::new(descriptor.clone(), Arc::clone(&api));
        source
            .open_watch(&descriptor, ZoneRevision::new(1))
            .await
            .unwrap();

        let waiting_source = Arc::clone(&source);
        let pending = tokio::spawn(async move { waiting_source.receive_watch().await });
        api.watch_receive_started.notified().await;
        source.dispatch_change(
            controller_key(),
            change(
                first.clone(),
                2,
                BTreeSet::from([CoreTriggerReason::SpecGenerationChanged]),
            ),
            operation("stream-first"),
        )
        .unwrap();
        api.push_watch_change(Ok(Some((
            change(
                second.clone(),
                3,
                BTreeSet::from([CoreTriggerReason::DeletionRequested]),
            ),
            operation("stream-second"),
        ))));
        assert_eq!(
            pending.await.unwrap(),
            Err(WatchFailure::Backpressure)
        );
        let WatchEvent::Hint(first_hint) = source.receive_watch().await.unwrap() else {
            panic!("the admitted stream change must remain available");
        };
        assert_eq!(first_hint.key(), &first);
        let WatchEvent::Hint(second_hint) = source.receive_watch().await.unwrap() else {
            panic!("the backpressured stream change must be retried");
        };
        assert_eq!(second_hint.key(), &second);
        assert_eq!(second_hint.revision(), ZoneRevision::new(3));
    }

    #[tokio::test]
    async fn test_persistence_is_revision_idempotent_and_retention_bounded() {
        let api = TestRegisteredApi::new(
            InitialList {
                resources: Vec::new(),
                snapshot_revision: ZoneRevision::new(1),
            },
            BTreeMap::new(),
        );
        let target = key("outcome", 6);
        for revision in [2, 2, 3, 4] {
            api.persist_outcome(&ReconcileProjection::new(
                target.clone(),
                ZoneRevision::new(revision),
                ResourcePhase::Failed,
                ProjectionDisposition::Failed,
                ReconcileReason::HandlerTerminal,
                false,
            ))
            .await
            .unwrap();
        }
        let outcomes = api
            .outcomes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(outcomes.len(), OUTCOME_RETENTION);
        assert_eq!(outcomes[0].1, ZoneRevision::new(3));
        assert_eq!(outcomes[1].1, ZoneRevision::new(4));
    }
}
