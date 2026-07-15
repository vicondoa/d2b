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
};

use async_trait::async_trait;
use d2b_contracts::{
    broker_wire::BrokerCallerRole,
    provider_registry_v2::{
        LocalRuntimeProviderBindingV2, ProviderBindingV2, ProviderRegistryEntryV2,
    },
    public_wire::{MutationFlags, VmLifecycleRequest},
    v2_identity::ProviderId,
    v2_provider::{
        AuthorizedProviderScope, HandleId, HandleOwner, MutationState, ObservationReason,
        ObservedLifecycleState, OperationId, PlanId, ProviderDescriptor,
    },
};
use d2b_provider_audio_pipewire_vhost_user::{AudioEffectPort, AudioQueryPort};
use d2b_provider_device_host_mediated::{DeviceEffectPort, DeviceQueryPort};
use d2b_provider_display_wayland::DisplayEffectPort;
use d2b_provider_network_local_realm::NetworkEffectPort;
use d2b_provider_observability_local::{ObservabilityExportPort, ObservabilityQueryPort};
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
    ServerState, TypedError, dispatch_broker_vm_start, dispatch_broker_vm_stop_as,
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
                            binding: binding.clone(),
                        });
                    builder.bind_runtime(entry.descriptor.clone(), adapter)?;
                }
            }
        }
        builder.finish()
    }
}

#[derive(Clone)]
struct DaemonLocalRuntimeControl {
    state: Weak<ServerState>,
    entry: ProviderRegistryEntryV2,
    binding: LocalRuntimeProviderBindingV2,
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
            || context.operation().scope.realm_id() != &self.binding.realm_id
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
        if request.target().realm_id() != &self.binding.realm_id
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
                realm_id: self.binding.realm_id.clone(),
                workload_id: self.binding.workload_id.clone(),
            },
            handle_id,
            HandleOwner::RealmController {
                realm_id: self.binding.realm_id.clone(),
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

    fn invoke_direct_start(
        &self,
        request: &RuntimeOperationControl,
        resolved: &ResolvedDaemonRuntime,
    ) -> Result<(), RuntimeControlError> {
        let (lifecycle, _, result_slot) = resolved
            .state
            .provider_registry()
            .map_err(|_| RuntimeControlError::Unavailable)?
            .lifecycle_dispatch()
            .invocation(&request.context().operation().operation_id, &resolved.vm)?;
        let result = dispatch_broker_vm_start(&resolved.state, lifecycle);
        let applied = result.as_ref().is_ok_and(Self::response_applied);
        *result_slot
            .lock()
            .map_err(|_| RuntimeControlError::Unavailable)? = Some(result);
        if applied {
            Ok(())
        } else {
            Err(RuntimeControlError::Unavailable)
        }
    }

    fn invoke_direct_stop(
        &self,
        request: &RuntimeOperationControl,
        resolved: &ResolvedDaemonRuntime,
    ) -> Result<(), RuntimeControlError> {
        let (lifecycle, caller_role, result_slot) = resolved
            .state
            .provider_registry()
            .map_err(|_| RuntimeControlError::Unavailable)?
            .lifecycle_dispatch()
            .invocation(&request.context().operation().operation_id, &resolved.vm)?;
        let result = dispatch_broker_vm_stop_as(&resolved.state, lifecycle, caller_role);
        let applied = result.as_ref().is_ok_and(Self::response_applied);
        *result_slot
            .lock()
            .map_err(|_| RuntimeControlError::Unavailable)? = Some(result);
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
        if resolved
            .state
            .pidfd_table
            .still_alive_same_start_time(&resolved.vm, &resolved.role)
        {
            return self.observed(request.context(), &resolved);
        }
        self.invoke_direct_start(&request, &resolved)?;
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
        if !resolved
            .state
            .pidfd_table
            .still_alive_same_start_time(&resolved.vm, &resolved.role)
        {
            return self.observed(request.context(), &resolved);
        }
        self.invoke_direct_stop(&request, &resolved)?;
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
        if !resolved
            .state
            .pidfd_table
            .still_alive_same_start_time(&resolved.vm, &resolved.role)
        {
            return Ok(RuntimeMutationOutcome::new(MutationState::NotApplicable));
        }
        let response = dispatch_broker_vm_stop_as(
            &resolved.state,
            Self::lifecycle_request(resolved.vm),
            BrokerCallerRole::AdminUid {
                uid: resolved.state.daemon_uid,
            },
        )
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
    use d2b_contracts::v2_identity::ProviderType;
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
}
