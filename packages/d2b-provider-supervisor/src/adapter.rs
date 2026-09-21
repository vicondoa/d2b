//! Bounded async adapter for the blocking process effect owner.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{
    Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel,
};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Mutex as AsyncMutex;
use std::task::{Context, Poll, Waker};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::broker::{BrokerLaunchResolver, BrokerProcessBackend};
use d2b_contracts_resource::v3::ResourceUid;
use d2b_provider_process::{
    AdoptionCandidate, BackendObservation, LaunchTicket, LaunchedProcess, PidfdEvidence,
    ProcessConformanceError, ProcessEffectBackend, ProcessEffectError, ProcessIdentityDigest,
    ProcessLaunchEffectPort, ProcessLaunchRequest, ProcessRequest, ProcessStopClass, StopClass,
};
use tracing::{debug, error, warn};
/// Default upper bound for concurrent blocking process effects.
pub const DEFAULT_BLOCKING_LIMIT: usize = 16;



type Job = Box<dyn FnOnce() + Send + 'static>;

struct BlockingPool {
    sender: Option<SyncSender<Job>>,
    workers: Vec<JoinHandle<()>>,
    deadline_sender: Option<SyncSender<Deadline>>,
    deadline_worker: Option<JoinHandle<()>>,
}

struct Deadline {
    at: Instant,
    state: Weak<dyn DeadlineState>,
}

trait DeadlineState: Send + Sync {
    fn is_completed(&self) -> bool;
    fn wake_deadline(&self);
}

impl BlockingPool {
    fn new(limit: usize) -> Self {
        let (sender, receiver) = sync_channel::<Job>(limit);
        let receiver = Arc::new(Mutex::new(receiver));
        let workers = (0..limit)
            .map(|_| {
                let receiver = Arc::clone(&receiver);
                std::thread::Builder::new()
                    .name("d2b-process-effect".to_owned())
                    .spawn(move || worker(receiver))
                    .expect("create bounded process effect worker")
            })
            .collect();
        // Bounded deadline queue: capacity is twice the blocking-pool limit, so a
        // registration can never outgrow the in-flight job set while the deadline
        // worker keeps draining. A full queue only means the worker is
        // momentarily starved: the registration is deferred (the job still runs
        // and `JobFuture::poll` enforces the deadline) instead of blocking the
        // executor worker or spurious-failing a healthy launch. Only a
        // disconnected queue - the worker is gone - refuses with launch-failed.
        let (deadline_sender, deadline_receiver) = sync_channel::<Deadline>(limit * 2);
        let deadline_worker = std::thread::Builder::new()
            .name("d2b-process-deadlines".to_owned())
            .spawn(move || deadline_worker(deadline_receiver))
            .expect("create process effect deadline worker");
        Self {
            sender: Some(sender),
            workers,
            deadline_sender: Some(deadline_sender),
            deadline_worker: Some(deadline_worker),
        }
    }

    fn submit<T, F>(&self, timeout: Duration, operation: F) -> JobFuture<T>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, ProcessEffectError> + Send + 'static,
    {
        self.submit_with_deadline(timeout, move |_| operation())
    }

    fn submit_with_deadline<T, F>(&self, timeout: Duration, operation: F) -> JobFuture<T>
    where
        T: Send + 'static,
        F: FnOnce(Instant) -> Result<T, ProcessEffectError> + Send + 'static,
    {
        let deadline = Instant::now() + timeout;
        let state = Arc::new(JobState::default());
        let worker_state = Arc::clone(&state);
        let job = Box::new(move || worker_state.complete(operation(deadline)));
        let deadline_state: Arc<dyn DeadlineState> = state.clone();
        match self
            .deadline_sender
            .as_ref()
            .expect("deadline sender present")
            .try_send(Deadline {
                at: deadline,
                state: Arc::downgrade(&deadline_state),
            }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                // The deadline registration is a wake-up optimization, not the
                // enforcement: `JobFuture::poll` resolves the job once its
                // deadline passes, and a late completion is quarantined by the
                // caller's deadline arm. A full queue means the deadline worker
                // has not drained yet (the host is momentarily starved), not
                // that the effect failed - so the job still proceeds below and
                // a healthy launch is never spurious-failed by load. Only a
                // hung job loses its deadline wake; the poll path still bounds
                // every caller that keeps polling.
                warn!(
                    provider = "supervisor",
                    "deadline registration deferred; the deadline worker is momentarily starved"
                );
            }
            Err(TrySendError::Disconnected(_)) => {
                // The deadline worker is gone: no wake source remains for a
                // hung effect, so refusing here (as the disconnected job-channel
                // arm does) is honest instead of proceeding into an
                // unterminated wait.
                error!(
                    provider = "supervisor",
                    "deadline queue disconnected; reporting launch-failed for the blocked effect"
                );
                state.complete(Err(ProcessEffectError::LaunchFailed));
                return JobFuture { state, deadline };
            }
        }
        let submit_error = match self
            .sender
            .as_ref()
            .expect("pool sender present")
            .try_send(job)
        {
            Ok(()) => None,
            Err(TrySendError::Full(_)) => {
                debug!(
                    provider = "supervisor",
                    "blocking pool saturated; reporting busy for the process effect"
                );
                Some(ProcessEffectError::Busy)
            }
            Err(TrySendError::Disconnected(_)) => {
                error!(
                    provider = "supervisor",
                    "blocking pool disconnected; reporting launch-failed for the process effect"
                );
                Some(ProcessEffectError::LaunchFailed)
            }
        };
        if let Some(error) = submit_error {
            state.complete(Err(error));
        }
        JobFuture { state, deadline }
    }
}

