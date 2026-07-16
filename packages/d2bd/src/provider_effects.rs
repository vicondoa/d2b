//! Descriptor-bound daemon effect ports for first-party providers.
//!
//! Provider crates receive only their semantic port traits. This module is the
//! trusted composition seam that binds those ports to one exact descriptor
//! before registry construction. The port implementations remain responsible
//! for resolving generated opaque IDs into current daemon, host, and broker
//! behavior; a missing or stale binding is an error, never a no-op.

use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{Arc, Mutex, Weak},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    broker_wire::BrokerCallerRole,
    provider_registry_v2::{
        LocalRuntimeProviderBindingV2, ProviderBindingV2, ProviderRegistryEntryV2,
    },
    public_wire::{MutationFlags, VmLifecycleRequest},
    v2_identity::{ProviderId, ProviderType, RealmId},
    v2_provider::{
        AuthorizedProviderScope, HandleId, HandleOwner, IdempotencyKey, MutationState,
        ObservabilityCursor, ObservabilityView, ObservationReason, ObservedLifecycleState,
        OperationId, PlanId, ProviderDescriptor, ProviderHealthReason, ProviderHealthState,
        ProviderMethod, ProviderRemediation,
    },
};
use d2b_provider_audio_pipewire_vhost_user::{AudioEffectPort, AudioQueryPort};
use d2b_provider_device_host_mediated::{DeviceEffectPort, DeviceQueryPort};
use d2b_provider_display_wayland::DisplayEffectPort;
use d2b_provider_network_local_realm::NetworkEffectPort;
use d2b_provider_observability_local::{
    BoundedExportSink, ClosedMetricLabels, ExportPortOutcome, ExportSinkStatus,
    LocalObservabilityStatus, LocalObservationRecord, MetricLabel,
    OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES, ObservabilityCall, ObservabilityExportIntent,
    ObservabilityExportPort, ObservabilityPortError, ObservabilityQueryIntent,
    ObservabilityQueryPort, OperationLabel, OutcomeLabel, ProjectionKind, ProjectionPage,
};
use d2b_provider_runtime_local::{
    LocalRuntimeKind, RuntimeAdoptionControl, RuntimeAdoptionMismatch, RuntimeAdoptionOutcome,
    RuntimeConfiguredItemControl, RuntimeControlContext, RuntimeControlError, RuntimeControlPort,
    RuntimeEnsureControl, RuntimeHealth, RuntimeMutationOutcome, RuntimeObservedState,
    RuntimeOperationControl, RuntimePlanDecision, RuntimeResourceIdentity,
};
use d2b_provider_storage_local::StorageEffectPort;
use d2b_provider_substrate_host::HostSubstratePort;
use d2b_provider_transport_local::LocalEndpointPort;

use crate::{
    ServerState, TypedError, block_on_future, daemon_audit::DaemonAuditSinkStatus,
    dispatch_broker_vm_start_async, dispatch_broker_vm_stop_as_async,
    provider_registry::resolve_current_runtime_route,
};

