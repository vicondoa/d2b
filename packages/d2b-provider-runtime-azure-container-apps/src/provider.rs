#![allow(clippy::result_large_err)] // ProviderFailure is the canonical provider contract error.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    future::{Future, poll_fn},
    ops::Deref,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
        mpsc as std_mpsc,
    },
    time::Duration,
};

use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole, ServicePackage},
    v2_identity::{ProviderType, WorkloadId},
    v2_provider::{
        AdoptionRequest, AdoptionState, CgroupAuthority, CredentialLeaseState,
        CredentialLeaseTransferPolicy, DeviceMediationPosture, Generation,
        MAX_PROVIDER_LEASE_LIFETIME_MS, MAX_SAFE_JSON_INTEGER, MutationReceipt, MutationState,
        NetworkPosture, ObservationReason, ObservedLifecycleState, OperationBinding,
        PROVIDER_SCHEMA_VERSION, PersistentIdentityPosture, PlanId, PlannedResourceClass,
        ProcessAuthority, Provider, ProviderCallContext, ProviderCapability, ProviderCapabilitySet,
        ProviderContractError, ProviderDescriptor, ProviderFailure, ProviderFailureKind,
        ProviderFuture, ProviderHandle, ProviderHealth, ProviderHealthReason, ProviderHealthState,
        ProviderMethod, ProviderObservation, ProviderOperationContext, ProviderOperationRequest,
        ProviderPlacement, ProviderPlan, ProviderRemediation, ProviderResult, RetryClass,
        RuntimeProvider, SdkOperationClass, UserNamespacePosture,
    },
};
use d2b_provider::{ProviderClock, SystemProviderClock};
use d2b_provider_toolkit::ProviderValues;
use tokio::{
    runtime::Builder,
    sync::{Mutex, MutexGuard, mpsc, oneshot},
    task::JoinSet,
    time::{Instant, timeout, timeout_at},
};

#[cfg(test)]
use std::sync::Weak;

use crate::{
    control::{
        AcaControl, AcaControlContext, AcaControlError, AcaControlErrorKind, AcaControlHealth,
        AcaCredentialLease, AcaCredentialLeaseClient, AcaCredentialLeaseRequest,
        AcaCredentialPurpose, MAX_ACA_LEASE_CLEANUP_MS,
    },
    types::{
        AcaDeleteOutcome, AcaDesiredDiskImage, AcaDesiredSandbox, AcaDiskImageRecord,
        AcaDiskImageSource, AcaResourceBinding, AcaRuntimeConfig, AcaSandboxLifecycle,
        AcaSandboxRecord, AcaWorkloadQuery,
    },
};

pub const ACA_IMPLEMENTATION_ID: &str = "azure-container-apps";

const ACA_RUNTIME_METHODS: [ProviderMethod; 7] = [
    ProviderMethod::RuntimePlan,
    ProviderMethod::RuntimeEnsure,
    ProviderMethod::RuntimeStart,
    ProviderMethod::RuntimeStop,
    ProviderMethod::RuntimeInspect,
    ProviderMethod::RuntimeAdopt,
    ProviderMethod::RuntimeDestroy,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaProviderBuildError {
    Descriptor(ProviderContractError),
    WrongImplementation,
    WrongAuthority,
    WrongPlacement,
    CapabilityMismatch,
    CredentialDescriptorInvalid,
    CredentialNotColocated,
}

impl fmt::Display for AcaProviderBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Descriptor(_) => "Azure Container Apps provider descriptor is invalid",
            Self::WrongImplementation => {
                "Azure Container Apps provider implementation identifier is invalid"
            }
            Self::WrongAuthority => {
                "Azure Container Apps provider runtime authority posture is invalid"
            }
            Self::WrongPlacement => {
                "Azure Container Apps provider must run in a configured provider agent"
            }
            Self::CapabilityMismatch => {
                "Azure Container Apps provider capabilities do not match implemented behavior"
            }
            Self::CredentialDescriptorInvalid => {
                "Azure Container Apps credential provider descriptor is invalid"
            }
            Self::CredentialNotColocated => {
                "Azure Container Apps credential provider is not co-located with the runtime"
            }
        })
    }
}

impl Error for AcaProviderBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Descriptor(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ProviderContractError> for AcaProviderBuildError {
    fn from(value: ProviderContractError) -> Self {
        Self::Descriptor(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseKind {
    Plan,
    Handle,
    Observation,
    Receipt,
}

#[derive(Clone)]
enum CachedResponse {
    Plan(Box<ProviderResult<ProviderPlan>>),
    Handle(Box<ProviderResult<ProviderHandle>>),
    Observation(Box<ProviderResult<ProviderObservation>>),
    Receipt(Box<ProviderResult<MutationReceipt>>),
}

impl CachedResponse {
    const fn kind(&self) -> ResponseKind {
        match self {
            Self::Plan(_) => ResponseKind::Plan,
            Self::Handle(_) => ResponseKind::Handle,
            Self::Observation(_) => ResponseKind::Observation,
            Self::Receipt(_) => ResponseKind::Receipt,
        }
    }

    fn retry_class(&self) -> Option<RetryClass> {
        match self {
            Self::Plan(result) => result.as_ref().as_ref().err().map(|failure| failure.retry),
            Self::Handle(result) => result.as_ref().as_ref().err().map(|failure| failure.retry),
            Self::Observation(result) => {
                result.as_ref().as_ref().err().map(|failure| failure.retry)
            }
            Self::Receipt(result) => result.as_ref().as_ref().err().map(|failure| failure.retry),
        }
    }

    fn should_record(&self) -> bool {
        self.retry_class() != Some(RetryClass::SameOperation)
    }

    fn is_success(&self) -> bool {
        match self {
            Self::Plan(result) => result.is_ok(),
            Self::Handle(result) => result.is_ok(),
            Self::Observation(result) => result.is_ok(),
            Self::Receipt(result) => result.is_ok(),
        }
    }
}

#[derive(Clone)]
struct CompletedOperation {
    operation: ProviderOperationContext,
    expires_at_unix_ms: u64,
    observation_satisfied: bool,
    response: CachedResponse,
}

#[derive(Default)]
struct OperationLedger {
    completed: BTreeMap<d2b_contracts::v2_provider::OperationId, CompletedOperation>,
}

struct CallDeadline {
    at: Instant,
}

impl CallDeadline {
    fn new(context: &ProviderCallContext<'_>, now_unix_ms: u64) -> Self {
        let wall_remaining_ms = context
            .operation
            .expires_at_unix_ms
            .saturating_sub(now_unix_ms);
        let remaining_ms =
            u64::from(context.monotonic_deadline_remaining_ms).min(wall_remaining_ms);
        Self {
            at: Instant::now() + Duration::from_millis(remaining_ms),
        }
    }

    fn remaining(&self) -> Option<Duration> {
        self.at.checked_duration_since(Instant::now())
    }
}

struct ActiveCredentialLease {
    lease: Option<AcaCredentialLease>,
    client: Arc<dyn AcaCredentialLeaseClient>,
    observer: Arc<dyn LeaseCleanupObserver>,
    executor: Arc<dyn LeaseCleanupExecutor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeaseCleanupOutcome {
    Revoked,
    Timeout,
    Failed,
    RuntimeUnavailable,
    Saturated,
    Cancelled,
}

impl LeaseCleanupOutcome {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Revoked => "revoked",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
            Self::RuntimeUnavailable => "runtime-unavailable",
            Self::Saturated => "saturated",
            Self::Cancelled => "cancelled",
        }
    }
}

pub(crate) const ACA_LEASE_CLEANUP_TARGET: &str =
    "d2b_provider_runtime_azure_container_apps::credential_lease_cleanup";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LeaseCleanupEvent {
    component: &'static str,
    operation: &'static str,
    outcome: LeaseCleanupOutcome,
}

impl LeaseCleanupEvent {
    const fn new(outcome: LeaseCleanupOutcome) -> Self {
        Self {
            component: "credential-lease",
            operation: "revoke",
            outcome,
        }
    }

    pub(crate) const fn component(self) -> &'static str {
        self.component
    }

    pub(crate) const fn operation(self) -> &'static str {
        self.operation
    }

    pub(crate) const fn outcome(self) -> LeaseCleanupOutcome {
        self.outcome
    }
}

pub(crate) trait LeaseCleanupObserver: Send + Sync {
    fn observe(&self, event: LeaseCleanupEvent);
}

struct TracingLeaseCleanupObserver;

impl LeaseCleanupObserver for TracingLeaseCleanupObserver {
    fn observe(&self, event: LeaseCleanupEvent) {
        tracing::info!(
            target: ACA_LEASE_CLEANUP_TARGET,
            component = event.component(),
            operation = event.operation(),
            outcome = event.outcome().as_str(),
        );
    }
}

struct LeaseCleanupCompletion {
    observer: Arc<dyn LeaseCleanupObserver>,
}

impl LeaseCleanupCompletion {
    fn finish(self, outcome: LeaseCleanupOutcome) {
        self.observer.observe(LeaseCleanupEvent::new(outcome));
    }
}

pub(crate) struct LeaseCleanupJob {
    lease: Option<AcaCredentialLease>,
    client: Arc<dyn AcaCredentialLeaseClient>,
    completion: Option<LeaseCleanupCompletion>,
    fallback_outcome: LeaseCleanupOutcome,
}

impl LeaseCleanupJob {
    fn finish(&mut self, outcome: LeaseCleanupOutcome) {
        if let Some(completion) = self.completion.take() {
            completion.finish(outcome);
        }
    }

    fn mark_running(&mut self) {
        self.fallback_outcome = LeaseCleanupOutcome::Cancelled;
    }
}

impl Drop for LeaseCleanupJob {
    fn drop(&mut self) {
        self.finish(self.fallback_outcome);
    }
}

pub(crate) trait LeaseCleanupExecutor: Send + Sync {
    fn enqueue(&self, job: LeaseCleanupJob);
}

struct UnavailableLeaseCleanupExecutor;

impl LeaseCleanupExecutor for UnavailableLeaseCleanupExecutor {
    fn enqueue(&self, mut job: LeaseCleanupJob) {
        job.finish(LeaseCleanupOutcome::RuntimeUnavailable);
    }
}

struct ChannelLeaseCleanupExecutor {
    sender: mpsc::Sender<LeaseCleanupJob>,
    shutdown_sender: StdMutex<Option<oneshot::Sender<()>>>,
    worker: StdMutex<Option<std::thread::JoinHandle<()>>>,
    stopped: Arc<AtomicBool>,
    joined: Arc<AtomicBool>,
}

impl ChannelLeaseCleanupExecutor {
    fn start(
        queue_capacity: usize,
        max_in_flight: usize,
        shutdown_timeout: Duration,
    ) -> Option<Arc<Self>> {
        if queue_capacity == 0 || max_in_flight == 0 {
            return None;
        }
        let (sender, receiver) = mpsc::channel(queue_capacity);
        let (shutdown_sender, shutdown_receiver) = oneshot::channel();
        let (ready_sender, ready_receiver) = std_mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let worker = std::thread::Builder::new()
            .name("d2b-aca-lease-cleanup".to_owned())
            .spawn(move || {
                let runtime = Builder::new_current_thread().enable_time().build();
                let Ok(runtime) = runtime else {
                    let _result = ready_sender.send(false);
                    worker_stopped.store(true, Ordering::Release);
                    return;
                };
                if ready_sender.send(true).is_err() {
                    worker_stopped.store(true, Ordering::Release);
                    return;
                }
                runtime.block_on(run_cleanup_worker(
                    receiver,
                    shutdown_receiver,
                    max_in_flight,
                    shutdown_timeout,
                ));
                worker_stopped.store(true, Ordering::Release);
            })
            .ok()?;
        if ready_receiver.recv().ok() != Some(true) {
            let _result = worker.join();
            return None;
        }
        Some(Arc::new(Self {
            sender,
            shutdown_sender: StdMutex::new(Some(shutdown_sender)),
            worker: StdMutex::new(Some(worker)),
            stopped,
            joined: Arc::new(AtomicBool::new(false)),
        }))
    }

    fn request_shutdown(&self) {
        let sender = self
            .shutdown_sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(sender) = sender {
            let _result = sender.send(());
        }
    }

    fn shutdown_and_join(&self) {
        self.request_shutdown();
        let mut worker = self
            .worker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(handle) = worker.take() else {
            return;
        };
        if handle.thread().id() == std::thread::current().id() {
            return;
        }
        let _result = handle.join();
        self.stopped.store(true, Ordering::Release);
        self.joined.store(true, Ordering::Release);
    }

    #[cfg(test)]
    fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
}

impl LeaseCleanupExecutor for ChannelLeaseCleanupExecutor {
    fn enqueue(&self, job: LeaseCleanupJob) {
        match self.sender.try_send(job) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(mut job)) => {
                job.finish(LeaseCleanupOutcome::Saturated);
            }
            Err(mpsc::error::TrySendError::Closed(mut job)) => {
                job.finish(LeaseCleanupOutcome::RuntimeUnavailable);
            }
        }
    }
}

