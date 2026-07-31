//! Bounded async adapter for the blocking process effect owner.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{
    Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError, channel, sync_channel,
};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Waker};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use d2b_process::{
    AdoptionCandidate, BackendObservation, LaunchTicket, LaunchedProcess, PidfdEvidence,
    ProcessConformanceError, ProcessEffectBackend, ProcessEffectError, ProcessIdentityDigest,
    ProcessLaunchEffectPort, ProcessRequest, ProcessStopClass, StopClass,
};
/// Default upper bound for concurrent blocking process effects.
pub const DEFAULT_BLOCKING_LIMIT: usize = 16;

type Job = Box<dyn FnOnce() + Send + 'static>;

struct BlockingPool {
    sender: Option<SyncSender<Job>>,
    workers: Vec<JoinHandle<()>>,
    deadline_sender: Option<Sender<Deadline>>,
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
        let (deadline_sender, deadline_receiver) = channel();
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
        let state = Arc::new(JobState::default());
        let worker_state = Arc::clone(&state);
        let job = Box::new(move || worker_state.complete(operation()));
        let submit_error = match self
            .sender
            .as_ref()
            .expect("pool sender present")
            .try_send(job)
        {
            Ok(()) => None,
            Err(TrySendError::Full(_)) => Some(ProcessEffectError::Busy),
            Err(TrySendError::Disconnected(_)) => Some(ProcessEffectError::LaunchFailed),
        };
        if let Some(error) = submit_error {
            state.complete(Err(error));
        }
        let deadline = Instant::now() + timeout;
        let deadline_state: Arc<dyn DeadlineState> = state.clone();
        if self
            .deadline_sender
            .as_ref()
            .expect("deadline sender present")
            .send(Deadline {
                at: deadline,
                state: Arc::downgrade(&deadline_state),
            })
            .is_err()
        {
            state.complete(Err(ProcessEffectError::LaunchFailed));
        }
        JobFuture { state, deadline }
    }
}

impl Drop for BlockingPool {
    fn drop(&mut self) {
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        self.deadline_sender.take();
        if let Some(worker) = self.deadline_worker.take() {
            let _ = worker.join();
        }
    }
}

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
    fn complete(&self, result: Result<T, ProcessEffectError>) {
        if let Ok(mut slot) = self.result.lock() {
            *slot = Some(result);
        }
        self.completed.store(true, Ordering::Release);
        self.wake();
    }

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
                    if let Some(deadline) = deadlines.pop() {
                        if let Some(state) = deadline.state.upgrade()
                            && !state.is_completed()
                        {
                            state.wake_deadline();
                        }
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
    handles: Mutex<BTreeMap<ProcessIdentityDigest, Arc<B::Handle>>>,
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
                handles: Mutex::new(BTreeMap::new()),
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

    fn remember(
        &self,
        identity: ProcessIdentityDigest,
        handle: B::Handle,
    ) -> Result<(), ProcessEffectError> {
        self.inner
            .handles
            .lock()
            .map_err(|_| ProcessEffectError::LaunchFailed)?
            .insert(identity, Arc::new(handle));
        Ok(())
    }

    fn handle(
        &self,
        identity: &ProcessIdentityDigest,
    ) -> Result<Arc<B::Handle>, ProcessEffectError> {
        self.inner
            .handles
            .lock()
            .map_err(|_| ProcessEffectError::StopFailed)?
            .get(identity)
            .cloned()
            .ok_or(ProcessEffectError::Vanished)
    }
}

impl<B: ProcessEffectBackend> ProcessLaunchEffectPort for ProviderSupervisor<B> {
    async fn launch(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<LaunchedProcess, ProcessConformanceError> {
        let request = ProcessRequest::new(ticket.clone());
        let timeout = Duration::from_millis(u64::from(ticket.operation().deadline_ms()));
        let launch = self
            .blocking(timeout, move |backend| backend.launch(request))
            .await
            .map_err(map_error)?;
        let (observation, handle) = launch.into_parts();
        let identity = observation.identity();
        self.remember(identity, handle).map_err(map_error)?;
        Ok(LaunchedProcess {
            identity,
            observed: observation.observed().clone(),
            pidfd: PidfdEvidence::held(),
            wait_reap_owner: observation.wait_reap_owner(),
        })
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
            .map_err(map_error)?;
        Ok(PidfdEvidence::held())
    }

    async fn stop(
        &self,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> Result<(), ProcessConformanceError> {
        let handle = self.handle(identity).map_err(map_error)?;
        let backend_class = match class {
            StopClass::Drain => ProcessStopClass::Drain,
            StopClass::Terminate => ProcessStopClass::Terminate,
        };
        self.blocking(self.inner.default_timeout, move |backend| {
            backend.stop(handle.as_ref(), backend_class)
        })
        .await
        .map_err(map_error)
    }
}

fn map_error(error: ProcessEffectError) -> ProcessConformanceError {
    match error {
        ProcessEffectError::WaitOwnerMismatch => ProcessConformanceError::WaitOwnerMismatch,
        ProcessEffectError::IdentityChanged | ProcessEffectError::ObserveFailed => {
            ProcessConformanceError::AdoptionAmbiguous
        }
        ProcessEffectError::PidfdUnavailable | ProcessEffectError::Vanished => {
            ProcessConformanceError::PidfdUnavailable
        }
        ProcessEffectError::DeadlineExceeded | ProcessEffectError::Busy => {
            ProcessConformanceError::DeadlineExceeded
        }
        ProcessEffectError::UnsupportedProvider
        | ProcessEffectError::ResolutionFailed
        | ProcessEffectError::LaunchFailed
        | ProcessEffectError::StopFailed => ProcessConformanceError::LaunchFailed,
        _ => ProcessConformanceError::LaunchFailed,
    }
}