#[cfg(test)]
thread_local! {
    static TEST_RUNTIME_START_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static TEST_RUNTIME_STOP_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_test_runtime_lifecycle_calls() {
    TEST_RUNTIME_START_CALLS.set(0);
    TEST_RUNTIME_STOP_CALLS.set(0);
}

#[cfg(test)]
pub(crate) fn test_runtime_lifecycle_calls() -> (usize, usize) {
    (
        TEST_RUNTIME_START_CALLS.get(),
        TEST_RUNTIME_STOP_CALLS.get(),
    )
}

type LifecycleDispatchResult = Result<serde_json::Value, TypedError>;
type LifecycleInvocationParts = (
    VmLifecycleRequest,
    BrokerCallerRole,
    Arc<Mutex<Option<LifecycleDispatchResult>>>,
);
const MAX_TRACKED_LIFECYCLE_MUTATIONS: usize = 256;

#[derive(Debug)]
struct ProviderLifecycleInvocation {
    request: VmLifecycleRequest,
    caller_role: BrokerCallerRole,
    result: Arc<Mutex<Option<LifecycleDispatchResult>>>,
}

#[derive(Debug, Default)]
pub struct ProviderLifecycleDispatch {
    invocations: Mutex<BTreeMap<String, ProviderLifecycleInvocation>>,
}

pub(crate) struct ProviderLifecycleInvocationHandle {
    dispatch: Arc<ProviderLifecycleDispatch>,
    operation_id: String,
    result: Arc<Mutex<Option<LifecycleDispatchResult>>>,
    finished: bool,
}

impl ProviderLifecycleDispatch {
    pub(crate) fn begin(
        self: &Arc<Self>,
        operation_id: &OperationId,
        request: VmLifecycleRequest,
        caller_role: BrokerCallerRole,
    ) -> Result<ProviderLifecycleInvocationHandle, TypedError> {
        let operation_id = operation_id.as_str().to_owned();
        let result = Arc::new(Mutex::new(None));
        let mut invocations = self
            .invocations
            .lock()
            .map_err(|_| TypedError::InternalConfig {
                detail: "provider lifecycle invocation table is poisoned".to_owned(),
            })?;
        if invocations.contains_key(&operation_id) {
            return Err(TypedError::InternalConfig {
                detail: "duplicate provider lifecycle operation id".to_owned(),
            });
        }
        invocations.insert(
            operation_id.clone(),
            ProviderLifecycleInvocation {
                request,
                caller_role,
                result: Arc::clone(&result),
            },
        );
        Ok(ProviderLifecycleInvocationHandle {
            dispatch: Arc::clone(self),
            operation_id,
            result,
            finished: false,
        })
    }

    fn invocation(
        &self,
        operation_id: &OperationId,
        vm: &str,
    ) -> Result<LifecycleInvocationParts, RuntimeControlError> {
        let invocations = self
            .invocations
            .lock()
            .map_err(|_| RuntimeControlError::Unavailable)?;
        let invocation = invocations
            .get(operation_id.as_str())
            .ok_or(RuntimeControlError::InvalidRequest)?;
        if invocation.request.vm != vm {
            return Err(RuntimeControlError::UnauthorizedScope);
        }
        Ok((
            invocation.request.clone(),
            invocation.caller_role.clone(),
            Arc::clone(&invocation.result),
        ))
    }

    fn remove(&self, operation_id: &str) {
        if let Ok(mut invocations) = self.invocations.lock() {
            invocations.remove(operation_id);
        }
    }
}

impl ProviderLifecycleInvocationHandle {
    pub(crate) fn finish(mut self) -> Option<LifecycleDispatchResult> {
        self.dispatch.remove(&self.operation_id);
        self.finished = true;
        self.result.lock().ok()?.take()
    }
}

impl Drop for ProviderLifecycleInvocationHandle {
    fn drop(&mut self) {
        if !self.finished {
            self.dispatch.remove(&self.operation_id);
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LifecycleMutationKey {
    operation_id: OperationId,
    idempotency_key: IdempotencyKey,
    method: ProviderMethod,
}

impl LifecycleMutationKey {
    fn from_request(request: &RuntimeOperationControl) -> Self {
        let operation = request.context().operation();
        Self {
            operation_id: operation.operation_id.clone(),
            idempotency_key: operation.idempotency_key.clone(),
            method: operation.method,
        }
    }
}

struct TrackedLifecycleMutation {
    result: Mutex<Option<LifecycleDispatchResult>>,
    completed: tokio::sync::Notify,
}

impl TrackedLifecycleMutation {
    fn pending() -> Self {
        Self {
            result: Mutex::new(None),
            completed: tokio::sync::Notify::new(),
        }
    }

    fn result(&self) -> Option<LifecycleDispatchResult> {
        self.result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn complete(&self, result: LifecycleDispatchResult) {
        let mut slot = self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(result);
        drop(slot);
        self.completed.notify_waiters();
    }

    async fn wait(&self) -> LifecycleDispatchResult {
        loop {
            let completed = self.completed.notified();
            if let Some(result) = self.result() {
                return result;
            }
            completed.await;
        }
    }
}

#[derive(Default)]
struct ProviderLifecycleTasks {
    tasks: Mutex<BTreeMap<LifecycleMutationKey, Arc<TrackedLifecycleMutation>>>,
}

impl ProviderLifecycleTasks {
    fn existing(
        &self,
        key: &LifecycleMutationKey,
    ) -> Result<Option<Arc<TrackedLifecycleMutation>>, RuntimeControlError> {
        self.tasks
            .lock()
            .map(|tasks| tasks.get(key).cloned())
            .map_err(|_| RuntimeControlError::Unavailable)
    }

    fn spawn<F>(
        &self,
        key: LifecycleMutationKey,
        work: F,
    ) -> Result<Arc<TrackedLifecycleMutation>, RuntimeControlError>
    where
        F: FnOnce() -> LifecycleDispatchResult + Send + 'static,
    {
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| RuntimeControlError::Unavailable)?;
        if let Some(task) = tasks.get(&key) {
            return Ok(Arc::clone(task));
        }
        if tasks.len() >= MAX_TRACKED_LIFECYCLE_MUTATIONS
            && let Some(completed) = tasks
                .iter()
                .find_map(|(key, task)| task.result().is_some().then(|| key.clone()))
        {
            tasks.remove(&completed);
        }
        if tasks.len() >= MAX_TRACKED_LIFECYCLE_MUTATIONS {
            return Err(RuntimeControlError::Unavailable);
        }

        let task = Arc::new(TrackedLifecycleMutation::pending());
        tasks.insert(key.clone(), Arc::clone(&task));
        let owned_task = Arc::clone(&task);
        if std::thread::Builder::new()
            .name("d2b-provider-lifecycle".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                    .unwrap_or_else(|_| {
                        Err(TypedError::InternalConfig {
                            detail: "provider lifecycle worker failed".to_owned(),
                        })
                    });
                owned_task.complete(result);
            })
            .is_err()
        {
            tasks.remove(&key);
            return Err(RuntimeControlError::Unavailable);
        }
        Ok(task)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonEffectAdapterError {
    DuplicateBinding,
    MappingUnavailable,
    ConfigurationMismatch,
    TransactionAborted,
}

impl fmt::Display for DaemonEffectAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DuplicateBinding => "duplicate daemon provider effect binding",
            Self::MappingUnavailable => "generated daemon provider mapping is unavailable",
            Self::ConfigurationMismatch => {
                "daemon provider effect binding does not match the accepted descriptor"
            }
            Self::TransactionAborted => "daemon provider effect binding transaction was aborted",
        })
    }
}

impl Error for DaemonEffectAdapterError {}

struct ExactEffect<T: ?Sized> {
    descriptor: ProviderDescriptor,
    effect: Arc<T>,
}

impl<T: ?Sized> Clone for ExactEffect<T> {
    fn clone(&self) -> Self {
        Self {
            descriptor: self.descriptor.clone(),
            effect: Arc::clone(&self.effect),
        }
    }
}

#[derive(Clone)]
pub struct DeviceEffectAdapter {
    effects: Arc<dyn DeviceEffectPort>,
    queries: Arc<dyn DeviceQueryPort>,
}

impl DeviceEffectAdapter {
    pub fn effects(&self) -> Arc<dyn DeviceEffectPort> {
        Arc::clone(&self.effects)
    }

    pub fn queries(&self) -> Arc<dyn DeviceQueryPort> {
        Arc::clone(&self.queries)
    }
}

#[derive(Clone)]
pub struct AudioEffectAdapter {
    effects: Arc<dyn AudioEffectPort>,
    queries: Arc<dyn AudioQueryPort>,
}

impl AudioEffectAdapter {
    pub fn effects(&self) -> Arc<dyn AudioEffectPort> {
        Arc::clone(&self.effects)
    }

    pub fn queries(&self) -> Arc<dyn AudioQueryPort> {
        Arc::clone(&self.queries)
    }
}

#[derive(Clone)]
pub struct ObservabilityEffectAdapter {
    queries: Arc<dyn ObservabilityQueryPort>,
    exports: Arc<dyn ObservabilityExportPort>,
}

impl ObservabilityEffectAdapter {
    pub fn queries(&self) -> Arc<dyn ObservabilityQueryPort> {
        Arc::clone(&self.queries)
    }

    pub fn exports(&self) -> Arc<dyn ObservabilityExportPort> {
        Arc::clone(&self.exports)
    }
}

#[derive(Clone, Default)]
pub struct DaemonEffectAdapters {
    runtime: BTreeMap<ProviderId, ExactEffect<dyn RuntimeControlPort>>,
    transport: BTreeMap<ProviderId, ExactEffect<dyn LocalEndpointPort>>,
    substrate: BTreeMap<ProviderId, ExactEffect<dyn HostSubstratePort>>,
    display: BTreeMap<ProviderId, ExactEffect<dyn DisplayEffectPort>>,
    network: BTreeMap<ProviderId, ExactEffect<dyn NetworkEffectPort>>,
    storage: BTreeMap<ProviderId, ExactEffect<dyn StorageEffectPort>>,
    device: BTreeMap<ProviderId, ExactEffect<DeviceEffectAdapter>>,
    audio: BTreeMap<ProviderId, ExactEffect<AudioEffectAdapter>>,
    observability: BTreeMap<ProviderId, ExactEffect<ObservabilityEffectAdapter>>,
}

impl fmt::Debug for DaemonEffectAdapters {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonEffectAdapters")
            .field("runtime", &self.runtime.len())
            .field("transport", &self.transport.len())
            .field("substrate", &self.substrate.len())
            .field("display", &self.display.len())
            .field("network", &self.network.len())
            .field("storage", &self.storage.len())
            .field("device", &self.device.len())
            .field("audio", &self.audio.len())
            .field("observability", &self.observability.len())
            .finish()
    }
}