impl Drop for ChannelLeaseCleanupExecutor {
    fn drop(&mut self) {
        self.shutdown_and_join();
    }
}

async fn run_cleanup_worker(
    mut receiver: mpsc::Receiver<LeaseCleanupJob>,
    mut shutdown: oneshot::Receiver<()>,
    max_in_flight: usize,
    shutdown_timeout: Duration,
) {
    let mut tasks = JoinSet::new();
    let mut shutdown_requested = false;
    let mut monitor_shutdown = true;
    loop {
        if tasks.len() >= max_in_flight {
            tokio::select! {
                result = &mut shutdown, if monitor_shutdown => {
                    if result.is_ok() {
                        shutdown_requested = true;
                        break;
                    }
                    monitor_shutdown = false;
                }
                _ = tasks.join_next() => {}
            }
            continue;
        }
        tokio::select! {
            result = &mut shutdown, if monitor_shutdown => {
                if result.is_ok() {
                    shutdown_requested = true;
                    break;
                }
                monitor_shutdown = false;
            }
            job = receiver.recv() => {
                let Some(mut job) = job else {
                    break;
                };
                job.mark_running();
                tasks.spawn(run_cleanup_job(job));
            }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }

    if shutdown_requested {
        receiver.close();
        let shutdown_deadline = Instant::now() + shutdown_timeout;
        loop {
            while tasks.len() < max_in_flight {
                let Ok(mut job) = receiver.try_recv() else {
                    break;
                };
                job.mark_running();
                tasks.spawn(run_cleanup_job(job));
            }
            if tasks.is_empty() {
                return;
            }
            if timeout_at(shutdown_deadline, tasks.join_next())
                .await
                .is_err()
            {
                while let Ok(mut job) = receiver.try_recv() {
                    job.finish(LeaseCleanupOutcome::Cancelled);
                }
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return;
            }
        }
    }

    while tasks.join_next().await.is_some() {}
}

async fn run_cleanup_job(mut job: LeaseCleanupJob) {
    let Some(lease) = job.lease.take() else {
        job.finish(LeaseCleanupOutcome::Failed);
        return;
    };
    let client = Arc::clone(&job.client);
    let outcome = match timeout(
        Duration::from_millis(u64::from(MAX_ACA_LEASE_CLEANUP_MS)),
        client.revoke(&lease),
    )
    .await
    {
        Ok(Ok(())) => LeaseCleanupOutcome::Revoked,
        Ok(Err(_)) => LeaseCleanupOutcome::Failed,
        Err(_) => LeaseCleanupOutcome::Timeout,
    };
    job.finish(outcome);
}

const ACA_LEASE_CLEANUP_QUEUE_CAPACITY: usize = 128;
const ACA_LEASE_CLEANUP_MAX_IN_FLIGHT: usize = 16;
const ACA_LEASE_CLEANUP_SHUTDOWN_GRACE_MS: u64 = MAX_ACA_LEASE_CLEANUP_MS as u64 + 250;

fn lease_cleanup_executor() -> Arc<dyn LeaseCleanupExecutor> {
    if let Some(executor) = ChannelLeaseCleanupExecutor::start(
        ACA_LEASE_CLEANUP_QUEUE_CAPACITY,
        ACA_LEASE_CLEANUP_MAX_IN_FLIGHT,
        Duration::from_millis(ACA_LEASE_CLEANUP_SHUTDOWN_GRACE_MS),
    ) {
        executor
    } else {
        Arc::new(UnavailableLeaseCleanupExecutor)
    }
}

impl ActiveCredentialLease {
    fn new(
        lease: AcaCredentialLease,
        client: Arc<dyn AcaCredentialLeaseClient>,
        observer: Arc<dyn LeaseCleanupObserver>,
        executor: Arc<dyn LeaseCleanupExecutor>,
    ) -> Self {
        Self {
            lease: Some(lease),
            client,
            observer,
            executor,
        }
    }

    fn start_revoke(&mut self) -> Result<(), ()> {
        let lease = self.lease.take().ok_or(())?;
        self.executor.enqueue(LeaseCleanupJob {
            lease: Some(lease),
            client: Arc::clone(&self.client),
            completion: Some(LeaseCleanupCompletion {
                observer: Arc::clone(&self.observer),
            }),
            fallback_outcome: LeaseCleanupOutcome::RuntimeUnavailable,
        });
        Ok(())
    }
}

impl Deref for ActiveCredentialLease {
    type Target = AcaCredentialLease;

    fn deref(&self) -> &Self::Target {
        let Some(lease) = self.lease.as_ref() else {
            unreachable!()
        };
        lease
    }
}

impl Drop for ActiveCredentialLease {
    fn drop(&mut self) {
        let _cleanup = self.start_revoke();
    }
}

#[cfg(test)]
pub(crate) fn drop_lease_after_request_runtime_shutdown_for_test(
    lease: AcaCredentialLease,
    client: Arc<dyn AcaCredentialLeaseClient>,
    observer: Arc<dyn LeaseCleanupObserver>,
    executor: Arc<dyn LeaseCleanupExecutor>,
) {
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread().enable_time().build().unwrap();
        let active = {
            let _runtime_guard = runtime.enter();
            ActiveCredentialLease::new(lease, client, observer, executor)
        };
        drop(runtime);
        drop(active);
    })
    .join()
    .unwrap();
}