impl Drop for BlockingPool {
    // Worker teardown joins the deadline worker: Drop is synchronous by
    // construction (no executor is available), and the workers finish as
    // soon as their channels close, so the join is bounded.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn drop(&mut self) {
        self.sender.take();
        self.workers.clear();
        self.deadline_sender.take();
        if let Some(worker) = self.deadline_worker.take() {
            let _ = worker.join();
        }
    }
}

// Dedicated bounded worker per plan R4: this fn runs on its own pool
// worker thread and blocks on the bounded sync_channel admission queue.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn worker(receiver: Arc<Mutex<Receiver<Job>>>) {
    loop {
        let job = {
            let Ok(receiver) = receiver.lock() else {
                return;
            };
            receiver.recv()
        };
        match job {
            Ok(job) => job(),
            Err(_) => return,
        }
    }
}

struct JobState<T> {
    result: Mutex<Option<Result<T, ProcessEffectError>>>,
    waker: Mutex<Option<Waker>>,
    completed: AtomicBool,
}

impl<T> Default for JobState<T> {
    fn default() -> Self {
        Self {
            result: Mutex::new(None),
            waker: Mutex::new(None),
            completed: AtomicBool::new(false),
        }
    }
}

impl<T> JobState<T> {
    // Synchronous by construction: the completion slot is written from the
    // bounded worker thread and from async callers' error arms, and read
    // from the sync `Future::poll` half. The Future trait cannot await, so
    // the slot must stay a std mutex; the critical sections are short and
    // never held across a suspension point.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn complete(&self, result: Result<T, ProcessEffectError>) {
        if self.completed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Ok(mut slot) = self.result.lock() {
            *slot = Some(result);
        }
        self.wake();
    }

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn wake(&self) {
        if let Ok(mut waker) = self.waker.lock()
            && let Some(waker) = waker.take()
        {
            waker.wake();
        }
    }
}

impl<T: Send> DeadlineState for JobState<T> {
    fn is_completed(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }

    fn wake_deadline(&self) {
        self.wake();
    }
}