impl DaemonEffectAdapters {
    pub fn builder() -> DaemonEffectAdaptersBuilder {
        DaemonEffectAdaptersBuilder::default()
    }

    pub fn runtime(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn RuntimeControlPort>, DaemonEffectAdapterError> {
        resolve(&self.runtime, descriptor)
    }

    pub fn transport(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn LocalEndpointPort>, DaemonEffectAdapterError> {
        resolve(&self.transport, descriptor)
    }

    pub fn substrate(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn HostSubstratePort>, DaemonEffectAdapterError> {
        resolve(&self.substrate, descriptor)
    }

    pub fn display(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn DisplayEffectPort>, DaemonEffectAdapterError> {
        resolve(&self.display, descriptor)
    }

    pub fn network(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn NetworkEffectPort>, DaemonEffectAdapterError> {
        resolve(&self.network, descriptor)
    }

    pub fn storage(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<Arc<dyn StorageEffectPort>, DaemonEffectAdapterError> {
        resolve(&self.storage, descriptor)
    }

    pub fn device(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<DeviceEffectAdapter, DaemonEffectAdapterError> {
        resolve(&self.device, descriptor).map(|adapter| adapter.as_ref().clone())
    }

    pub fn audio(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<AudioEffectAdapter, DaemonEffectAdapterError> {
        resolve(&self.audio, descriptor).map(|adapter| adapter.as_ref().clone())
    }

    pub fn observability(
        &self,
        descriptor: &ProviderDescriptor,
    ) -> Result<ObservabilityEffectAdapter, DaemonEffectAdapterError> {
        resolve(&self.observability, descriptor).map(|adapter| adapter.as_ref().clone())
    }

    pub(crate) fn for_server_state(
        state: Weak<ServerState>,
        entries: &[ProviderRegistryEntryV2],
    ) -> Result<Self, DaemonEffectAdapterError> {
        let mut builder = Self::builder();
        for entry in entries {
            match &entry.binding {
                ProviderBindingV2::LocalRuntime(binding) => {
                    let adapter: Arc<dyn RuntimeControlPort> =
                        Arc::new(DaemonLocalRuntimeControl {
                            state: state.clone(),
                            entry: entry.clone(),
                            realm_id: entry.descriptor.placement.realm_id().clone(),
                            binding: binding.clone(),
                            lifecycle_tasks: Arc::new(ProviderLifecycleTasks::default()),
                        });
                    builder.bind_runtime(entry.descriptor.clone(), adapter)?;
                }
                ProviderBindingV2::LocalObservability(_) => {
                    let state = state
                        .upgrade()
                        .ok_or(DaemonEffectAdapterError::MappingUnavailable)?;
                    let adapter = Arc::new(DaemonLocalObservability {
                        realm_id: entry.descriptor.placement.realm_id().clone(),
                        metrics: Arc::clone(&state.metrics_registry),
                        audit: Arc::clone(&state.daemon_audit),
                        connections: state.conn_semaphore.clone(),
                    });
                    builder.bind_observability(
                        entry.descriptor.clone(),
                        adapter.clone(),
                        adapter,
                    )?;
                }
            }
        }
        builder.finish()
    }
}

#[derive(Clone)]
struct DaemonLocalObservability {
    realm_id: RealmId,
    metrics: Arc<crate::metrics::Registry>,
    audit: Arc<crate::daemon_audit::DaemonAuditLog>,
    connections: crate::concurrency::ConnSemaphore,
}

impl fmt::Debug for DaemonLocalObservability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonLocalObservability")
            .finish_non_exhaustive()
    }
}

impl DaemonLocalObservability {
    fn validate_call(&self, context: &ObservabilityCall) -> Result<(), ObservabilityPortError> {
        if context.scope().realm_id() == &self.realm_id {
            Ok(())
        } else {
            Err(ObservabilityPortError::Denied)
        }
    }

    fn status(&self) -> LocalObservabilityStatus {
        match self.audit.sink_health_report().status {
            DaemonAuditSinkStatus::Ok => LocalObservabilityStatus::healthy(),
            DaemonAuditSinkStatus::Degraded | DaemonAuditSinkStatus::Unavailable => {
                LocalObservabilityStatus {
                    health_state: ProviderHealthState::Degraded,
                    health_reason: ProviderHealthReason::ProviderDegraded,
                    remediation: ProviderRemediation::InspectProvider,
                }
            }
        }
    }

    fn now_unix_ms() -> Result<u64, ObservabilityPortError> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ObservabilityPortError::Unavailable)?
            .as_millis()
            .try_into()
            .map_err(|_| ObservabilityPortError::Unavailable)
    }

    fn record(
        observed_at_unix_ms: u64,
        projection: ProjectionKind,
        labels: ClosedMetricLabels,
        value: u64,
    ) -> Result<LocalObservationRecord, ObservabilityPortError> {
        LocalObservationRecord::new(observed_at_unix_ms, projection, labels, value)
            .map_err(|_| ObservabilityPortError::InvalidProjection)
    }

    fn records_for(
        &self,
        view: Option<ObservabilityView>,
    ) -> Result<Vec<LocalObservationRecord>, ObservabilityPortError> {
        let now = Self::now_unix_ms()?;
        let status = self.status();
        let health_outcome = if status.health_state == ProviderHealthState::Healthy {
            OutcomeLabel::Success
        } else {
            OutcomeLabel::Unavailable
        };
        let metrics = self.metrics.local_observability_projection();
        let mut records = Vec::new();
        if view.is_none() || view == Some(ObservabilityView::Health) {
            records.push(Self::record(
                now,
                ProjectionKind::AuditSummary,
                ClosedMetricLabels::new(
                    ProviderType::Observability,
                    status.health_state,
                    MetricLabel::ProviderHealth,
                    OperationLabel::Health,
                    health_outcome,
                ),
                1,
            )?);
            records.push(Self::record(
                now,
                ProjectionKind::Metrics,
                ClosedMetricLabels::new(
                    ProviderType::Observability,
                    ProviderHealthState::Healthy,
                    MetricLabel::QueueDepth,
                    OperationLabel::Query,
                    OutcomeLabel::Success,
                ),
                u64::try_from(self.connections.in_flight()).unwrap_or(u64::MAX),
            )?);
        }
        if view.is_none() || view == Some(ObservabilityView::Lifecycle) {
            records.push(Self::record(
                now,
                ProjectionKind::Metrics,
                ClosedMetricLabels::new(
                    ProviderType::Runtime,
                    ProviderHealthState::Healthy,
                    MetricLabel::LifecycleTransition,
                    OperationLabel::Inspect,
                    OutcomeLabel::Success,
                ),
                metrics.lifecycle_transitions,
            )?);
        }
        if view.is_none() || view == Some(ObservabilityView::Operations) {
            records.push(Self::record(
                now,
                ProjectionKind::Metrics,
                ClosedMetricLabels::new(
                    ProviderType::Observability,
                    ProviderHealthState::Healthy,
                    MetricLabel::OperationTotal,
                    OperationLabel::Query,
                    OutcomeLabel::Success,
                ),
                metrics.operation_total,
            )?);
            records.push(Self::record(
                now,
                ProjectionKind::TraceSummary,
                ClosedMetricLabels::new(
                    ProviderType::Observability,
                    ProviderHealthState::Healthy,
                    MetricLabel::OperationDuration,
                    OperationLabel::Query,
                    OutcomeLabel::Success,
                ),
                metrics.operation_duration_ms,
            )?);
        }
        records.sort_unstable();
        Ok(records)
    }

    fn page(
        &self,
        intent: &ObservabilityQueryIntent,
    ) -> Result<ProjectionPage, ObservabilityPortError> {
        let records = self.records_for(Some(intent.view))?;
        let offset = match intent.cursor() {
            None => 0,
            Some(cursor) => cursor
                .as_str()
                .strip_prefix("offset-")
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|offset| *offset <= records.len())
                .ok_or(ObservabilityPortError::InvalidProjection)?,
        };
        let byte_capacity = intent.bounds.max_bytes / OBSERVATION_RECORD_ENCODED_UPPER_BOUND_BYTES;
        let capacity = usize::from(
            intent
                .bounds
                .max_records
                .min(u16::try_from(byte_capacity).unwrap_or(u16::MAX)),
        );
        let end = offset.saturating_add(capacity).min(records.len());
        let next_cursor = (end < records.len())
            .then(|| ObservabilityCursor::parse(format!("offset-{end}")))
            .transpose()
            .map_err(|_| ObservabilityPortError::InvalidProjection)?;
        ProjectionPage::new(
            records[offset..end].to_vec(),
            next_cursor,
            end < records.len(),
        )
        .map_err(|_| ObservabilityPortError::InvalidProjection)
    }
}

#[async_trait]
impl ObservabilityQueryPort for DaemonLocalObservability {
    async fn health(
        &self,
        context: ObservabilityCall,
    ) -> Result<LocalObservabilityStatus, ObservabilityPortError> {
        self.validate_call(&context)?;
        Ok(self.status())
    }

    async fn status(
        &self,
        context: ObservabilityCall,
    ) -> Result<LocalObservabilityStatus, ObservabilityPortError> {
        self.validate_call(&context)?;
        Ok(self.status())
    }

    async fn query(
        &self,
        context: ObservabilityCall,
        intent: ObservabilityQueryIntent,
    ) -> Result<ProjectionPage, ObservabilityPortError> {
        self.validate_call(&context)?;
        self.page(&intent)
    }
}

#[async_trait]
impl ObservabilityExportPort for DaemonLocalObservability {
    async fn export(
        &self,
        context: ObservabilityCall,
        intent: ObservabilityExportIntent,
        sink: BoundedExportSink,
    ) -> Result<ExportPortOutcome, ObservabilityPortError> {
        self.validate_call(&context)?;
        let mut emitted = false;
        for record in self.records_for(None)?.into_iter().filter(|record| {
            record.observed_at_unix_ms() >= intent.start_at_unix_ms
                && record.observed_at_unix_ms() <= intent.end_at_unix_ms
        }) {
            match sink
                .emit(record)
                .map_err(|_| ObservabilityPortError::InvalidProjection)?
            {
                ExportSinkStatus::Emitted => emitted = true,
                ExportSinkStatus::Truncated => {
                    sink.mark_source_truncated()
                        .map_err(|_| ObservabilityPortError::InvalidProjection)?;
                    break;
                }
            }
        }
        Ok(ExportPortOutcome::new(if emitted {
            MutationState::Applied
        } else {
            MutationState::NotApplicable
        }))
    }
}

#[derive(Clone)]
struct DaemonLocalRuntimeControl {
    state: Weak<ServerState>,
    entry: ProviderRegistryEntryV2,
    realm_id: RealmId,
    binding: LocalRuntimeProviderBindingV2,
    lifecycle_tasks: Arc<ProviderLifecycleTasks>,
}

impl fmt::Debug for DaemonLocalRuntimeControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonLocalRuntimeControl")
            .field("descriptor", &self.entry.descriptor)
            .finish_non_exhaustive()
    }
}

struct ResolvedDaemonRuntime {
    state: Arc<ServerState>,
    vm: String,
    role: String,
}

impl DaemonLocalRuntimeControl {
    fn kind(&self) -> Result<LocalRuntimeKind, RuntimeControlError> {
        match self.entry.descriptor.implementation_id.as_str() {
            d2b_provider_runtime_local::CLOUD_HYPERVISOR_IMPLEMENTATION_ID => {
                Ok(LocalRuntimeKind::CloudHypervisor)
            }
            d2b_provider_runtime_local::QEMU_MEDIA_IMPLEMENTATION_ID => {
                Ok(LocalRuntimeKind::QemuMedia)
            }
            _ => Err(RuntimeControlError::InvalidRequest),
        }
    }