#[cfg(test)]
pub(crate) fn lease_cleanup_executor_for_test(
    queue_capacity: usize,
    max_in_flight: usize,
) -> Arc<dyn LeaseCleanupExecutor> {
    match ChannelLeaseCleanupExecutor::start(
        queue_capacity,
        max_in_flight,
        Duration::from_millis(ACA_LEASE_CLEANUP_SHUTDOWN_GRACE_MS),
    ) {
        Some(value) => value,
        None => Arc::new(UnavailableLeaseCleanupExecutor),
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct LeaseCleanupExecutorControl {
    executor: Weak<ChannelLeaseCleanupExecutor>,
    stopped: Arc<AtomicBool>,
    joined: Arc<AtomicBool>,
}

#[cfg(test)]
impl LeaseCleanupExecutorControl {
    pub(crate) fn request_shutdown(&self) {
        if let Some(executor) = self.executor.upgrade() {
            executor.request_shutdown();
        }
    }

    pub(crate) fn shutdown_and_join(&self) {
        if let Some(executor) = self.executor.upgrade() {
            executor.shutdown_and_join();
        }
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.executor
            .upgrade()
            .is_none_or(|executor| executor.is_closed())
    }

    pub(crate) fn is_joined(&self) -> bool {
        self.joined.load(Ordering::Acquire)
    }
}

#[cfg(test)]
pub(crate) fn controlled_lease_cleanup_executor_for_test(
    queue_capacity: usize,
    max_in_flight: usize,
    shutdown_timeout: Duration,
) -> Option<(Arc<dyn LeaseCleanupExecutor>, LeaseCleanupExecutorControl)> {
    let executor =
        ChannelLeaseCleanupExecutor::start(queue_capacity, max_in_flight, shutdown_timeout)?;
    let control = LeaseCleanupExecutorControl {
        executor: Arc::downgrade(&executor),
        stopped: Arc::clone(&executor.stopped),
        joined: Arc::clone(&executor.joined),
    };
    Some((executor, control))
}

#[cfg(test)]
pub(crate) fn unavailable_lease_cleanup_executor_for_test() -> Arc<dyn LeaseCleanupExecutor> {
    Arc::new(UnavailableLeaseCleanupExecutor)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingMismatch {
    Identity,
    Configuration,
    Generation,
}

#[derive(Debug, Clone, Copy)]
struct ObservationStatus {
    lifecycle: ObservedLifecycleState,
    adoption: AdoptionState,
    reason: ObservationReason,
    health_state: ProviderHealthState,
    health_reason: ProviderHealthReason,
    remediation: ProviderRemediation,
}

struct RunningPoll<'a> {
    query: &'a AcaWorkloadQuery,
    expected: &'a AcaResourceBinding,
    created_by: &'a OperationBinding,
    mutation_started: bool,
}

#[derive(Clone, Copy)]
struct MutationDispatch {
    prior_started: bool,
    current_mutates: bool,
}

impl MutationDispatch {
    const fn read_only(prior_started: bool) -> Self {
        Self {
            prior_started,
            current_mutates: false,
        }
    }

    const fn mutating(prior_started: bool) -> Self {
        Self {
            prior_started,
            current_mutates: true,
        }
    }

    const fn before_dispatch(self) -> bool {
        self.prior_started
    }

    const fn after_poll(self, polled: bool) -> bool {
        self.prior_started || (self.current_mutates && polled)
    }
}

struct EnsuredDiskImage {
    record: AcaDiskImageRecord,
    mutation_started: bool,
}

pub struct AzureContainerAppsRuntimeProvider {
    descriptor: ProviderDescriptor,
    credential_descriptor: ProviderDescriptor,
    configuration: AcaRuntimeConfig,
    credential_client: Arc<dyn AcaCredentialLeaseClient>,
    control: Arc<dyn AcaControl>,
    clock: Arc<dyn ProviderClock>,
    lease_cleanup_observer: Arc<dyn LeaseCleanupObserver>,
    lease_cleanup_executor: Arc<dyn LeaseCleanupExecutor>,
    operation_gate: Mutex<()>,
    ledger: Mutex<OperationLedger>,
}

impl fmt::Debug for AzureContainerAppsRuntimeProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureContainerAppsRuntimeProvider")
            .field("provider_generation", &self.descriptor.registry_generation)
            .field("configuration", &self.configuration)
            .finish_non_exhaustive()
    }
}

impl AzureContainerAppsRuntimeProvider {
    pub fn new(
        descriptor: ProviderDescriptor,
        configuration: AcaRuntimeConfig,
        credential_client: Arc<dyn AcaCredentialLeaseClient>,
        control: Arc<dyn AcaControl>,
    ) -> Result<Self, AcaProviderBuildError> {
        Self::with_clock(
            descriptor,
            configuration,
            credential_client,
            control,
            Arc::new(SystemProviderClock),
        )
    }