// Dedicated bounded worker per plan R4: the deadline registrations are
// bounded by the pool's in-flight job limit (each job registers exactly one),
// and this fn blocks on the queue only on its own dedicated thread.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn deadline_worker(receiver: Receiver<Deadline>) {
    let mut deadlines = Vec::<Deadline>::new();
    loop {
        deadlines.retain(|deadline| {
            deadline
                .state
                .upgrade()
                .is_some_and(|state| !state.is_completed())
        });
        deadlines.sort_by_key(|deadline| std::cmp::Reverse(deadline.at));
        let next_wait = deadlines
            .last()
            .map(|deadline| deadline.at.saturating_duration_since(Instant::now()));
        let received = match next_wait {
            Some(wait) => receiver.recv_timeout(wait),
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match received {
            Ok(deadline) => deadlines.push(deadline),
            Err(RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                while deadlines.last().is_some_and(|deadline| deadline.at <= now) {
                    if let Some(deadline) = deadlines.pop()
                        && let Some(state) = deadline.state.upgrade()
                        && !state.is_completed()
                    {
                        state.wake_deadline();
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

struct JobFuture<T> {
    state: Arc<JobState<T>>,
    deadline: Instant,
}

impl<T> Future for JobFuture<T> {
    type Output = Result<T, ProcessEffectError>;

    // Synchronous by construction: `Future::poll` is a sync trait method
    // (no await possible), so the shared completion slot must be a std
    // mutex; the critical sections are short and never held across a
    // suspension point.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if let Ok(mut result) = self.state.result.lock()
            && let Some(result) = result.take()
        {
            return Poll::Ready(result);
        }
        if Instant::now() >= self.deadline {
            return Poll::Ready(Err(ProcessEffectError::DeadlineExceeded));
        }
        if let Ok(mut waker) = self.state.waker.lock() {
            *waker = Some(context.waker().clone());
        }
        if let Ok(mut result) = self.state.result.lock()
            && let Some(result) = result.take()
        {
            return Poll::Ready(result);
        }
        Poll::Pending
    }
}

enum LaunchOutcome {
    OnTime(BackendObservation),
    TimedOut,
}

struct RuntimeState<H> {
    handles: BTreeMap<ProcessIdentityDigest, Arc<H>>,
    launches: BTreeSet<ResourceUid>,
}

impl<H> Default for RuntimeState<H> {
    fn default() -> Self {
        Self {
            handles: BTreeMap::new(),
            launches: BTreeSet::new(),
        }
    }
}

/// The fixed core-owned implementation of [`ProcessLaunchEffectPort`].
///
/// The adapter admits at most `blocking_limit` blocking calls at once and runs
/// each admitted call on a dedicated bounded worker pool. Handles remain private in an
/// identity-keyed table; Providers receive only opaque evidence.
pub struct ProviderSupervisor<B: ProcessEffectBackend> {
    inner: Arc<Inner<B>>,
}

struct Inner<B: ProcessEffectBackend> {
    backend: Arc<B>,
    pool: BlockingPool,
    state: Arc<AsyncMutex<RuntimeState<B::Handle>>>,
    default_timeout: Duration,
}

impl<B: ProcessEffectBackend> Clone for ProviderSupervisor<B> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<B: ProcessEffectBackend> std::fmt::Debug for ProviderSupervisor<B> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProviderSupervisor(<redacted>)")
    }
}

impl<B: ProcessEffectBackend> ProviderSupervisor<B> {
    /// Build an adapter with the default blocking concurrency bound.
    pub fn new(backend: B) -> Self {
        Self::with_limits(backend, DEFAULT_BLOCKING_LIMIT, Duration::from_secs(30))
    }

    /// Build an adapter with explicit blocking concurrency and fallback timeout.
    ///
    /// A zero blocking limit is rejected because it would deadlock every call.
    pub fn with_limits(backend: B, blocking_limit: usize, default_timeout: Duration) -> Self {
        assert!(blocking_limit > 0, "blocking limit must be nonzero");
        assert!(!default_timeout.is_zero(), "timeout must be nonzero");
        Self {
            inner: Arc::new(Inner {
                backend: Arc::new(backend),
                pool: BlockingPool::new(blocking_limit),
                state: Arc::new(AsyncMutex::new(RuntimeState::default())),
                default_timeout,
            }),
        }
    }

    async fn blocking<T, F>(&self, timeout: Duration, operation: F) -> Result<T, ProcessEffectError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<B>) -> Result<T, ProcessEffectError> + Send + 'static,
    {
        let backend = Arc::clone(&self.inner.backend);
        self.inner
            .pool
            .submit(timeout, move || operation(backend))
            .await
    }

    /// Finalize one exact process identity after observation says it exited.
    ///
    /// The local handle remains private to the supervisor while the backend
    /// removes its broker or service-manager registration. Only after that
    /// effect succeeds is the identity forgotten from the supervisor table.
    pub async fn finalize_identity(
        &self,
        identity: &ProcessIdentityDigest,
    ) -> Result<(), ProcessConformanceError> {
        let handle = self.handle(identity).await.map_err(map_error)?;
        let finalize_handle = Arc::clone(&handle);
        let finalize_identity = *identity;
        let state = Arc::clone(&self.inner.state);
        // The closure runs on the dedicated blocking worker, so the async
        // mutex is taken with its synchronous blocking variant there.
        let result = self
            .blocking(self.inner.default_timeout, move |backend| {
                let result = backend.finalize(finalize_handle.as_ref());
                if result.is_ok() || result == Err(ProcessEffectError::Vanished){
                    // Dedicated blocking worker thread: blocking here parks only
                    // this worker, never an executor thread.
                    let mut state = state.blocking_lock();
                    if state
                        .handles
                        .get(&finalize_identity)
                        .is_some_and(|retained| Arc::ptr_eq(retained, &finalize_handle))
                    {
                        state.handles.remove(&finalize_identity);
                    }
                }
                result
            })
            .await;
        match result {
            Ok(()) | Err(ProcessEffectError::Vanished) => Ok(()),
            Err(error) => Err(map_error(error)),
        }
    }

    /// Take the Provider-controller bootstrap endpoint retained with one handle.
    pub async fn take_controller_bootstrap(
        &self,
        identity: &ProcessIdentityDigest,
    ) -> Result<Option<std::os::fd::OwnedFd>, ProcessConformanceError> {
        let handle = self.handle(identity).await.map_err(map_error)?;
        self.blocking(self.inner.default_timeout, move |backend| {
            backend.take_controller_bootstrap(handle.as_ref())
        })
        .await
        .map_err(map_error)
    }

    async fn remember(
        &self,
        identity: ProcessIdentityDigest,
        handle: B::Handle,
    ) -> Result<(), ProcessEffectError> {
        self.inner
            .state
            .lock()
            .await
            .handles
            .insert(identity, Arc::new(handle));
        Ok(())
    }

    async fn begin_launch(&self, ticket: &LaunchTicket) -> Result<ResourceUid, ProcessEffectError> {
        let operation_uid = ticket.operation().operation_uid().clone();
        let mut state = self
            .inner
            .state
            .lock()
            .await;
        if !state.launches.insert(operation_uid.clone()) {
            return Err(ProcessEffectError::Busy);
        }
        Ok(operation_uid)
    }

    async fn quarantine_launch(&self, operation_uid: &ResourceUid) -> Result<bool, ProcessEffectError> {
        let state = self
            .inner
            .state
            .lock()
            .await;
        Ok(state.launches.contains(operation_uid))
    }

    async fn finish_launch_success(&self, operation_uid: &ResourceUid) -> Result<(), ProcessEffectError> {
        self.inner
            .state
            .lock()
            .await
            .launches
            .remove(operation_uid);
        Ok(())
    }

    async fn handle(
        &self,
        identity: &ProcessIdentityDigest,
    ) -> Result<Arc<B::Handle>, ProcessEffectError> {
        self.inner
            .state
            .lock()
            .await
            .handles
            .get(identity)
            .cloned()
            .ok_or(ProcessEffectError::Vanished)
    }

    async fn quarantine_handle(
        &self,
        identity: ProcessIdentityDigest,
        handle: &Arc<B::Handle>,
    ) -> Result<bool, ProcessEffectError> {
        let state = self
            .inner
            .state
            .lock()
            .await;
        Ok(state
            .handles
            .get(&identity)
            .is_some_and(|retained| Arc::ptr_eq(retained, handle)))
    }

    async fn launch_with_timeout(
        &self,
        ticket: &LaunchTicket,
        request: ProcessLaunchRequest,
        timeout: Duration,
    ) -> Result<LaunchedProcess, ProcessConformanceError> {
        let operation_uid = self.begin_launch(ticket).await.map_err(map_error)?;
        let backend = Arc::clone(&self.inner.backend);
        let state = Arc::clone(&self.inner.state);
        let worker_operation_uid = operation_uid.clone();
        let outcome = self
            .inner
            .pool
            .submit_with_deadline(timeout, move |deadline| {
                let launch = backend.launch_with_inherited_fds(request);
                let late = Instant::now() >= deadline;
                match (launch, late) {
                    (Err(error), late) => {
                        state
                            .blocking_lock()
                            .launches
                            .remove(&worker_operation_uid);
                        if late {
                            Err(ProcessEffectError::DeadlineExceeded)
                        } else {
                            Err(error)
                        }
                    }
                    (Ok(launch), false) => {
                        let (observation, handle) = launch.into_parts();
                        let identity = observation.identity();
                        state
                            .blocking_lock()
                            .handles
                            .insert(identity, Arc::new(handle));
                        Ok(LaunchOutcome::OnTime(observation))
                    }
                    (Ok(launch), true) => {
                        let (observation, handle) = launch.into_parts();
                        let identity = observation.identity();
                        let handle = Arc::new(handle);
                        state
                            .blocking_lock()
                            .handles
                            .insert(identity, Arc::clone(&handle));
                        match backend.stop(handle.as_ref(), ProcessStopClass::Terminate) {
                            Ok(()) | Err(ProcessEffectError::Vanished) => {
                                let mut state = state.blocking_lock();
                                if state
                                    .handles
                                    .get(&identity)
                                    .is_some_and(|retained| Arc::ptr_eq(retained, &handle))
                                {
                                    state.handles.remove(&identity);
                                }
                                state.launches.remove(&worker_operation_uid);
                                Ok(LaunchOutcome::TimedOut)
                            }
                            Err(stop_error) => {
                                error!(
                                    provider = "supervisor",
                                    identity = identity.to_hex(),
                                    stop_error = ?stop_error,
                                    "stop of timed-out launch failed; process fate unknown"
                                );
                                Err(ProcessEffectError::FateUnknown)
                            }
                        }
                    }
                }
            })
            .await;
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(ProcessEffectError::DeadlineExceeded) => {
                warn!(
                    provider = "supervisor",
                    "launch effect exceeded its deadline"
                );
                return if self.quarantine_launch(&operation_uid).await.map_err(map_error)? {
                    warn!(
                        provider = "supervisor",
                        "late launch quarantined; reporting adoption-ambiguous"
                    );
                    Err(ProcessConformanceError::AdoptionAmbiguous)
                } else {
                    Err(ProcessConformanceError::DeadlineExceeded)
                };
            }
            Err(error) => return Err(map_error(error)),
        };
        let observation = match outcome {
            LaunchOutcome::TimedOut => return Err(ProcessConformanceError::DeadlineExceeded),
            LaunchOutcome::OnTime(observation) => observation,
        };
        self.finish_launch_success(&operation_uid)
            .await
            .map_err(map_error)?;
        let identity = observation.identity();
        Ok(LaunchedProcess {
            identity,
            observed: observation.observed().clone(),
            pidfd: PidfdEvidence::held(),
            wait_reap_owner: observation.wait_reap_owner(),
        })
    }
}

impl<B: ProcessEffectBackend> ProcessLaunchEffectPort for ProviderSupervisor<B> {
    async fn launch(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<LaunchedProcess, ProcessConformanceError> {
        self.launch_with_inherited_fds(ticket, Vec::new()).await
    }

    async fn launch_with_inherited_fds(
        &self,
        ticket: &LaunchTicket,
        inherited_fds: Vec<std::os::fd::OwnedFd>,
    ) -> Result<LaunchedProcess, ProcessConformanceError> {
        let timeout = Duration::from_millis(u64::from(ticket.operation().deadline_ms()));
        let request = ProcessLaunchRequest::new(ProcessRequest::new(ticket.clone()), inherited_fds)
            .map_err(|_| {
                warn!(
                    provider = "supervisor",
                    "launch request rejected as invalid ticket"
                );
                ProcessConformanceError::InvalidTicket
            })?;
        self.launch_with_timeout(ticket, request, timeout).await
    }

    async fn observe(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<Option<AdoptionCandidate>, ProcessConformanceError> {
        let request = ProcessRequest::new(ticket.clone());
        let timeout = Duration::from_millis(u64::from(ticket.operation().deadline_ms()));
        let observation = self
            .blocking(timeout, move |backend| backend.observe(request))
            .await
            .map_err(map_error)?;
        Ok(observation.map(|observation| AdoptionCandidate {
            identity: observation.identity(),
            observed: observation.observed().clone(),
            wait_reap_owner: observation.wait_reap_owner(),
        }))
    }

    async fn probe(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<Option<AdoptionCandidate>, ProcessConformanceError> {
        let request = ProcessRequest::new(ticket.clone());
        let timeout = Duration::from_millis(u64::from(ticket.operation().deadline_ms()));
        let observation = self
            .blocking(timeout, move |backend| backend.probe(request))
            .await
            .map_err(map_error)?;
        Ok(observation.map(|observation| AdoptionCandidate {
            identity: observation.identity(),
            observed: observation.observed().clone(),
            wait_reap_owner: observation.wait_reap_owner(),
        }))
    }

    async fn open_pidfd(
        &self,
        candidate: &AdoptionCandidate,
    ) -> Result<PidfdEvidence, ProcessConformanceError> {
        let observation = BackendObservation::new(
            candidate.identity,
            candidate.observed.clone(),
            candidate.wait_reap_owner,
        );
        let handle = self
            .blocking(self.inner.default_timeout, move |backend| {
                backend.open_pidfd(observation)
            })
            .await
            .map_err(map_error)?;
        self.remember(candidate.identity, handle)
            .await
            .map_err(map_error)?;
        Ok(PidfdEvidence::held())
    }

    async fn stop(
        &self,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> Result<(), ProcessConformanceError> {
        let handle = self.handle(identity).await.map_err(map_error)?;
        let backend_class = match class {
            StopClass::Drain => ProcessStopClass::Drain,
            StopClass::Terminate => ProcessStopClass::Terminate,
        };
        let stop_handle = Arc::clone(&handle);
        if class == StopClass::Terminate {
            let backend = Arc::clone(&self.inner.backend);
            let state = Arc::clone(&self.inner.state);
            let stop_identity = *identity;
            let result = self
                .inner
                .pool
                .submit_with_deadline(self.inner.default_timeout, move |deadline| {
                    let result = backend.stop(stop_handle.as_ref(), backend_class);
                    let late = Instant::now() >= deadline;
                    if matches!(result, Ok(()) | Err(ProcessEffectError::Vanished)) {
                        let mut state = state.blocking_lock();
                        if state
                            .handles
                            .get(&stop_identity)
                            .is_some_and(|retained| Arc::ptr_eq(retained, &stop_handle))
                        {
                            state.handles.remove(&stop_identity);
                        }
                        return if late {
                            Err(ProcessEffectError::DeadlineExceeded)
                        } else {
                            Ok(())
                        };
                    }
                    if late {
                        error!(
                            provider = "supervisor",
                            identity = stop_identity.to_hex(),
                            "late stop could not confirm termination; process fate unknown"
                        );
                        return Err(ProcessEffectError::FateUnknown);
                    }
                    result
                })
                .await;
            if matches!(result, Err(ProcessEffectError::DeadlineExceeded))
                && self
                    .quarantine_handle(*identity, &handle)
                    .await
                    .map_err(map_error)?
            {
                warn!(
                    provider = "supervisor",
                    identity = identity.to_hex(),
                    "stop exceeded its deadline; identity quarantined as adoption-ambiguous"
                );
                return Err(ProcessConformanceError::AdoptionAmbiguous);
            }
            return result.map_err(map_error);
        }
        self.blocking(self.inner.default_timeout, move |backend| {
            backend.stop(stop_handle.as_ref(), backend_class)
        })
        .await
        .map_err(map_error)
    }
}

impl<R: BrokerLaunchResolver> ProviderSupervisor<BrokerProcessBackend<R>> {
    /// Verify that a peer PID still names the exact process represented by a
    /// retained broker pidfd and opaque process identity.
pub fn matches_peer_process(
        &self,
        identity: &ProcessIdentityDigest,
        peer_pid: i32,
    ) -> Result<bool, ProcessConformanceError> {
        // Sync public surface with no async form: the runtime state lock is
        // taken fail-closed via try_lock rather than blocking an executor worker.
        let state = match self.inner.state.try_lock() {
            Ok(state) => state,
            Err(_) => return Err(ProcessConformanceError::DeadlineExceeded),
        };
        let handle = state
            .handles
            .get(identity)
            .cloned()
            .ok_or(ProcessConformanceError::PidfdUnavailable)?;
        drop(state);
        self.inner
            .backend
            .matches_peer_process(handle.as_ref(), peer_pid)
            .map_err(map_error)
    }
}

fn map_error(error: ProcessEffectError) -> ProcessConformanceError {
    match error {
        ProcessEffectError::WaitOwnerMismatch => ProcessConformanceError::WaitOwnerMismatch,
        ProcessEffectError::IdentityChanged | ProcessEffectError::FateUnknown => {
            ProcessConformanceError::AdoptionAmbiguous
        }
        ProcessEffectError::PidfdUnavailable | ProcessEffectError::Vanished => {
            ProcessConformanceError::PidfdUnavailable
        }
        ProcessEffectError::DeadlineExceeded | ProcessEffectError::Busy => {
            ProcessConformanceError::DeadlineExceeded
        }
        // Trusted launch configuration the ticket names but the bundle holds
        // no matching intent for: nothing was launched and no observed
        // identity is in question, so the refusal keeps its own code. Folding
        // it into `LaunchFailed` made the Process driver re-enter its
        // budgeted retry arm for a refusal no retry can reverse (the
        // terminal `resolution-failed` spelling never reached
        // `provider_error_kind`).
        ProcessEffectError::ResolutionFailed => ProcessConformanceError::ResolutionFailed,
        ProcessEffectError::UnsupportedProvider
        | ProcessEffectError::LaunchFailed
        | ProcessEffectError::ObserveFailed
        | ProcessEffectError::StopFailed => ProcessConformanceError::LaunchFailed,
        _ => ProcessConformanceError::LaunchFailed,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU8};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{LazyLock, Mutex, MutexGuard};

    use d2b_process_conformance::testing::{block_on, fixtures};
    use d2b_provider_process::{IdentityBinding, ObservedIdentity, WaitReapOwner};

    use super::*;

    /// Serializes the tests that drive a `ProviderSupervisor` blocking pool.
    ///
    /// The test harness's `block_on` is a noop-waker spin loop. Each test
    /// builds its own supervisor with its own bounded deadline queue, but
    /// running several spin loops in parallel starves the per-pool deadline
    /// worker threads; a burst of rapid submits then overflows the queue and
    /// a healthy launch spuriously reports `LaunchFailed`. The lock keeps
    /// the pool-driving tests sequential so each pool's worker drains on
    /// time (the broker registry-guard precedent; the pools themselves have
    /// no shared state - only CPU contention is serialized away).
    struct PoolTestGuard {
        _lock: MutexGuard<'static, ()>,
    }

    impl PoolTestGuard {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn new() -> Self {
            static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
            Self {
                _lock: LOCK
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            }
        }
    }

    /// A resolution refusal is not an ambiguous identity: the effect ports
    /// project `ResolutionFailed` under its own code, so an adopt probe the
    /// trusted-intent fence refuses reaches the Process driver as the
    /// terminal `resolution-failed` and is neither quarantined as an
    /// ambiguous identity (R15 is about observed identity) nor retried under
    /// the restart budget. Observed identity drift keeps `AdoptionAmbiguous`.
    #[test]
    fn resolution_refusals_are_not_adoption_ambiguity() {
        assert_eq!(
            map_error(ProcessEffectError::ResolutionFailed),
            ProcessConformanceError::ResolutionFailed
        );
        assert_eq!(
            map_error(ProcessEffectError::ResolutionFailed).code(),
            "resolution-failed"
        );
        assert_eq!(
            map_error(ProcessEffectError::IdentityChanged),
            ProcessConformanceError::AdoptionAmbiguous
        );
    }

    struct ControlledBackend {
        started: Mutex<Option<Sender<()>>>,
        release: Mutex<Receiver<()>>,
        live: Arc<AtomicBool>,
        stop_fails: bool,
        next_identity: AtomicU8,
    }

    impl ControlledBackend {
        fn observation(&self) -> BackendObservation {
            let seed = self.next_identity.fetch_add(1, Ordering::Relaxed);
            BackendObservation::new(
                ProcessIdentityDigest::from_bytes([seed; 32]),
                ObservedIdentity::from_verified([IdentityBinding::Cgroup]),
                WaitReapOwner::Local,
            )
        }
    }

    impl ProcessEffectBackend for ControlledBackend {
        type Handle = ();

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn launch(
            &self,
            _request: ProcessRequest,
        ) -> Result<d2b_provider_process::BackendLaunch<Self::Handle>, ProcessEffectError> {
            self.live.store(true, Ordering::Release);
            if let Some(started) = self.started.lock().unwrap().take() {
                started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
            }
            Ok(d2b_provider_process::BackendLaunch::new(
                self.observation(),
                (),
            ))
        }

        fn observe(
            &self,
            _request: ProcessRequest,
        ) -> Result<Option<BackendObservation>, ProcessEffectError> {
            Ok(None)
        }

        fn open_pidfd(
            &self,
            _observation: BackendObservation,
        ) -> Result<Self::Handle, ProcessEffectError> {
            Ok(())
        }

        fn stop(
            &self,
            _handle: &Self::Handle,
            _class: ProcessStopClass,
        ) -> Result<(), ProcessEffectError> {
            if self.stop_fails {
                return Err(ProcessEffectError::StopFailed);
            }
            self.live.store(false, Ordering::Release);
            Ok(())
        }
    }

    fn controlled_backend(
        stop_fails: bool,
    ) -> (ControlledBackend, Receiver<()>, Sender<()>, Arc<AtomicBool>) {
        let (started_sender, started_receiver) = channel();
        let (release_sender, release_receiver) = channel();
        let live = Arc::new(AtomicBool::new(false));
        (
            ControlledBackend {
                started: Mutex::new(Some(started_sender)),
                release: Mutex::new(release_receiver),
                live: Arc::clone(&live),
                stop_fails,
                next_identity: AtomicU8::new(1),
            },
            started_receiver,
            release_sender,
            live,
        )
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
        let deadline = Instant::now() + timeout;
        while !predicate() {
            assert!(Instant::now() < deadline, "condition did not become true");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn timed_out_launch_is_quarantined_until_late_cleanup_succeeds() {
        let _guard = PoolTestGuard::new();
        let (backend, started, release, live) = controlled_backend(false);
        let supervisor = ProviderSupervisor::new(backend);
        let worker_supervisor = supervisor.clone();
        let (result_sender, result_receiver) = channel();
        let thread = std::thread::spawn(move || {
            let ticket = fixtures::ticket_builder().build().unwrap();
            let request = ProcessLaunchRequest::empty(ProcessRequest::new(ticket.clone())).unwrap();
            result_sender
                .send(block_on(worker_supervisor.launch_with_timeout(
                    &ticket,
                    request,
                    Duration::from_millis(10),
                )))
                .unwrap();
        });

        started.recv().unwrap();
        assert!(live.load(Ordering::Acquire));
        assert_eq!(
            result_receiver
                .recv_timeout(Duration::from_millis(250))
                .unwrap()
                .unwrap_err(),
            ProcessConformanceError::AdoptionAmbiguous
        );
        thread.join().unwrap();
        {
            let state = supervisor.inner.state.blocking_lock();
            assert_eq!(state.launches.len(), 1);
        }
        release.send(()).unwrap();
        wait_until(Duration::from_millis(250), || !live.load(Ordering::Acquire));
        let state = supervisor.inner.state.blocking_lock();
        assert!(state.launches.is_empty());
        assert!(state.handles.is_empty());
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn a_late_launch_cleanup_failure_stays_quarantined_and_tracked() {
        let _guard = PoolTestGuard::new();
        let (backend, started, release, live) = controlled_backend(true);
        let supervisor = ProviderSupervisor::new(backend);
        let worker_supervisor = supervisor.clone();
        let (result_sender, result_receiver) = channel();
        let thread = std::thread::spawn(move || {
            let ticket = fixtures::ticket_builder().build().unwrap();
            let request = ProcessLaunchRequest::empty(ProcessRequest::new(ticket.clone())).unwrap();
            result_sender
                .send(block_on(worker_supervisor.launch_with_timeout(
                    &ticket,
                    request,
                    Duration::from_millis(10),
                )))
                .unwrap();
        });

        started.recv().unwrap();
        assert_eq!(
            result_receiver
                .recv_timeout(Duration::from_millis(250))
                .unwrap()
                .unwrap_err(),
            ProcessConformanceError::AdoptionAmbiguous
        );
        thread.join().unwrap();
        assert!(live.load(Ordering::Acquire));
        release.send(()).unwrap();
        wait_until(Duration::from_millis(250), || {
            !supervisor.inner.state.blocking_lock().handles.is_empty()
        });
        let state = supervisor.inner.state.blocking_lock();
        assert_eq!(state.launches.len(), 1);
        assert_eq!(state.handles.len(), 1);
    }

    struct HungStopBackend {
        stop_started: Mutex<Option<Sender<()>>>,
        launch_delay: Duration,
        live: Arc<AtomicBool>,
    }

    impl ProcessEffectBackend for HungStopBackend {
        type Handle = ();

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn launch(
            &self,
            _request: ProcessRequest,
        ) -> Result<d2b_provider_process::BackendLaunch<Self::Handle>, ProcessEffectError> {
            if !self.launch_delay.is_zero() {
                std::thread::sleep(self.launch_delay);
            }
            self.live.store(true, Ordering::Release);
            Ok(d2b_provider_process::BackendLaunch::new(
                BackendObservation::new(
                    ProcessIdentityDigest::from_bytes([9; 32]),
                    ObservedIdentity::from_verified([IdentityBinding::Cgroup]),
                    WaitReapOwner::Local,
                ),
                (),
            ))
        }

        fn observe(
            &self,
            _request: ProcessRequest,
        ) -> Result<Option<BackendObservation>, ProcessEffectError> {
            Ok(None)
        }

        fn open_pidfd(
            &self,
            _observation: BackendObservation,
        ) -> Result<Self::Handle, ProcessEffectError> {
            Ok(())
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn stop(
            &self,
            _handle: &Self::Handle,
            _class: ProcessStopClass,
        ) -> Result<(), ProcessEffectError> {
            if let Some(started) = self.stop_started.lock().unwrap().take() {
                started.send(()).unwrap();
            }
            loop {
                std::thread::park();
            }
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn hung_late_launch_cleanup_is_bounded_and_quarantined() {
        let _guard = PoolTestGuard::new();
        let (stop_started_sender, stop_started_receiver) = channel();
        let live = Arc::new(AtomicBool::new(false));
        let supervisor = ProviderSupervisor::with_limits(
            HungStopBackend {
                stop_started: Mutex::new(Some(stop_started_sender)),
                launch_delay: Duration::from_millis(20),
                live: Arc::clone(&live),
            },
            1,
            Duration::from_millis(10),
        );
        let ticket = fixtures::ticket_builder().build().unwrap();
        let worker_supervisor = supervisor.clone();
        let (result_sender, result_receiver) = channel();
        std::thread::spawn(move || {
            result_sender
                .send(block_on(worker_supervisor.launch_with_timeout(
                    &ticket,
                    ProcessLaunchRequest::empty(ProcessRequest::new(ticket.clone())).unwrap(),
                    Duration::from_millis(10),
                )))
                .unwrap();
        });

        assert_eq!(
            result_receiver
                .recv_timeout(Duration::from_millis(250))
                .unwrap()
                .unwrap_err(),
            ProcessConformanceError::AdoptionAmbiguous
        );
        stop_started_receiver
            .recv_timeout(Duration::from_millis(250))
            .unwrap();
        assert!(live.load(Ordering::Acquire));
        let state = supervisor.inner.state.blocking_lock();
        assert_eq!(state.launches.len(), 1);
        assert_eq!(state.handles.len(), 1);
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[test]
    fn hung_terminate_is_bounded_and_quarantined() {
        let _guard = PoolTestGuard::new();
        let (stop_started_sender, stop_started_receiver) = channel();
        let supervisor = ProviderSupervisor::with_limits(
            HungStopBackend {
                stop_started: Mutex::new(Some(stop_started_sender)),
                launch_delay: Duration::ZERO,
                live: Arc::new(AtomicBool::new(false)),
            },
            1,
            Duration::from_millis(10),
        );
        let ticket = fixtures::ticket_builder().build().unwrap();
        let launched = block_on(supervisor.launch(&ticket)).unwrap();
        let worker_supervisor = supervisor.clone();
        let identity = launched.identity;
        let (result_sender, result_receiver) = channel();
        std::thread::spawn(move || {
            result_sender
                .send(block_on(
                    worker_supervisor.stop(&identity, StopClass::Terminate),
                ))
                .unwrap();
        });

        assert_eq!(
            result_receiver
                .recv_timeout(Duration::from_millis(250))
                .unwrap()
                .unwrap_err(),
            ProcessConformanceError::AdoptionAmbiguous
        );
        stop_started_receiver
            .recv_timeout(Duration::from_millis(250))
            .unwrap();
        let state = supervisor.inner.state.blocking_lock();
        assert!(state.handles.contains_key(&launched.identity));
    }

    #[test]
    fn terminal_stops_retire_retained_handles() {
        let _guard = PoolTestGuard::new();
        let (_unused_sender, release_receiver) = channel();
        let supervisor = ProviderSupervisor::new(ControlledBackend {
            started: Mutex::new(None),
            release: Mutex::new(release_receiver),
            live: Arc::new(AtomicBool::new(false)),
            stop_fails: false,
            next_identity: AtomicU8::new(1),
        });
        let ticket = fixtures::ticket_builder().build().unwrap();

        for _ in 0..64 {
            let launched = block_on(supervisor.launch(&ticket)).unwrap();
            block_on(supervisor.stop(&launched.identity, StopClass::Terminate)).unwrap();
            assert!(supervisor.inner.state.blocking_lock().handles.is_empty());
        }
    }

    #[test]
    fn terminal_finalization_retires_a_naturally_exited_handle() {
        let _guard = PoolTestGuard::new();
        let (_unused_sender, release_receiver) = channel();
        let supervisor = ProviderSupervisor::new(ControlledBackend {
            started: Mutex::new(None),
            release: Mutex::new(release_receiver),
            live: Arc::new(AtomicBool::new(false)),
            stop_fails: false,
            next_identity: AtomicU8::new(1),
        });
        let ticket = fixtures::ticket_builder().build().unwrap();
        let launched = block_on(supervisor.launch(&ticket)).unwrap();
        block_on(supervisor.finalize_identity(&launched.identity)).unwrap();
        assert!(supervisor.inner.state.blocking_lock().handles.is_empty());
    }

    struct ProbeOnlyBackend {
        observe_calls: Arc<AtomicU8>,
        probe_calls: Arc<AtomicU8>,
    }

    impl ProcessEffectBackend for ProbeOnlyBackend {
        type Handle = ();

        fn launch(
            &self,
            _request: ProcessRequest,
        ) -> Result<d2b_provider_process::BackendLaunch<Self::Handle>, ProcessEffectError> {
            unreachable!("probe-only backend is not used for launch")
        }

        fn observe(
            &self,
            _request: ProcessRequest,
        ) -> Result<Option<BackendObservation>, ProcessEffectError> {
            self.observe_calls.fetch_add(1, Ordering::Relaxed);
            Ok(None)
        }

        fn probe(
            &self,
            _request: ProcessRequest,
        ) -> Result<Option<BackendObservation>, ProcessEffectError> {
            self.probe_calls.fetch_add(1, Ordering::Relaxed);
            Ok(None)
        }

        fn open_pidfd(
            &self,
            _observation: BackendObservation,
        ) -> Result<Self::Handle, ProcessEffectError> {
            unreachable!("probe-only backend is not used for pidfd opens")
        }

        fn stop(
            &self,
            _handle: &Self::Handle,
            _class: ProcessStopClass,
        ) -> Result<(), ProcessEffectError> {
            unreachable!("probe-only backend is not used for stops")
        }
    }

    #[test]
    fn probe_uses_the_non_mutating_backend_seam() {
        let observe_calls = Arc::new(AtomicU8::new(0));
        let probe_calls = Arc::new(AtomicU8::new(0));
        let supervisor = ProviderSupervisor::new(ProbeOnlyBackend {
            observe_calls: Arc::clone(&observe_calls),
            probe_calls: Arc::clone(&probe_calls),
        });
        let ticket = fixtures::ticket_builder().build().unwrap();

        assert_eq!(block_on(supervisor.probe(&ticket)).unwrap(), None);
        assert_eq!(probe_calls.load(Ordering::Relaxed), 1);
        assert_eq!(observe_calls.load(Ordering::Relaxed), 0);
    }
}