    fn resolve(
        &self,
        context: &RuntimeControlContext,
    ) -> Result<ResolvedDaemonRuntime, RuntimeControlError> {
        if context.is_cancelled() {
            return Err(RuntimeControlError::CancelledBeforeMutation);
        }
        if context.effective_deadline_remaining_ms() == 0 {
            return Err(RuntimeControlError::DeadlineExpiredBeforeMutation);
        }
        if context.kind() != self.kind()?
            || context.operation().provider_id != self.entry.descriptor.provider_id
            || context.operation().provider_generation != self.entry.descriptor.registry_generation
            || context.operation().scope.realm_id() != &self.realm_id
            || context.operation().scope.workload_id() != Some(&self.binding.workload_id)
        {
            return Err(RuntimeControlError::UnauthorizedScope);
        }

        let state = self
            .state
            .upgrade()
            .ok_or(RuntimeControlError::Unavailable)?;
        let (vm, role) = resolve_current_runtime_route(&state, &self.entry)
            .map_err(|_| RuntimeControlError::InvalidRequest)?;
        Ok(ResolvedDaemonRuntime { state, vm, role })
    }

    fn validate_target(
        &self,
        request: &RuntimeOperationControl,
    ) -> Result<ResolvedDaemonRuntime, RuntimeControlError> {
        if request.target().realm_id() != &self.realm_id
            || request.target().workload_id() != Some(&self.binding.workload_id)
        {
            return Err(RuntimeControlError::UnauthorizedScope);
        }
        self.resolve(request.context())
    }