    pub fn with_clock(
        descriptor: ProviderDescriptor,
        configuration: AcaRuntimeConfig,
        credential_client: Arc<dyn AcaCredentialLeaseClient>,
        control: Arc<dyn AcaControl>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, AcaProviderBuildError> {
        Self::with_clock_and_cleanup_services(
            descriptor,
            configuration,
            credential_client,
            control,
            clock,
            Arc::new(TracingLeaseCleanupObserver),
            lease_cleanup_executor(),
        )
    }

    fn with_clock_and_cleanup_services(
        descriptor: ProviderDescriptor,
        configuration: AcaRuntimeConfig,
        credential_client: Arc<dyn AcaCredentialLeaseClient>,
        control: Arc<dyn AcaControl>,
        clock: Arc<dyn ProviderClock>,
        lease_cleanup_observer: Arc<dyn LeaseCleanupObserver>,
        lease_cleanup_executor: Arc<dyn LeaseCleanupExecutor>,
    ) -> Result<Self, AcaProviderBuildError> {
        Self::validate_descriptor(&descriptor)?;
        let credential_descriptor = credential_client.descriptor();
        Self::validate_credential_descriptor(&descriptor, &credential_descriptor)?;

        Ok(Self {
            descriptor,
            credential_descriptor,
            configuration,
            credential_client,
            control,
            clock,
            lease_cleanup_observer,
            lease_cleanup_executor,
            operation_gate: Mutex::new(()),
            ledger: Mutex::new(OperationLedger::default()),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_cleanup_services_for_test(
        descriptor: ProviderDescriptor,
        configuration: AcaRuntimeConfig,
        credential_client: Arc<dyn AcaCredentialLeaseClient>,
        control: Arc<dyn AcaControl>,
        clock: Arc<dyn ProviderClock>,
        lease_cleanup_observer: Arc<dyn LeaseCleanupObserver>,
        lease_cleanup_executor: Arc<dyn LeaseCleanupExecutor>,
    ) -> Result<Self, AcaProviderBuildError> {
        Self::with_clock_and_cleanup_services(
            descriptor,
            configuration,
            credential_client,
            control,
            clock,
            lease_cleanup_observer,
            lease_cleanup_executor,
        )
    }

    pub(crate) fn validate_descriptor(
        descriptor: &ProviderDescriptor,
    ) -> Result<(), AcaProviderBuildError> {
        descriptor.validate()?;
        if descriptor.implementation_id.as_str() != ACA_IMPLEMENTATION_ID {
            return Err(AcaProviderBuildError::WrongImplementation);
        }
        let d2b_contracts::v2_provider::ProviderAuthority::Runtime { posture } =
            &descriptor.authority
        else {
            return Err(AcaProviderBuildError::WrongAuthority);
        };
        if posture.process != ProcessAuthority::ProviderManagedRemote
            || posture.cgroup != CgroupAuthority::ProviderManagedRemote
            || posture.network != NetworkPosture::IsolatedNamespace
            || posture.user_namespace != UserNamespacePosture::None
            || posture.persistent_identity != PersistentIdentityPosture::NonCopyableAttested
            || posture.device_mediation != DeviceMediationPosture::ProviderManagedTyped
        {
            return Err(AcaProviderBuildError::WrongAuthority);
        }
        if !matches!(
            descriptor.placement,
            ProviderPlacement::ProviderAgent {
                endpoint_role: EndpointRole::ProviderAgent,
                service: ServicePackage::ProviderV2,
                ..
            }
        ) {
            return Err(AcaProviderBuildError::WrongPlacement);
        }
        if descriptor.capabilities != Self::advertised_capabilities()? {
            return Err(AcaProviderBuildError::CapabilityMismatch);
        }
        Ok(())
    }

    pub(crate) fn validate_credential_descriptor(
        descriptor: &ProviderDescriptor,
        credential_descriptor: &ProviderDescriptor,
    ) -> Result<(), AcaProviderBuildError> {
        credential_descriptor
            .validate()
            .map_err(|_| AcaProviderBuildError::CredentialDescriptorInvalid)?;
        if credential_descriptor.provider_type() != ProviderType::Credential {
            return Err(AcaProviderBuildError::CredentialDescriptorInvalid);
        }
        let consumer_binding = descriptor
            .placement
            .credential_binding()
            .ok_or(AcaProviderBuildError::WrongPlacement)?;
        let credential_binding = credential_descriptor
            .placement
            .credential_binding()
            .ok_or(AcaProviderBuildError::CredentialNotColocated)?;
        if consumer_binding != credential_binding
            || descriptor.provider_id == credential_descriptor.provider_id
        {
            return Err(AcaProviderBuildError::CredentialNotColocated);
        }

        Ok(())
    }

    pub fn advertised_capabilities() -> Result<ProviderCapabilitySet, ProviderContractError> {
        ProviderCapabilitySet::new(
            ACA_RUNTIME_METHODS
                .into_iter()
                .map(ProviderCapability)
                .collect(),
        )
    }

    fn now(&self) -> u64 {
        self.clock.now_unix_ms().min(MAX_SAFE_JSON_INTEGER)
    }

    fn values(&self, now: u64) -> Result<ProviderValues, ProviderContractError> {
        ProviderValues::new(&self.descriptor, now)
    }

    fn failure(
        &self,
        operation: &ProviderOperationContext,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        let now = self.now();
        self.values(now)
            .and_then(|values| values.failure(operation, kind, retry, reason, remediation))
            .unwrap_or_else(|_| ProviderFailure {
                kind,
                retry,
                provider_type: ProviderType::Runtime,
                binding: operation.binding(),
                correlation_id: operation.correlation_id.clone(),
                occurred_at_unix_ms: now,
                reason,
                remediation,
            })
    }

    fn contract_failure(
        &self,
        operation: &ProviderOperationContext,
        error: ProviderContractError,
    ) -> ProviderFailure {
        match error {
            ProviderContractError::ScopeMismatch => self.failure(
                operation,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::OperatorInteraction,
            ),
            ProviderContractError::CapabilityMismatch
            | ProviderContractError::MissingRequiredCapability => self.failure(
                operation,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            ProviderContractError::RequestExpired
            | ProviderContractError::RequestLifetimeExceeded
            | ProviderContractError::InvalidTimeRange => self.failure(
                operation,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::Never,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ),
            ProviderContractError::AdoptionEvidenceMismatch
            | ProviderContractError::AdoptionAmbiguous => self.failure(
                operation,
                ProviderFailureKind::AdoptionRejected,
                RetryClass::Never,
                ProviderHealthReason::GenerationMismatch,
                ProviderRemediation::ReplaceGeneration,
            ),
            _ => self.failure(
                operation,
                ProviderFailureKind::InvalidRequest,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
        }
    }

    fn internal_failure(&self, operation: &ProviderOperationContext) -> ProviderFailure {
        self.failure(
            operation,
            ProviderFailureKind::InvariantViolation,
            RetryClass::Never,
            ProviderHealthReason::ConfigurationMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn deadline_failure(
        &self,
        operation: &ProviderOperationContext,
        mutation_started: bool,
    ) -> ProviderFailure {
        if mutation_started {
            self.failure(
                operation,
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::InspectProvider,
            )
        } else {
            self.failure(
                operation,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            )
        }
    }

    fn effective_remaining(
        &self,
        deadline: &CallDeadline,
        operation: &ProviderOperationContext,
        mutation_started: bool,
    ) -> ProviderResult<Duration> {
        let monotonic_remaining = deadline
            .remaining()
            .ok_or_else(|| self.deadline_failure(operation, mutation_started))?;
        let wall_remaining_ms = operation.expires_at_unix_ms.saturating_sub(self.now());
        if wall_remaining_ms == 0 {
            return Err(self.deadline_failure(operation, mutation_started));
        }
        let effective = monotonic_remaining.min(Duration::from_millis(wall_remaining_ms));
        if effective.as_millis() == 0 {
            Err(self.deadline_failure(operation, mutation_started))
        } else {
            Ok(effective)
        }
    }

    fn effective_remaining_ms(
        &self,
        deadline: &CallDeadline,
        operation: &ProviderOperationContext,
        mutation_started: bool,
    ) -> ProviderResult<u32> {
        let remaining_ms = self
            .effective_remaining(deadline, operation, mutation_started)?
            .as_millis();
        Ok(remaining_ms.min(u128::from(u32::MAX)) as u32)
    }

    fn finish_lease<T>(
        &self,
        lease: ActiveCredentialLease,
        result: ProviderResult<T>,
    ) -> ProviderResult<T> {
        drop(lease);
        result
    }

    fn validate_call(
        &self,
        context: &ProviderCallContext<'_>,
        expected_method: ProviderMethod,
        now: u64,
    ) -> ProviderResult<()> {
        if context.cancelled {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.deadline_failure(context.operation, false));
        }
        context
            .validate()
            .map_err(|error| self.contract_failure(context.operation, error))?;
        context
            .operation
            .validate(&self.descriptor, now)
            .map_err(|error| self.contract_failure(context.operation, error))?;
        if context.peer_role != EndpointRole::RealmController
            || context.service != ServicePackage::ProviderV2
        {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::OperatorInteraction,
            ));
        }
        if context.operation.method != expected_method {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        Ok(())
    }

    fn validate_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        expected_method: ProviderMethod,
        now: u64,
    ) -> ProviderResult<WorkloadId> {
        self.validate_call(context, expected_method, now)?;
        if context.operation != &request.context {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::InvalidRequest,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        request
            .validate_method(&self.descriptor, now, expected_method)
            .map_err(|error| self.contract_failure(context.operation, error))?;
        request.target.workload_id().cloned().ok_or_else(|| {
            self.failure(
                context.operation,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::OperatorInteraction,
            )
        })
    }

    fn validate_ensure_plan(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
        now: u64,
    ) -> ProviderResult<WorkloadId> {
        self.validate_call(context, ProviderMethod::RuntimeEnsure, now)?;
        let scope = &context.operation.scope;
        if plan.schema_version != PROVIDER_SCHEMA_VERSION
            || plan.method != ProviderMethod::RuntimePlan
            || plan.binding.provider_id != self.descriptor.provider_id
            || plan.binding.provider_generation != self.descriptor.registry_generation
            || plan.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
            || plan.realm_id != *scope.realm_id()
            || plan.workload_id.as_ref() != scope.workload_id()
            || plan.created_at_unix_ms > now
            || plan.expires_at_unix_ms <= now
            || plan.resources.as_slice() != [PlannedResourceClass::WorkloadExecution]
        {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::InvalidRequest,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        plan.workload_id.clone().ok_or_else(|| {
            self.failure(
                context.operation,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::OperatorInteraction,
            )
        })
    }

    fn validate_adoption(
        &self,
        context: &ProviderCallContext<'_>,
        request: &AdoptionRequest,
        now: u64,
    ) -> ProviderResult<WorkloadId> {
        self.validate_call(context, ProviderMethod::RuntimeAdopt, now)?;
        if context.operation != &request.context {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::InvalidRequest,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ));
        }
        request
            .validate(&self.descriptor, now)
            .map_err(|error| self.contract_failure(context.operation, error))?;
        let expected_owner = self
            .values(now)
            .map_err(|_| self.internal_failure(context.operation))?
            .provider_owner(&request.handle.realm_id);
        if request.expected_owner != expected_owner
            || request.handle.realm_id != *context.operation.scope.realm_id()
            || request.handle.workload_id.as_ref() != context.operation.scope.workload_id()
        {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::OperatorInteraction,
            ));
        }
        request.handle.workload_id.clone().ok_or_else(|| {
            self.failure(
                context.operation,
                ProviderFailureKind::InvalidRequest,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::RepairConfiguration,
            )
        })
    }

    async fn acquire_gate<'a>(
        &'a self,
        deadline: &CallDeadline,
        operation: &ProviderOperationContext,
    ) -> ProviderResult<MutexGuard<'a, ()>> {
        timeout_at(deadline.at, self.operation_gate.lock())
            .await
            .map_err(|_| self.deadline_failure(operation, false))
    }

    async fn cached_response(
        &self,
        operation: &ProviderOperationContext,
        kind: ResponseKind,
    ) -> ProviderResult<Option<CachedResponse>> {
        let mut ledger = self.ledger.lock().await;
        let now = self.now();
        ledger
            .completed
            .retain(|_, completed| completed.expires_at_unix_ms > now);
        if let Some(completed) = ledger.completed.get(&operation.operation_id) {
            if completed.operation != *operation || completed.response.kind() != kind {
                return Err(self.failure(
                    operation,
                    ProviderFailureKind::InvalidRequest,
                    RetryClass::Never,
                    ProviderHealthReason::ConfigurationMismatch,
                    ProviderRemediation::RepairConfiguration,
                ));
            }
            if completed.response.retry_class() == Some(RetryClass::AfterObservation)
                && completed.observation_satisfied
            {
                ledger.completed.remove(&operation.operation_id);
                return Ok(None);
            }
            return Ok(Some(completed.response.clone()));
        }
        if ledger.completed.len() >= self.configuration.completed_operation_capacity() {
            return Err(self.failure(
                operation,
                ProviderFailureKind::Unavailable,
                RetryClass::AfterInteraction,
                ProviderHealthReason::QueuePressure,
                ProviderRemediation::OperatorInteraction,
            ));
        }
        Ok(None)
    }

    async fn record_response(
        &self,
        operation: &ProviderOperationContext,
        response: CachedResponse,
    ) {
        if !response.should_record() {
            return;
        }
        let mut ledger = self.ledger.lock().await;
        if operation.method == ProviderMethod::RuntimeInspect && response.is_success() {
            for completed in ledger.completed.values_mut() {
                if completed.operation.scope == operation.scope
                    && completed.response.retry_class() == Some(RetryClass::AfterObservation)
                {
                    completed.observation_satisfied = true;
                }
            }
        }
        ledger.completed.insert(
            operation.operation_id.clone(),
            CompletedOperation {
                operation: operation.clone(),
                expires_at_unix_ms: operation.expires_at_unix_ms,
                observation_satisfied: false,
                response,
            },
        );
    }

    async fn await_external<T>(
        &self,
        deadline: &CallDeadline,
        operation: &ProviderOperationContext,
        dispatch: MutationDispatch,
        future: impl Future<Output = Result<T, AcaControlError>>,
    ) -> ProviderResult<T> {
        let remaining =
            self.effective_remaining(deadline, operation, dispatch.before_dispatch())?;
        let polled = AtomicBool::new(false);
        tokio::pin!(future);
        let result = timeout(
            remaining,
            poll_fn(|context| {
                polled.store(true, Ordering::Release);
                future.as_mut().poll(context)
            }),
        )
        .await;
        let mutation_started = dispatch.after_poll(polled.load(Ordering::Acquire));
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(self.control_failure(operation, error, mutation_started)),
            Err(_) => Err(self.deadline_failure(operation, mutation_started)),
        }
    }