    fn resource_identity(
        &self,
        _context: &RuntimeControlContext,
    ) -> Result<RuntimeResourceIdentity, RuntimeControlError> {
        let handle_id = HandleId::parse(format!(
            "runtime-{}",
            self.entry.descriptor.provider_id.as_str()
        ))
        .map_err(|_| RuntimeControlError::InvariantViolation)?;
        Ok(RuntimeResourceIdentity::new(
            self.kind()?,
            self.entry.descriptor.provider_id.clone(),
            self.entry.descriptor.registry_generation,
            AuthorizedProviderScope::Workload {
                realm_id: self.realm_id.clone(),
                workload_id: self.binding.workload_id.clone(),
            },
            handle_id,
            HandleOwner::RealmController {
                realm_id: self.realm_id.clone(),
            },
            self.entry.descriptor.registry_generation,
            self.entry
                .descriptor
                .configuration_schema_fingerprint
                .clone(),
        ))
    }

    fn observed(
        &self,
        context: &RuntimeControlContext,
        resolved: &ResolvedDaemonRuntime,
    ) -> Result<RuntimeObservedState, RuntimeControlError> {
        let running = resolved
            .state
            .pidfd_table
            .still_alive_same_start_time(&resolved.vm, &resolved.role);
        RuntimeObservedState::new(
            running
                .then(|| self.resource_identity(context))
                .transpose()?,
            if running {
                ObservedLifecycleState::Running
            } else {
                ObservedLifecycleState::Stopped
            },
            ObservationReason::None,
            RuntimeHealth::healthy(),
        )
        .map_err(|_| RuntimeControlError::InvariantViolation)
    }

    fn lifecycle_request(vm: String) -> VmLifecycleRequest {
        VmLifecycleRequest {
            vm,
            flags: MutationFlags {
                dry_run: false,
                apply: true,
                json: true,
            },
            force: false,
            no_wait_api: true,
        }
    }

    fn response_applied(response: &serde_json::Value) -> bool {
        response.get("outcome").and_then(serde_json::Value::as_str) == Some("applied")
    }

    fn publish_lifecycle_result(
        result_slot: &Arc<Mutex<Option<LifecycleDispatchResult>>>,
        result: &LifecycleDispatchResult,
    ) -> Result<(), RuntimeControlError> {
        *result_slot
            .lock()
            .map_err(|_| RuntimeControlError::Unavailable)? = Some(result.clone());
        Ok(())
    }

    fn publish_retry_result(
        &self,
        request: &RuntimeOperationControl,
        vm: &str,
        result: &LifecycleDispatchResult,
    ) -> Result<(), RuntimeControlError> {
        let state = self
            .state
            .upgrade()
            .ok_or(RuntimeControlError::Unavailable)?;
        let dispatch = state
            .provider_registry()
            .map_err(|_| RuntimeControlError::Unavailable)?
            .lifecycle_dispatch();
        if let Ok((_, _, result_slot)) =
            dispatch.invocation(&request.context().operation().operation_id, vm)
        {
            Self::publish_lifecycle_result(&result_slot, result)?;
        }
        Ok(())
    }

    async fn invoke_direct_start(
        &self,
        request: &RuntimeOperationControl,
        resolved: &ResolvedDaemonRuntime,
        lifecycle_permit: Option<crate::concurrency::MappedLifecyclePermit>,
    ) -> Result<(), RuntimeControlError> {
        let key = LifecycleMutationKey::from_request(request);
        if let Some(task) = self.lifecycle_tasks.existing(&key)? {
            let result = task.wait().await;
            self.publish_retry_result(request, &resolved.vm, &result)?;
            return if result.as_ref().is_ok_and(Self::response_applied) {
                Ok(())
            } else {
                Err(RuntimeControlError::Unavailable)
            };
        }
        if request.context().is_cancelled() {
            return Err(RuntimeControlError::CancelledBeforeMutation);
        }
        let lifecycle_permit = lifecycle_permit.ok_or(RuntimeControlError::InvariantViolation)?;
        let (lifecycle, _, result_slot) = resolved
            .state
            .provider_registry()
            .map_err(|_| RuntimeControlError::Unavailable)?
            .lifecycle_dispatch()
            .invocation(&request.context().operation().operation_id, &resolved.vm)?;
        let state = Arc::clone(&resolved.state);
        let task = self.lifecycle_tasks.spawn(key, move || {
            let _lifecycle_permit = lifecycle_permit;
            let result = block_on_future(dispatch_broker_vm_start_async(&state, lifecycle));
            let _ = Self::publish_lifecycle_result(&result_slot, &result);
            result
        })?;
        let result = task.wait().await;
        let applied = result.as_ref().is_ok_and(Self::response_applied);
        if applied {
            Ok(())
        } else {
            Err(RuntimeControlError::Unavailable)
        }
    }

    async fn invoke_direct_stop(
        &self,
        request: &RuntimeOperationControl,
        resolved: &ResolvedDaemonRuntime,
        lifecycle_permit: Option<crate::concurrency::MappedLifecyclePermit>,
    ) -> Result<(), RuntimeControlError> {
        let key = LifecycleMutationKey::from_request(request);
        if let Some(task) = self.lifecycle_tasks.existing(&key)? {
            let result = task.wait().await;
            self.publish_retry_result(request, &resolved.vm, &result)?;
            return if result.as_ref().is_ok_and(Self::response_applied) {
                Ok(())
            } else {
                Err(RuntimeControlError::Unavailable)
            };
        }
        if request.context().is_cancelled() {
            return Err(RuntimeControlError::CancelledBeforeMutation);
        }
        let lifecycle_permit = lifecycle_permit.ok_or(RuntimeControlError::InvariantViolation)?;
        let (lifecycle, caller_role, result_slot) = resolved
            .state
            .provider_registry()
            .map_err(|_| RuntimeControlError::Unavailable)?
            .lifecycle_dispatch()
            .invocation(&request.context().operation().operation_id, &resolved.vm)?;
        let state = Arc::clone(&resolved.state);
        let task = self.lifecycle_tasks.spawn(key, move || {
            let _lifecycle_permit = lifecycle_permit;
            let result = block_on_future(dispatch_broker_vm_stop_as_async(
                &state,
                lifecycle,
                caller_role,
            ));
            let _ = Self::publish_lifecycle_result(&result_slot, &result);
            result
        })?;
        let result = task.wait().await;
        let applied = result.as_ref().is_ok_and(Self::response_applied);
        if applied {
            Ok(())
        } else {
            Err(RuntimeControlError::Unavailable)
        }
    }
}

#[async_trait]
impl RuntimeControlPort for DaemonLocalRuntimeControl {
    async fn health(
        &self,
        context: RuntimeControlContext,
    ) -> Result<RuntimeHealth, RuntimeControlError> {
        self.resolve(&context)?;
        Ok(RuntimeHealth::healthy())
    }