    fn control_failure(
        &self,
        operation: &ProviderOperationContext,
        error: AcaControlError,
        mutation_started: bool,
    ) -> ProviderFailure {
        if mutation_started
            && matches!(
                error.kind(),
                AcaControlErrorKind::Unavailable
                    | AcaControlErrorKind::Conflict
                    | AcaControlErrorKind::InvalidResponse
                    | AcaControlErrorKind::Cancelled
                    | AcaControlErrorKind::DeadlineExpired
                    | AcaControlErrorKind::Ambiguous
            )
        {
            return self.failure(
                operation,
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::InspectProvider,
            );
        }
        match error.kind() {
            AcaControlErrorKind::Authentication => self.failure(
                operation,
                ProviderFailureKind::CredentialLeaseInvalid,
                RetryClass::AfterInteraction,
                ProviderHealthReason::AuthenticationFailed,
                ProviderRemediation::ReEnrollPeer,
            ),
            AcaControlErrorKind::Authorization => self.failure(
                operation,
                ProviderFailureKind::Unavailable,
                RetryClass::AfterInteraction,
                ProviderHealthReason::AuthenticationFailed,
                ProviderRemediation::OperatorInteraction,
            ),
            AcaControlErrorKind::RateLimited => self.failure(
                operation,
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::QueuePressure,
                ProviderRemediation::RetryBounded,
            ),
            AcaControlErrorKind::Cancelled => self.failure(
                operation,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
            AcaControlErrorKind::DeadlineExpired => self.deadline_failure(operation, false),
            AcaControlErrorKind::InvalidResponse => self.internal_failure(operation),
            AcaControlErrorKind::Unavailable
            | AcaControlErrorKind::Conflict
            | AcaControlErrorKind::NotFound
            | AcaControlErrorKind::Ambiguous => self.failure(
                operation,
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
        }
    }

    async fn acquire_lease(
        &self,
        deadline: &CallDeadline,
        operation: &ProviderOperationContext,
        purpose: AcaCredentialPurpose,
    ) -> ProviderResult<ActiveCredentialLease> {
        let now = self.now();
        let effective_remaining_ms = self.effective_remaining_ms(deadline, operation, false)?;
        let requested_expiry_unix_ms = operation
            .expires_at_unix_ms
            .min(
                now.saturating_add(u64::from(effective_remaining_ms))
                    .min(MAX_SAFE_JSON_INTEGER),
            )
            .min(
                now.saturating_add(MAX_PROVIDER_LEASE_LIFETIME_MS)
                    .min(MAX_SAFE_JSON_INTEGER),
            );
        if requested_expiry_unix_ms <= now {
            return Err(self.deadline_failure(operation, false));
        }
        let request =
            AcaCredentialLeaseRequest::new(operation.clone(), purpose, requested_expiry_unix_ms);
        let lease = self
            .await_external(
                deadline,
                operation,
                MutationDispatch::read_only(false),
                self.credential_client.acquire(&request),
            )
            .await?;
        let lease = ActiveCredentialLease::new(
            lease,
            Arc::clone(&self.credential_client),
            Arc::clone(&self.lease_cleanup_observer),
            Arc::clone(&self.lease_cleanup_executor),
        );
        let validation_failed = lease
            .metadata()
            .validate(&self.credential_descriptor, &self.descriptor, self.now())
            .is_err()
            || lease.metadata().state != CredentialLeaseState::Active
            || lease.metadata().transfer_policy != CredentialLeaseTransferPolicy::Forbidden
            || purpose.required_operations().iter().any(|operation_class| {
                !lease
                    .metadata()
                    .allowed_operations
                    .contains(operation_class)
            });
        if validation_failed {
            let failure = self.failure(
                operation,
                ProviderFailureKind::CredentialLeaseInvalid,
                RetryClass::AfterInteraction,
                ProviderHealthReason::AuthenticationFailed,
                ProviderRemediation::ReEnrollPeer,
            );
            drop(lease);
            return Err(failure);
        }
        Ok(lease)
    }

    fn control_context(
        &self,
        deadline: &CallDeadline,
        operation: &ProviderOperationContext,
        lease: &AcaCredentialLease,
        operation_class: SdkOperationClass,
        dispatch: MutationDispatch,
    ) -> ProviderResult<AcaControlContext> {
        if !lease
            .metadata()
            .allowed_operations
            .contains(&operation_class)
        {
            return Err(self.failure(
                operation,
                ProviderFailureKind::CredentialLeaseInvalid,
                RetryClass::AfterInteraction,
                ProviderHealthReason::AuthenticationFailed,
                ProviderRemediation::ReEnrollPeer,
            ));
        }
        let remaining_ms =
            self.effective_remaining_ms(deadline, operation, dispatch.before_dispatch())?;
        Ok(AcaControlContext::new(
            operation.binding(),
            operation.method,
            operation_class,
            remaining_ms,
        ))
    }

    fn query(
        &self,
        operation: &ProviderOperationContext,
        workload_id: &WorkloadId,
    ) -> AcaWorkloadQuery {
        AcaWorkloadQuery::new(operation.scope.realm_id().clone(), workload_id.clone())
    }

    fn expected_binding(
        &self,
        operation: &ProviderOperationContext,
        workload_id: &WorkloadId,
        resource_generation: Generation,
        created_by: OperationBinding,
    ) -> AcaResourceBinding {
        AcaResourceBinding::new(
            operation.scope.realm_id().clone(),
            workload_id.clone(),
            self.descriptor.provider_id.clone(),
            self.descriptor.registry_generation,
            self.descriptor.configuration_schema_fingerprint.clone(),
            resource_generation,
            created_by,
        )
    }

    fn request_resource_generation(&self, request: &ProviderOperationRequest) -> Generation {
        match &request.target {
            d2b_contracts::v2_provider::ProviderTarget::Handle {
                handle_generation, ..
            } => *handle_generation,
            _ => self.configuration.initial_resource_generation(),
        }
    }

    fn verify_binding(
        actual: &AcaResourceBinding,
        expected: &AcaResourceBinding,
    ) -> Result<(), BindingMismatch> {
        if actual.realm_id() != expected.realm_id()
            || actual.workload_id() != expected.workload_id()
            || actual.provider_id() != expected.provider_id()
        {
            Err(BindingMismatch::Identity)
        } else if actual.configuration_fingerprint() != expected.configuration_fingerprint() {
            Err(BindingMismatch::Configuration)
        } else if actual.provider_generation() != expected.provider_generation()
            || actual.resource_generation() != expected.resource_generation()
        {
            Err(BindingMismatch::Generation)
        } else {
            Ok(())
        }
    }

    fn verify_created_by(
        actual: &AcaResourceBinding,
        expected: &OperationBinding,
    ) -> Result<(), BindingMismatch> {
        if actual.created_by() == expected {
            Ok(())
        } else {
            Err(BindingMismatch::Identity)
        }
    }

    fn handle_id(
        &self,
        operation: &ProviderOperationContext,
        record: &AcaSandboxRecord,
    ) -> ProviderResult<d2b_contracts::v2_provider::HandleId> {
        d2b_contracts::v2_provider::HandleId::parse(format!("aca-{}", record.id().as_str()))
            .map_err(|_| self.internal_failure(operation))
    }

    fn verify_target_handle(
        &self,
        request: &ProviderOperationRequest,
        record: &AcaSandboxRecord,
    ) -> Result<(), BindingMismatch> {
        if let d2b_contracts::v2_provider::ProviderTarget::Handle { handle_id, .. } =
            &request.target
        {
            let expected = format!("aca-{}", record.id().as_str());
            if handle_id.as_str() != expected {
                return Err(BindingMismatch::Identity);
            }
        }
        Ok(())
    }

    fn mismatch_reason(mismatch: BindingMismatch) -> ObservationReason {
        match mismatch {
            BindingMismatch::Identity => ObservationReason::IdentityMismatch,
            BindingMismatch::Configuration => ObservationReason::ConfigurationMismatch,
            BindingMismatch::Generation => ObservationReason::GenerationMismatch,
        }
    }

    fn mismatch_health(mismatch: BindingMismatch) -> (ProviderHealthReason, ProviderRemediation) {
        match mismatch {
            BindingMismatch::Identity => (
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ),
            BindingMismatch::Configuration => (
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            BindingMismatch::Generation => (
                ProviderHealthReason::GenerationMismatch,
                ProviderRemediation::ReplaceGeneration,
            ),
        }
    }

    fn mismatch_failure(
        &self,
        operation: &ProviderOperationContext,
        mismatch: BindingMismatch,
    ) -> ProviderFailure {
        let (reason, remediation) = Self::mismatch_health(mismatch);
        self.failure(
            operation,
            ProviderFailureKind::AdoptionRejected,
            RetryClass::AfterInteraction,
            reason,
            remediation,
        )
    }

    async fn find_sandboxes(
        &self,
        deadline: &CallDeadline,
        operation: &ProviderOperationContext,
        lease: &AcaCredentialLease,
        query: &AcaWorkloadQuery,
        mutation_started: bool,
    ) -> ProviderResult<crate::types::AcaSandboxCandidates> {
        let dispatch = MutationDispatch::read_only(mutation_started);
        let control_context = self.control_context(
            deadline,
            operation,
            lease,
            SdkOperationClass::Discover,
            dispatch,
        )?;
        self.await_external(
            deadline,
            operation,
            dispatch,
            self.control.find_sandboxes(lease, &control_context, query),
        )
        .await
    }

    fn multiple_candidates_failure(&self, operation: &ProviderOperationContext) -> ProviderFailure {
        self.failure(
            operation,
            ProviderFailureKind::AdoptionRejected,
            RetryClass::AfterInteraction,
            ProviderHealthReason::AdoptionAmbiguous,
            ProviderRemediation::OperatorInteraction,
        )
    }

    fn handle_from_plan(
        &self,
        operation: &ProviderOperationContext,
        plan: &ProviderPlan,
        record: &AcaSandboxRecord,
        now: u64,
    ) -> ProviderResult<ProviderHandle> {
        let values = self
            .values(now)
            .map_err(|_| self.internal_failure(operation))?;
        values
            .handle_from_plan(
                plan,
                self.handle_id(operation, record)?,
                values.provider_owner(&plan.realm_id),
                record.binding().resource_generation(),
                None,
            )
            .map_err(|error| self.contract_failure(operation, error))
    }

    fn handle_from_request(
        &self,
        operation: &ProviderOperationContext,
        request: &ProviderOperationRequest,
        record: &AcaSandboxRecord,
        now: u64,
    ) -> ProviderResult<ProviderHandle> {
        let values = self
            .values(now)
            .map_err(|_| self.internal_failure(operation))?;
        values
            .handle_from_request(
                request,
                self.handle_id(operation, record)?,
                values.provider_owner(request.target.realm_id()),
                record.binding().resource_generation(),
                None,
            )
            .map_err(|error| self.contract_failure(operation, error))
    }

    fn observation(
        &self,
        operation: &ProviderOperationContext,
        handle: Option<&ProviderHandle>,
        status: ObservationStatus,
        now: u64,
    ) -> ProviderResult<ProviderObservation> {
        self.values(now)
            .map_err(|_| self.internal_failure(operation))?
            .observation(
                operation,
                handle,
                status.lifecycle,
                status.adoption,
                status.reason,
                status.health_state,
                status.health_reason,
                status.remediation,
            )
            .map_err(|error| self.contract_failure(operation, error))
    }

    fn observation_for_record(
        &self,
        operation: &ProviderOperationContext,
        handle: &ProviderHandle,
        record: &AcaSandboxRecord,
        adoption: AdoptionState,
        now: u64,
    ) -> ProviderResult<ProviderObservation> {
        let (lifecycle, health_state, health_reason, remediation) = match record.lifecycle() {
            AcaSandboxLifecycle::Provisioning => (
                ObservedLifecycleState::Planned,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            AcaSandboxLifecycle::Ready => (
                ObservedLifecycleState::Ready,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            AcaSandboxLifecycle::Running => (
                ObservedLifecycleState::Running,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            AcaSandboxLifecycle::Idle
            | AcaSandboxLifecycle::Stopping
            | AcaSandboxLifecycle::Stopped => (
                ObservedLifecycleState::Stopped,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            AcaSandboxLifecycle::Deleted => (
                ObservedLifecycleState::Destroyed,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            AcaSandboxLifecycle::Failed => (
                ObservedLifecycleState::Unknown,
                ProviderHealthState::Degraded,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::InspectProvider,
            ),
            AcaSandboxLifecycle::Unknown => (
                ObservedLifecycleState::Unknown,
                ProviderHealthState::Degraded,
                ProviderHealthReason::HealthStale,
                ProviderRemediation::InspectProvider,
            ),
        };
        self.observation(
            operation,
            Some(handle),
            ObservationStatus {
                lifecycle,
                adoption,
                reason: ObservationReason::None,
                health_state,
                health_reason,
                remediation,
            },
            now,
        )
    }

    fn rejected_observation(
        &self,
        operation: &ProviderOperationContext,
        handle: Option<&ProviderHandle>,
        mismatch: BindingMismatch,
        now: u64,
    ) -> ProviderResult<ProviderObservation> {
        let (health_reason, remediation) = Self::mismatch_health(mismatch);
        self.observation(
            operation,
            handle,
            ObservationStatus {
                lifecycle: ObservedLifecycleState::Quarantined,
                adoption: AdoptionState::Rejected,
                reason: Self::mismatch_reason(mismatch),
                health_state: ProviderHealthState::Failed,
                health_reason,
                remediation,
            },
            now,
        )
    }

    fn ambiguous_observation(
        &self,
        operation: &ProviderOperationContext,
        now: u64,
    ) -> ProviderResult<ProviderObservation> {
        self.observation(
            operation,
            None,
            ObservationStatus {
                lifecycle: ObservedLifecycleState::Quarantined,
                adoption: AdoptionState::Ambiguous,
                reason: ObservationReason::MultipleCandidates,
                health_state: ProviderHealthState::Failed,
                health_reason: ProviderHealthReason::AdoptionAmbiguous,
                remediation: ProviderRemediation::OperatorInteraction,
            },
            now,
        )
    }

    fn missing_observation(
        &self,
        operation: &ProviderOperationContext,
        adoption: AdoptionState,
        now: u64,
    ) -> ProviderResult<ProviderObservation> {
        if adoption == AdoptionState::Rejected {
            self.observation(
                operation,
                None,
                ObservationStatus {
                    lifecycle: ObservedLifecycleState::Quarantined,
                    adoption,
                    reason: ObservationReason::MissingEvidence,
                    health_state: ProviderHealthState::Failed,
                    health_reason: ProviderHealthReason::IdentityMismatch,
                    remediation: ProviderRemediation::ReEnrollPeer,
                },
                now,
            )
        } else {
            self.observation(
                operation,
                None,
                ObservationStatus {
                    lifecycle: ObservedLifecycleState::Destroyed,
                    adoption,
                    reason: ObservationReason::None,
                    health_state: ProviderHealthState::Healthy,
                    health_reason: ProviderHealthReason::None,
                    remediation: ProviderRemediation::None,
                },
                now,
            )
        }
    }

    fn verify_disk(
        &self,
        actual: &AcaDiskImageRecord,
        desired: &AcaDesiredDiskImage,
    ) -> Result<(), BindingMismatch> {
        Self::verify_binding(actual.binding(), desired.binding())?;
        Self::verify_created_by(actual.binding(), desired.binding().created_by())?;
        if actual.profile_id() != desired.profile_id() || actual.source() != desired.source() {
            return Err(BindingMismatch::Configuration);
        }
        Ok(())
    }

    async fn ensure_disk_image(
        &self,
        deadline: &CallDeadline,
        operation: &ProviderOperationContext,
        lease: &AcaCredentialLease,
        desired: &AcaDesiredDiskImage,
    ) -> ProviderResult<EnsuredDiskImage> {
        let (record, mutation_started) = match desired.source() {
            AcaDiskImageSource::ConfiguredDisk { .. } => {
                let dispatch = MutationDispatch::read_only(false);
                let control_context = self.control_context(
                    deadline,
                    operation,
                    lease,
                    SdkOperationClass::Read,
                    dispatch,
                )?;
                (
                    self.await_external(
                        deadline,
                        operation,
                        dispatch,
                        self.control
                            .resolve_configured_disk(lease, &control_context, desired),
                    )
                    .await?,
                    false,
                )
            }
            AcaDiskImageSource::ConfiguredContainerImage { .. } => {
                let discover_dispatch = MutationDispatch::read_only(false);
                let discover_context = self.control_context(
                    deadline,
                    operation,
                    lease,
                    SdkOperationClass::Discover,
                    discover_dispatch,
                )?;
                let candidates = self
                    .await_external(
                        deadline,
                        operation,
                        discover_dispatch,
                        self.control
                            .find_disk_images(lease, &discover_context, desired),
                    )
                    .await?;
                match candidates.as_slice() {
                    [] => {
                        let create_dispatch = MutationDispatch::mutating(false);
                        let create_context = self.control_context(
                            deadline,
                            operation,
                            lease,
                            SdkOperationClass::Create,
                            create_dispatch,
                        )?;
                        (
                            self.await_external(
                                deadline,
                                operation,
                                create_dispatch,
                                self.control
                                    .create_disk_image(lease, &create_context, desired),
                            )
                            .await?,
                            true,
                        )
                    }
                    [record] => (record.clone(), false),
                    _ => return Err(self.multiple_candidates_failure(operation)),
                }
            }
        };
        self.verify_disk(&record, desired)
            .map_err(|mismatch| self.mismatch_failure(operation, mismatch))?;
        Ok(EnsuredDiskImage {
            record,
            mutation_started,
        })
    }

    async fn plan_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderPlan> {
        let now = self.now();
        self.validate_request(context, request, ProviderMethod::RuntimePlan, now)?;
        let deadline = CallDeadline::new(context, now);
        let _gate = self.acquire_gate(&deadline, context.operation).await?;
        if let Some(cached) = self
            .cached_response(context.operation, ResponseKind::Plan)
            .await?
        {
            return match cached {
                CachedResponse::Plan(result) => *result,
                _ => Err(self.internal_failure(context.operation)),
            };
        }
        let result = (|| {
            let expires_at_unix_ms = request.context.expires_at_unix_ms.min(
                now.saturating_add(u64::from(self.configuration.plan_ttl_ms()))
                    .min(MAX_SAFE_JSON_INTEGER),
            );
            if expires_at_unix_ms <= now {
                return Err(self.deadline_failure(context.operation, false));
            }
            let plan_id = PlanId::parse(context.operation.operation_id.as_str().to_owned())
                .map_err(|_| self.internal_failure(context.operation))?;
            let resources = BoundedVec::new(vec![PlannedResourceClass::WorkloadExecution])
                .map_err(|_| self.internal_failure(context.operation))?;
            self.values(now)
                .map_err(|_| self.internal_failure(context.operation))?
                .plan(request, plan_id, expires_at_unix_ms, resources)
                .map_err(|error| self.contract_failure(context.operation, error))
        })();
        self.record_response(
            context.operation,
            CachedResponse::Plan(Box::new(result.clone())),
        )
        .await;
        result
    }

    async fn ensure_inner(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
    ) -> ProviderResult<ProviderHandle> {
        let now = self.now();
        let workload_id = self.validate_ensure_plan(context, plan, now)?;
        let deadline = CallDeadline::new(context, now);
        let _gate = self.acquire_gate(&deadline, context.operation).await?;
        if let Some(cached) = self
            .cached_response(context.operation, ResponseKind::Handle)
            .await?
        {
            return match cached {
                CachedResponse::Handle(result) => *result,
                _ => Err(self.internal_failure(context.operation)),
            };
        }

        let lease = self
            .acquire_lease(&deadline, context.operation, AcaCredentialPurpose::Ensure)
            .await?;
        let result = async {
            let query = self.query(context.operation, &workload_id);
            let candidates = self
                .find_sandboxes(&deadline, context.operation, &lease, &query, false)
                .await?;
            let expected = self.expected_binding(
                context.operation,
                &workload_id,
                self.configuration.initial_resource_generation(),
                plan.binding.clone(),
            );
            let record = match candidates.as_slice() {
                [record] => {
                    Self::verify_binding(record.binding(), &expected)
                        .and_then(|()| Self::verify_created_by(record.binding(), &plan.binding))
                        .map_err(|mismatch| self.mismatch_failure(context.operation, mismatch))?;
                    record.clone()
                }
                [] => {
                    let desired_disk = AcaDesiredDiskImage::new(
                        expected.clone(),
                        self.configuration.profile().profile_id().clone(),
                        self.configuration.profile().disk_image().clone(),
                    );
                    let disk = self
                        .ensure_disk_image(&deadline, context.operation, &lease, &desired_disk)
                        .await?;
                    let desired = AcaDesiredSandbox::new(
                        expected.clone(),
                        self.configuration.profile().clone(),
                        disk.record.id().clone(),
                    );
                    let create_dispatch = MutationDispatch::mutating(disk.mutation_started);
                    let create_context = self.control_context(
                        &deadline,
                        context.operation,
                        &lease,
                        SdkOperationClass::Create,
                        create_dispatch,
                    )?;
                    let created = self
                        .await_external(
                            &deadline,
                            context.operation,
                            create_dispatch,
                            self.control
                                .create_sandbox(&lease, &create_context, &desired),
                        )
                        .await?;
                    Self::verify_binding(created.binding(), &expected)
                        .and_then(|()| Self::verify_created_by(created.binding(), &plan.binding))
                        .map_err(|mismatch| self.mismatch_failure(context.operation, mismatch))?;
                    created
                }
                _ => return Err(self.multiple_candidates_failure(context.operation)),
            };
            if matches!(
                record.lifecycle(),
                AcaSandboxLifecycle::Deleted
                    | AcaSandboxLifecycle::Failed
                    | AcaSandboxLifecycle::Unknown
            ) {
                return Err(self.failure(
                    context.operation,
                    ProviderFailureKind::Unavailable,
                    RetryClass::AfterObservation,
                    ProviderHealthReason::ProviderDegraded,
                    ProviderRemediation::InspectProvider,
                ));
            }
            self.handle_from_plan(context.operation, plan, &record, now)
        }
        .await;
        let result = self.finish_lease(lease, result);
        self.record_response(
            context.operation,
            CachedResponse::Handle(Box::new(result.clone())),
        )
        .await;
        result
    }

    async fn poll_running(
        &self,
        deadline: &CallDeadline,
        operation: &ProviderOperationContext,
        lease: &AcaCredentialLease,
        initial: AcaSandboxRecord,
        poll: RunningPoll<'_>,
    ) -> ProviderResult<AcaSandboxRecord> {
        if matches!(
            initial.lifecycle(),
            AcaSandboxLifecycle::Running | AcaSandboxLifecycle::Ready
        ) {
            return Ok(initial);
        }
        let readiness = self.configuration.readiness();
        for attempt in 0..readiness.attempts() {
            if attempt > 0 {
                let sleep =
                    tokio::time::sleep(Duration::from_millis(u64::from(readiness.interval_ms())));
                timeout_at(deadline.at, sleep)
                    .await
                    .map_err(|_| self.deadline_failure(operation, poll.mutation_started))?;
            }
            let candidates = self
                .find_sandboxes(
                    deadline,
                    operation,
                    lease,
                    poll.query,
                    poll.mutation_started,
                )
                .await?;
            let record = match candidates.as_slice() {
                [record] => record.clone(),
                [] => {
                    return Err(self.failure(
                        operation,
                        ProviderFailureKind::Unavailable,
                        RetryClass::AfterObservation,
                        ProviderHealthReason::HealthStale,
                        ProviderRemediation::InspectProvider,
                    ));
                }
                _ => return Err(self.multiple_candidates_failure(operation)),
            };
            Self::verify_binding(record.binding(), poll.expected)
                .and_then(|()| Self::verify_created_by(record.binding(), poll.created_by))
                .map_err(|mismatch| self.mismatch_failure(operation, mismatch))?;
            if matches!(
                record.lifecycle(),
                AcaSandboxLifecycle::Running | AcaSandboxLifecycle::Ready
            ) {
                return Ok(record);
            }
            if matches!(
                record.lifecycle(),
                AcaSandboxLifecycle::Failed
                    | AcaSandboxLifecycle::Deleted
                    | AcaSandboxLifecycle::Unknown
            ) {
                return Err(self.failure(
                    operation,
                    ProviderFailureKind::Unavailable,
                    RetryClass::AfterObservation,
                    ProviderHealthReason::ProviderDegraded,
                    ProviderRemediation::InspectProvider,
                ));
            }
        }
        Err(self.deadline_failure(operation, poll.mutation_started))
    }

    async fn start_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderObservation> {
        let now = self.now();
        let workload_id =
            self.validate_request(context, request, ProviderMethod::RuntimeStart, now)?;
        let deadline = CallDeadline::new(context, now);
        let _gate = self.acquire_gate(&deadline, context.operation).await?;
        if let Some(cached) = self
            .cached_response(context.operation, ResponseKind::Observation)
            .await?
        {
            return match cached {
                CachedResponse::Observation(result) => *result,
                _ => Err(self.internal_failure(context.operation)),
            };
        }

        let lease = self
            .acquire_lease(&deadline, context.operation, AcaCredentialPurpose::Start)
            .await?;
        let result = async {
            let query = self.query(context.operation, &workload_id);
            let candidates = self
                .find_sandboxes(&deadline, context.operation, &lease, &query, false)
                .await?;
            let record = match candidates.as_slice() {
                [record] => record.clone(),
                [] => {
                    return Err(self.failure(
                        context.operation,
                        ProviderFailureKind::Unavailable,
                        RetryClass::AfterInteraction,
                        ProviderHealthReason::HealthStale,
                        ProviderRemediation::InspectProvider,
                    ));
                }
                _ => return Err(self.multiple_candidates_failure(context.operation)),
            };
            let expected = self.expected_binding(
                context.operation,
                &workload_id,
                self.request_resource_generation(request),
                context.operation.binding(),
            );
            Self::verify_binding(record.binding(), &expected)
                .and_then(|()| self.verify_target_handle(request, &record))
                .map_err(|mismatch| self.mismatch_failure(context.operation, mismatch))?;
            let created_by = record.binding().created_by().clone();

            let (record, mutation_started) = if matches!(
                record.lifecycle(),
                AcaSandboxLifecycle::Idle | AcaSandboxLifecycle::Stopped
            ) {
                let power_dispatch = MutationDispatch::mutating(false);
                let power_context = self.control_context(
                    &deadline,
                    context.operation,
                    &lease,
                    SdkOperationClass::Power,
                    power_dispatch,
                )?;
                let resumed = self
                    .await_external(
                        &deadline,
                        context.operation,
                        power_dispatch,
                        self.control
                            .resume_sandbox(&lease, &power_context, record.id()),
                    )
                    .await?;
                Self::verify_binding(resumed.binding(), &expected)
                    .and_then(|()| Self::verify_created_by(resumed.binding(), &created_by))
                    .map_err(|mismatch| self.mismatch_failure(context.operation, mismatch))?;
                (resumed, true)
            } else {
                (record, false)
            };
            let running = self
                .poll_running(
                    &deadline,
                    context.operation,
                    &lease,
                    record,
                    RunningPoll {
                        query: &query,
                        expected: &expected,
                        created_by: &created_by,
                        mutation_started,
                    },
                )
                .await?;
            let handle = self.handle_from_request(context.operation, request, &running, now)?;
            self.observation(
                context.operation,
                Some(&handle),
                ObservationStatus {
                    lifecycle: ObservedLifecycleState::Running,
                    adoption: AdoptionState::NotAttempted,
                    reason: ObservationReason::None,
                    health_state: ProviderHealthState::Healthy,
                    health_reason: ProviderHealthReason::None,
                    remediation: ProviderRemediation::None,
                },
                now,
            )
        }
        .await;
        let result = self.finish_lease(lease, result);
        self.record_response(
            context.operation,
            CachedResponse::Observation(Box::new(result.clone())),
        )
        .await;
        result
    }

    async fn stop_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderObservation> {
        let now = self.now();
        let workload_id =
            self.validate_request(context, request, ProviderMethod::RuntimeStop, now)?;
        let deadline = CallDeadline::new(context, now);
        let _gate = self.acquire_gate(&deadline, context.operation).await?;
        if let Some(cached) = self
            .cached_response(context.operation, ResponseKind::Observation)
            .await?
        {
            return match cached {
                CachedResponse::Observation(result) => *result,
                _ => Err(self.internal_failure(context.operation)),
            };
        }

        let lease = self
            .acquire_lease(&deadline, context.operation, AcaCredentialPurpose::Stop)
            .await?;
        let result = async {
            let query = self.query(context.operation, &workload_id);
            let candidates = self
                .find_sandboxes(&deadline, context.operation, &lease, &query, false)
                .await?;
            let Some(mut record) = (match candidates.as_slice() {
                [record] => Some(record.clone()),
                [] => None,
                _ => return Err(self.multiple_candidates_failure(context.operation)),
            }) else {
                return self.missing_observation(
                    context.operation,
                    AdoptionState::NotAttempted,
                    now,
                );
            };
            let expected = self.expected_binding(
                context.operation,
                &workload_id,
                self.request_resource_generation(request),
                context.operation.binding(),
            );
            Self::verify_binding(record.binding(), &expected)
                .and_then(|()| self.verify_target_handle(request, &record))
                .map_err(|mismatch| self.mismatch_failure(context.operation, mismatch))?;
            let created_by = record.binding().created_by().clone();
            if matches!(
                record.lifecycle(),
                AcaSandboxLifecycle::Running
                    | AcaSandboxLifecycle::Ready
                    | AcaSandboxLifecycle::Provisioning
            ) {
                let power_dispatch = MutationDispatch::mutating(false);
                let power_context = self.control_context(
                    &deadline,
                    context.operation,
                    &lease,
                    SdkOperationClass::Power,
                    power_dispatch,
                )?;
                record = self
                    .await_external(
                        &deadline,
                        context.operation,
                        power_dispatch,
                        self.control
                            .stop_sandbox(&lease, &power_context, record.id()),
                    )
                    .await?;
                Self::verify_binding(record.binding(), &expected)
                    .and_then(|()| Self::verify_created_by(record.binding(), &created_by))
                    .map_err(|mismatch| self.mismatch_failure(context.operation, mismatch))?;
            }
            if matches!(
                record.lifecycle(),
                AcaSandboxLifecycle::Running
                    | AcaSandboxLifecycle::Ready
                    | AcaSandboxLifecycle::Failed
                    | AcaSandboxLifecycle::Unknown
                    | AcaSandboxLifecycle::Deleted
            ) {
                return Err(self.failure(
                    context.operation,
                    ProviderFailureKind::Unavailable,
                    RetryClass::AfterObservation,
                    ProviderHealthReason::ProviderDegraded,
                    ProviderRemediation::InspectProvider,
                ));
            }
            let handle = self.handle_from_request(context.operation, request, &record, now)?;
            self.observation(
                context.operation,
                Some(&handle),
                ObservationStatus {
                    lifecycle: ObservedLifecycleState::Stopped,
                    adoption: AdoptionState::NotAttempted,
                    reason: ObservationReason::None,
                    health_state: ProviderHealthState::Healthy,
                    health_reason: ProviderHealthReason::None,
                    remediation: ProviderRemediation::None,
                },
                now,
            )
        }
        .await;
        let result = self.finish_lease(lease, result);
        self.record_response(
            context.operation,
            CachedResponse::Observation(Box::new(result.clone())),
        )
        .await;
        result
    }

    async fn inspect_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderObservation> {
        let now = self.now();
        let workload_id =
            self.validate_request(context, request, ProviderMethod::RuntimeInspect, now)?;
        let deadline = CallDeadline::new(context, now);
        let _gate = self.acquire_gate(&deadline, context.operation).await?;
        if let Some(cached) = self
            .cached_response(context.operation, ResponseKind::Observation)
            .await?
        {
            return match cached {
                CachedResponse::Observation(result) => *result,
                _ => Err(self.internal_failure(context.operation)),
            };
        }

        let lease = self
            .acquire_lease(&deadline, context.operation, AcaCredentialPurpose::Inspect)
            .await?;
        let result = async {
            let query = self.query(context.operation, &workload_id);
            let candidates = self
                .find_sandboxes(&deadline, context.operation, &lease, &query, false)
                .await?;
            let record = match candidates.as_slice() {
                [record] => record,
                [] => {
                    return self.missing_observation(
                        context.operation,
                        AdoptionState::NotAttempted,
                        now,
                    );
                }
                _ => return self.ambiguous_observation(context.operation, now),
            };
            let expected = self.expected_binding(
                context.operation,
                &workload_id,
                self.request_resource_generation(request),
                context.operation.binding(),
            );
            if let Err(mismatch) = Self::verify_binding(record.binding(), &expected)
                .and_then(|()| self.verify_target_handle(request, record))
            {
                return self.rejected_observation(context.operation, None, mismatch, now);
            }
            let handle = self.handle_from_request(context.operation, request, record, now)?;
            self.observation_for_record(
                context.operation,
                &handle,
                record,
                AdoptionState::NotAttempted,
                now,
            )
        }
        .await;
        let result = self.finish_lease(lease, result);
        self.record_response(
            context.operation,
            CachedResponse::Observation(Box::new(result.clone())),
        )
        .await;
        result
    }

    async fn adopt_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &AdoptionRequest,
    ) -> ProviderResult<ProviderObservation> {
        let now = self.now();
        let workload_id = self.validate_adoption(context, request, now)?;
        let deadline = CallDeadline::new(context, now);
        let _gate = self.acquire_gate(&deadline, context.operation).await?;
        if let Some(cached) = self
            .cached_response(context.operation, ResponseKind::Observation)
            .await?
        {
            return match cached {
                CachedResponse::Observation(result) => *result,
                _ => Err(self.internal_failure(context.operation)),
            };
        }

        let lease = self
            .acquire_lease(&deadline, context.operation, AcaCredentialPurpose::Adopt)
            .await?;
        let result = async {
            let query = self.query(context.operation, &workload_id);
            let candidates = self
                .find_sandboxes(&deadline, context.operation, &lease, &query, false)
                .await?;
            let record = match candidates.as_slice() {
                [record] => record,
                [] => {
                    return self.missing_observation(
                        context.operation,
                        AdoptionState::Rejected,
                        now,
                    );
                }
                _ => return self.ambiguous_observation(context.operation, now),
            };
            let expected = self.expected_binding(
                context.operation,
                &workload_id,
                request.expected_resource_generation,
                request.handle.created_by.clone(),
            );
            let remote_handle_id = self.handle_id(context.operation, record)?;
            let mismatch = Self::verify_binding(record.binding(), &expected)
                .and_then(|()| {
                    Self::verify_created_by(record.binding(), &request.handle.created_by)
                })
                .err()
                .or_else(|| {
                    (request.handle.handle_id != remote_handle_id)
                        .then_some(BindingMismatch::Identity)
                });
            if let Some(mismatch) = mismatch {
                return self.rejected_observation(
                    context.operation,
                    Some(&request.handle),
                    mismatch,
                    now,
                );
            }
            self.observation_for_record(
                context.operation,
                &request.handle,
                record,
                AdoptionState::Adopted,
                now,
            )
        }
        .await;
        let result = self.finish_lease(lease, result);
        self.record_response(
            context.operation,
            CachedResponse::Observation(Box::new(result.clone())),
        )
        .await;
        result
    }

    async fn destroy_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<MutationReceipt> {
        let now = self.now();
        let workload_id =
            self.validate_request(context, request, ProviderMethod::RuntimeDestroy, now)?;
        let deadline = CallDeadline::new(context, now);
        let _gate = self.acquire_gate(&deadline, context.operation).await?;
        if let Some(cached) = self
            .cached_response(context.operation, ResponseKind::Receipt)
            .await?
        {
            return match cached {
                CachedResponse::Receipt(result) => *result,
                _ => Err(self.internal_failure(context.operation)),
            };
        }

        let lease = self
            .acquire_lease(&deadline, context.operation, AcaCredentialPurpose::Destroy)
            .await?;
        let result = async {
            let query = self.query(context.operation, &workload_id);
            let candidates = self
                .find_sandboxes(&deadline, context.operation, &lease, &query, false)
                .await?;
            let state = match candidates.as_slice() {
                [] => MutationState::AlreadyApplied,
                [record] => {
                    let expected = self.expected_binding(
                        context.operation,
                        &workload_id,
                        self.request_resource_generation(request),
                        context.operation.binding(),
                    );
                    Self::verify_binding(record.binding(), &expected)
                        .and_then(|()| self.verify_target_handle(request, record))
                        .map_err(|mismatch| self.mismatch_failure(context.operation, mismatch))?;
                    let delete_dispatch = MutationDispatch::mutating(false);
                    let delete_context = self.control_context(
                        &deadline,
                        context.operation,
                        &lease,
                        SdkOperationClass::Delete,
                        delete_dispatch,
                    )?;
                    match self
                        .await_external(
                            &deadline,
                            context.operation,
                            delete_dispatch,
                            self.control
                                .delete_sandbox(&lease, &delete_context, record.id()),
                        )
                        .await?
                    {
                        AcaDeleteOutcome::Deleted => MutationState::Applied,
                        AcaDeleteOutcome::AlreadyAbsent => MutationState::AlreadyApplied,
                    }
                }
                _ => return Err(self.multiple_candidates_failure(context.operation)),
            };
            self.values(now)
                .map_err(|_| self.internal_failure(context.operation))?
                .receipt(context.operation, state)
                .map_err(|error| self.contract_failure(context.operation, error))
        }
        .await;
        let result = self.finish_lease(lease, result);
        self.record_response(
            context.operation,
            CachedResponse::Receipt(Box::new(result.clone())),
        )
        .await;
        result
    }
}

impl Provider for AzureContainerAppsRuntimeProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move {
            let now = self.now();
            self.validate_call(context, ProviderMethod::RuntimeInspect, now)?;
            let deadline = CallDeadline::new(context, now);
            let lease = self
                .acquire_lease(&deadline, context.operation, AcaCredentialPurpose::Health)
                .await?;
            let result = async {
                let dispatch = MutationDispatch::read_only(false);
                let control_context = self.control_context(
                    &deadline,
                    context.operation,
                    &lease,
                    SdkOperationClass::Read,
                    dispatch,
                )?;
                let health = self
                    .await_external(
                        &deadline,
                        context.operation,
                        dispatch,
                        self.control.health(&lease, &control_context),
                    )
                    .await?;
                let (state, reason, remediation) = match health {
                    AcaControlHealth::Ready => (
                        ProviderHealthState::Healthy,
                        ProviderHealthReason::None,
                        ProviderRemediation::None,
                    ),
                    AcaControlHealth::Degraded => (
                        ProviderHealthState::Degraded,
                        ProviderHealthReason::ProviderDegraded,
                        ProviderRemediation::InspectProvider,
                    ),
                    AcaControlHealth::Unavailable => (
                        ProviderHealthState::Unavailable,
                        ProviderHealthReason::HealthStale,
                        ProviderRemediation::RetryBounded,
                    ),
                };
                self.values(now)
                    .map_err(|_| self.internal_failure(context.operation))?
                    .health(state, reason, remediation)
                    .map_err(|error| self.contract_failure(context.operation, error))
            }
            .await;
            self.finish_lease(lease, result)
        })
    }
}

impl RuntimeProvider for AzureContainerAppsRuntimeProvider {
    fn capabilities(&self) -> ProviderCapabilitySet {
        self.descriptor.capabilities.clone()
    }

    fn plan<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderPlan> {
        Box::pin(self.plan_inner(context, request))
    }

    fn ensure<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        plan: &'a ProviderPlan,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(self.ensure_inner(context, plan))
    }

    fn start<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(self.start_inner(context, request))
    }

    fn stop<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(self.stop_inner(context, request))
    }

    fn inspect<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(self.inspect_inner(context, request))
    }

    fn adopt<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a AdoptionRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(self.adopt_inner(context, request))
    }

    fn destroy<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(self.destroy_inner(context, request))
    }
}