    async fn plan(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimePlanDecision, RuntimeControlError> {
        self.validate_target(&request)?;
        let plan_id = PlanId::parse(format!(
            "runtime-{}",
            self.entry.descriptor.provider_id.as_str()
        ))
        .map_err(|_| RuntimeControlError::InvariantViolation)?;
        Ok(RuntimePlanDecision::new(
            plan_id,
            request.context().operation().expires_at_unix_ms,
        ))
    }

    async fn ensure(
        &self,
        request: RuntimeEnsureControl,
    ) -> Result<RuntimeResourceIdentity, RuntimeControlError> {
        self.resolve(request.context())?;
        self.resource_identity(request.context())
    }

    async fn start(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError> {
        #[cfg(test)]
        TEST_RUNTIME_START_CALLS.set(TEST_RUNTIME_START_CALLS.get() + 1);
        let resolved = self.validate_target(&request)?;
        if self
            .lifecycle_tasks
            .existing(&LifecycleMutationKey::from_request(&request))?
            .is_some()
        {
            self.invoke_direct_start(&request, &resolved, None).await?;
            return self.observed(request.context(), &resolved);
        }
        let lifecycle_permit = resolved.state.op_locks.begin_mapped_lifecycle(&resolved.vm);
        if resolved
            .state
            .pidfd_table
            .still_alive_same_start_time(&resolved.vm, &resolved.role)
        {
            return self.observed(request.context(), &resolved);
        }
        self.invoke_direct_start(&request, &resolved, Some(lifecycle_permit))
            .await?;
        let observed = self.observed(request.context(), &resolved)?;
        if observed.lifecycle() == ObservedLifecycleState::Running {
            Ok(observed)
        } else {
            Err(RuntimeControlError::CompletionAmbiguous)
        }
    }

    async fn stop(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError> {
        #[cfg(test)]
        TEST_RUNTIME_STOP_CALLS.set(TEST_RUNTIME_STOP_CALLS.get() + 1);
        let resolved = self.validate_target(&request)?;
        if self
            .lifecycle_tasks
            .existing(&LifecycleMutationKey::from_request(&request))?
            .is_some()
        {
            self.invoke_direct_stop(&request, &resolved, None).await?;
            return self.observed(request.context(), &resolved);
        }
        let lifecycle_permit = resolved.state.op_locks.begin_mapped_lifecycle(&resolved.vm);
        if !resolved
            .state
            .pidfd_table
            .still_alive_same_start_time(&resolved.vm, &resolved.role)
        {
            return self.observed(request.context(), &resolved);
        }
        self.invoke_direct_stop(&request, &resolved, Some(lifecycle_permit))
            .await?;
        let observed = self.observed(request.context(), &resolved)?;
        if observed.lifecycle() == ObservedLifecycleState::Stopped {
            Ok(observed)
        } else {
            Err(RuntimeControlError::CompletionAmbiguous)
        }
    }

    async fn inspect(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError> {
        let resolved = self.validate_target(&request)?;
        self.observed(request.context(), &resolved)
    }

    async fn adopt(
        &self,
        request: RuntimeAdoptionControl,
    ) -> Result<RuntimeAdoptionOutcome, RuntimeControlError> {
        let resolved = self.resolve(request.context())?;
        let identity = self.resource_identity(request.context())?;
        if request.expected() != &identity {
            return Ok(RuntimeAdoptionOutcome::Rejected(
                RuntimeAdoptionMismatch::MissingEvidence,
            ));
        }
        let observed = self.observed(request.context(), &resolved)?;
        if observed.lifecycle() == ObservedLifecycleState::Running {
            Ok(RuntimeAdoptionOutcome::Adopted(Box::new(observed)))
        } else {
            Ok(RuntimeAdoptionOutcome::Rejected(
                RuntimeAdoptionMismatch::MissingEvidence,
            ))
        }
    }

    async fn destroy(
        &self,
        request: RuntimeOperationControl,
    ) -> Result<RuntimeMutationOutcome, RuntimeControlError> {
        let resolved = self.validate_target(&request)?;
        let key = LifecycleMutationKey::from_request(&request);
        let existing = self.lifecycle_tasks.existing(&key)?;
        let lifecycle_permit = existing
            .is_none()
            .then(|| resolved.state.op_locks.begin_mapped_lifecycle(&resolved.vm));
        if existing.is_none()
            && !resolved
                .state
                .pidfd_table
                .still_alive_same_start_time(&resolved.vm, &resolved.role)
        {
            return Ok(RuntimeMutationOutcome::new(MutationState::NotApplicable));
        }
        if existing.is_none() && request.context().is_cancelled() {
            return Err(RuntimeControlError::CancelledBeforeMutation);
        }
        let task = match existing {
            Some(task) => task,
            None => {
                let lifecycle_permit =
                    lifecycle_permit.ok_or(RuntimeControlError::InvariantViolation)?;
                let state = Arc::clone(&resolved.state);
                let lifecycle = Self::lifecycle_request(resolved.vm);
                let caller_role = BrokerCallerRole::AdminUid {
                    uid: state.daemon_uid,
                };
                self.lifecycle_tasks.spawn(key, move || {
                    let _lifecycle_permit = lifecycle_permit;
                    block_on_future(dispatch_broker_vm_stop_as_async(
                        &state,
                        lifecycle,
                        caller_role,
                    ))
                })?
            }
        };
        let response = task
            .wait()
            .await
            .map_err(|_| RuntimeControlError::Unavailable)?;
        if Self::response_applied(&response) {
            Ok(RuntimeMutationOutcome::new(MutationState::Applied))
        } else {
            Err(RuntimeControlError::CompletionAmbiguous)
        }
    }

    async fn execute_configured_item(
        &self,
        _request: RuntimeConfiguredItemControl,
    ) -> Result<RuntimeObservedState, RuntimeControlError> {
        Err(RuntimeControlError::InvalidRequest)
    }
}

#[derive(Default)]
pub struct DaemonEffectAdaptersBuilder {
    adapters: DaemonEffectAdapters,
    failed: bool,
}

impl fmt::Debug for DaemonEffectAdaptersBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonEffectAdaptersBuilder")
            .field("adapters", &self.adapters)
            .field("failed", &self.failed)
            .finish()
    }
}

macro_rules! bind_single_effect {
    ($method:ident, $field:ident, $trait:path) => {
        pub fn $method(
            &mut self,
            descriptor: ProviderDescriptor,
            effect: Arc<dyn $trait>,
        ) -> Result<&mut Self, DaemonEffectAdapterError> {
            let result = bind(&mut self.adapters.$field, descriptor, effect);
            self.finish_step(result)
        }
    };
}

impl DaemonEffectAdaptersBuilder {
    bind_single_effect!(bind_runtime, runtime, RuntimeControlPort);
    bind_single_effect!(bind_transport, transport, LocalEndpointPort);
    bind_single_effect!(bind_substrate, substrate, HostSubstratePort);
    bind_single_effect!(bind_display, display, DisplayEffectPort);
    bind_single_effect!(bind_network, network, NetworkEffectPort);
    bind_single_effect!(bind_storage, storage, StorageEffectPort);

    pub fn bind_device(
        &mut self,
        descriptor: ProviderDescriptor,
        effects: Arc<dyn DeviceEffectPort>,
        queries: Arc<dyn DeviceQueryPort>,
    ) -> Result<&mut Self, DaemonEffectAdapterError> {
        let adapter = Arc::new(DeviceEffectAdapter { effects, queries });
        let result = bind(&mut self.adapters.device, descriptor, adapter);
        self.finish_step(result)
    }

    pub fn bind_audio(
        &mut self,
        descriptor: ProviderDescriptor,
        effects: Arc<dyn AudioEffectPort>,
        queries: Arc<dyn AudioQueryPort>,
    ) -> Result<&mut Self, DaemonEffectAdapterError> {
        let adapter = Arc::new(AudioEffectAdapter { effects, queries });
        let result = bind(&mut self.adapters.audio, descriptor, adapter);
        self.finish_step(result)
    }

    pub fn bind_observability(
        &mut self,
        descriptor: ProviderDescriptor,
        queries: Arc<dyn ObservabilityQueryPort>,
        exports: Arc<dyn ObservabilityExportPort>,
    ) -> Result<&mut Self, DaemonEffectAdapterError> {
        let adapter = Arc::new(ObservabilityEffectAdapter { queries, exports });
        let result = bind(&mut self.adapters.observability, descriptor, adapter);
        self.finish_step(result)
    }

    fn finish_step(
        &mut self,
        result: Result<(), DaemonEffectAdapterError>,
    ) -> Result<&mut Self, DaemonEffectAdapterError> {
        match result {
            Ok(()) if !self.failed => Ok(self),
            Ok(()) => Err(DaemonEffectAdapterError::TransactionAborted),
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    pub fn finish(self) -> Result<DaemonEffectAdapters, DaemonEffectAdapterError> {
        if self.failed {
            Err(DaemonEffectAdapterError::TransactionAborted)
        } else {
            Ok(self.adapters)
        }
    }
}

fn bind<T: ?Sized>(
    bindings: &mut BTreeMap<ProviderId, ExactEffect<T>>,
    descriptor: ProviderDescriptor,
    effect: Arc<T>,
) -> Result<(), DaemonEffectAdapterError> {
    if bindings.contains_key(&descriptor.provider_id) {
        return Err(DaemonEffectAdapterError::DuplicateBinding);
    }
    bindings.insert(
        descriptor.provider_id.clone(),
        ExactEffect { descriptor, effect },
    );
    Ok(())
}

fn resolve<T: ?Sized>(
    bindings: &BTreeMap<ProviderId, ExactEffect<T>>,
    descriptor: &ProviderDescriptor,
) -> Result<Arc<T>, DaemonEffectAdapterError> {
    let binding = bindings
        .get(&descriptor.provider_id)
        .ok_or(DaemonEffectAdapterError::MappingUnavailable)?;
    if binding.descriptor != *descriptor {
        return Err(DaemonEffectAdapterError::ConfigurationMismatch);
    }
    Ok(Arc::clone(&binding.effect))
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };

    use d2b_contracts::{
        v2_component_session::EndpointRole,
        v2_identity::ProviderType,
        v2_provider::{
            ObservabilityExportFormat, ProviderOperationInput, ProviderPlacement, ProviderTarget,
        },
    };
    use d2b_provider_observability_local::{
        LocalObservabilityProvider, ObservabilityLimits, live_observability_capabilities,
    };
    use d2b_provider_toolkit::Fixture;

    use super::*;

    #[test]
    fn missing_and_mismatched_mappings_fail_closed() {
        let descriptor = Fixture::new(ProviderType::Runtime, 1)
            .expect("fixture")
            .descriptor;
        let adapters = DaemonEffectAdapters::builder()
            .finish()
            .expect("empty adapter set");
        assert!(matches!(
            adapters.runtime(&descriptor),
            Err(DaemonEffectAdapterError::MappingUnavailable)
        ));

        let mut bindings = BTreeMap::new();
        bind(&mut bindings, descriptor.clone(), Arc::new(1_u8)).expect("bind exact effect");
        let mut mismatched = descriptor;
        mismatched.registry_generation =
            d2b_contracts::v2_provider::Generation::new(2).expect("generation");
        assert!(matches!(
            resolve(&bindings, &mismatched),
            Err(DaemonEffectAdapterError::ConfigurationMismatch)
        ));
    }

    #[tokio::test]
    async fn concrete_observability_adapter_bounds_and_redacts_query_and_export() {
        let mut fixture = Fixture::new(ProviderType::Observability, 1).expect("fixture");
        fixture.descriptor.implementation_id =
            d2b_provider_observability_local::implementation_id();
        fixture.descriptor.capabilities = live_observability_capabilities().expect("capabilities");
        let realm_id = fixture.descriptor.placement.realm_id().clone();
        fixture.descriptor.placement = ProviderPlacement::TrustedFirstPartyInProcess {
            realm_id: realm_id.clone(),
            controller_role: EndpointRole::LocalRootController,
        };
        let now = DaemonLocalObservability::now_unix_ms().expect("wall clock");
        let fixture = Fixture::from_descriptor(
            fixture.descriptor,
            ProviderTarget::Realm {
                realm_id: realm_id.clone(),
            },
            now,
        )
        .expect("observability fixture");

        let metrics = Arc::new(crate::metrics::Registry::new());
        metrics.counter_inc(
            "d2b_daemon_workload_lifecycle_total",
            &[
                ("provider", "private-cardinality-canary"),
                ("operation", "start"),
                ("outcome", "success"),
            ],
        );
        metrics.histogram_observe(
            "d2b_daemon_vm_start_duration_seconds",
            &[("vm", "private-vm-canary"), ("outcome", "success")],
            1.25,
        );
        let connections = crate::concurrency::ConnSemaphore::new(2);
        let _connection = connections.try_acquire().expect("connection permit");
        let adapter = Arc::new(DaemonLocalObservability {
            realm_id,
            metrics,
            audit: Arc::new(crate::daemon_audit::DaemonAuditLog::no_op()),
            connections,
        });
        let provider = LocalObservabilityProvider::new(
            fixture.descriptor.clone(),
            ObservabilityLimits::new(2, 1_024, 60_000).expect("limits"),
            adapter.clone(),
            adapter,
        )
        .expect("provider");

        let first_request = fixture
            .request_with_input(
                ProviderMethod::ObservabilityQuery,
                ProviderOperationInput::ObservabilityQuery {
                    view: ObservabilityView::Operations,
                    cursor: None,
                    limit: 1,
                },
            )
            .expect("query request");
        let first_call = fixture.call_context(&first_request.context);
        let first = provider
            .bounded_query(&first_call, &first_request)
            .await
            .expect("first query page");
        assert_eq!(first.records().len(), 1);
        assert!(first.truncated());
        let cursor = first.next_cursor().cloned().expect("next cursor");

        let second_request = fixture
            .request_with_input(
                ProviderMethod::ObservabilityQuery,
                ProviderOperationInput::ObservabilityQuery {
                    view: ObservabilityView::Operations,
                    cursor: Some(cursor),
                    limit: 1,
                },
            )
            .expect("second query request");
        let second_call = fixture.call_context(&second_request.context);
        let second = provider
            .bounded_query(&second_call, &second_request)
            .await
            .expect("second query page");
        assert_eq!(second.records().len(), 1);
        assert!(!second.truncated());

        let export_request = fixture
            .request_with_input(
                ProviderMethod::ObservabilityExport,
                ProviderOperationInput::ObservabilityExport {
                    format: ObservabilityExportFormat::JsonLines,
                    start_at_unix_ms: now.saturating_sub(1_000),
                    end_at_unix_ms: now.saturating_add(1_000),
                },
            )
            .expect("export request");
        let export_call = fixture.call_context(&export_request.context);
        let export = provider
            .bounded_export(&export_call, &export_request)
            .await
            .expect("bounded export");
        assert!(export.record_count() <= 2);
        assert!(export.encoded_bytes() <= 1_024);
        let debug = format!("{first:?}{second:?}{export:?}");
        for forbidden in ["private-cardinality-canary", "private-vm-canary"] {
            assert!(!debug.contains(forbidden));
        }
    }

    #[tokio::test]
    async fn cancelled_waiter_leaves_cleanup_running_and_retry_joins() {
        let tasks = ProviderLifecycleTasks::default();
        let locks = Arc::new(crate::concurrency::OpLockManager::default());
        let lifecycle = locks.begin_mapped_lifecycle("vm-a");
        let key = LifecycleMutationKey {
            operation_id: OperationId::parse("lifecycle-cancellation").expect("operation id"),
            idempotency_key: IdempotencyKey::parse("lifecycle-cancellation")
                .expect("idempotency key"),
            method: ProviderMethod::RuntimeStop,
        };
        let dispatches = Arc::new(AtomicUsize::new(0));
        let cleanup_complete = Arc::new(AtomicBool::new(false));
        let task_dispatches = Arc::clone(&dispatches);
        let task_cleanup = Arc::clone(&cleanup_complete);
        let first = tasks
            .spawn(key.clone(), move || {
                let _lifecycle = lifecycle;
                task_dispatches.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(100));
                task_cleanup.store(true, Ordering::SeqCst);
                Ok(serde_json::json!({ "outcome": "applied" }))
            })
            .expect("spawn lifecycle cleanup");

        assert!(
            tokio::time::timeout(Duration::from_millis(1), first.wait())
                .await
                .is_err(),
            "the provider waiter must time out before cleanup completes"
        );
        assert!(
            locks.mapped_lifecycle_active("vm-a"),
            "the tracked worker must retain VM serialization after waiter timeout"
        );
        let (fresh_tx, fresh_rx) = mpsc::channel();
        let fresh_locks = Arc::clone(&locks);
        let fresh = std::thread::spawn(move || {
            let _fresh_lifecycle = fresh_locks.begin_mapped_lifecycle("vm-a");
            fresh_tx.send(()).expect("signal fresh lifecycle admission");
        });
        assert!(
            fresh_rx.recv_timeout(Duration::from_millis(10)).is_err(),
            "a fresh operation identity must wait for the timed-out cleanup"
        );
        let retry_dispatches = Arc::clone(&dispatches);
        let retry = tasks
            .spawn(key, move || {
                retry_dispatches.fetch_add(1, Ordering::SeqCst);
                Ok(serde_json::json!({ "outcome": "applied" }))
            })
            .expect("join lifecycle cleanup");
        assert!(Arc::ptr_eq(&first, &retry));
        let result = retry.wait().await.expect("cleanup result");
        assert_eq!(result["outcome"], "applied");
        assert!(cleanup_complete.load(Ordering::SeqCst));
        assert_eq!(
            dispatches.load(Ordering::SeqCst),
            1,
            "retry must not dispatch a second lifecycle mutation"
        );
        fresh_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("fresh lifecycle admitted after cleanup");
        fresh.join().expect("join fresh lifecycle");
        assert!(!locks.mapped_lifecycle_active("vm-a"));
    }
}
