use std::{
    collections::BTreeMap,
    fmt,
    future::{Future, ready},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole, ServicePackage},
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        Fingerprint, Generation, MAX_PROVIDER_REGISTRY_ENTRIES, PROVIDER_SCHEMA_VERSION,
        ProviderDescriptor, ProviderFactoryKey, ProviderMethod, ProviderOperationContext,
        ProviderRegistryAxis, ProviderRegistrySnapshot, ProviderRegistryUpdate,
        RegistryDrainPolicy, RegistryLifecycle,
    },
};
use tokio::{sync::Notify, time};

use crate::{
    CancellationToken, OwnedOperationContext, ProviderFactory, ProviderInstance,
    ProviderRuntimeError, RegistryBuildError, RegistryShutdownReport,
};

const ACCEPTING: u8 = 0;
const DRAINING: u8 = 1;
const RETIRED: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryLimits {
    pub total_in_flight: usize,
    pub per_provider_in_flight: usize,
}

impl RegistryLimits {
    pub fn validate(self) -> Result<Self, RegistryBuildError> {
        if self.total_in_flight == 0
            || self.per_provider_in_flight == 0
            || self.per_provider_in_flight > self.total_in_flight
        {
            Err(RegistryBuildError::BoundExceeded)
        } else {
            Ok(self)
        }
    }
}

impl Default for RegistryLimits {
    fn default() -> Self {
        Self {
            total_in_flight: 256,
            per_provider_in_flight: 32,
        }
    }
}

#[derive(Clone)]
pub struct AdmissionOptions {
    pub expected_method: ProviderMethod,
    pub peer_role: EndpointRole,
    pub service: ServicePackage,
    pub deadline_after: Duration,
    pub caller_cancellation: CancellationToken,
    pub now_unix_ms: u64,
}

impl fmt::Debug for AdmissionOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmissionOptions")
            .field("expected_method", &self.expected_method)
            .field("peer_role", &self.peer_role)
            .field("service", &self.service)
            .field("deadline_after", &self.deadline_after)
            .field("cancelled", &self.caller_cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

struct InFlightState {
    total: usize,
    by_provider: BTreeMap<ProviderId, usize>,
}

struct RegistryInner {
    snapshot: ProviderRegistrySnapshot,
    instances: BTreeMap<ProviderId, ProviderInstance>,
    lifecycle: AtomicU8,
    limits: RegistryLimits,
    in_flight: Mutex<InFlightState>,
    drained: Notify,
    cancellation: CancellationToken,
}

#[derive(Clone)]
pub struct ProviderRegistry {
    inner: Arc<RegistryInner>,
}

impl fmt::Debug for ProviderRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRegistry")
            .field("generation", &self.inner.snapshot.generation)
            .field("lifecycle", &self.lifecycle())
            .field("provider_count", &self.inner.instances.len())
            .finish_non_exhaustive()
    }
}

pub struct ProviderRegistryBuilder {
    generation: Generation,
    configuration_fingerprint: Fingerprint,
    published_at_unix_ms: u64,
    factories: BTreeMap<ProviderFactoryKey, Arc<dyn ProviderFactory>>,
    instances: BTreeMap<ProviderId, ProviderInstance>,
    limits: RegistryLimits,
    failed: bool,
}

impl fmt::Debug for ProviderRegistryBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRegistryBuilder")
            .field("generation", &self.generation)
            .field("factory_count", &self.factories.len())
            .field("instance_count", &self.instances.len())
            .finish_non_exhaustive()
    }
}

impl ProviderRegistryBuilder {
    pub fn new(
        generation: Generation,
        configuration_fingerprint: Fingerprint,
        published_at_unix_ms: u64,
    ) -> Self {
        Self {
            generation,
            configuration_fingerprint,
            published_at_unix_ms,
            factories: BTreeMap::new(),
            instances: BTreeMap::new(),
            limits: RegistryLimits::default(),
            failed: false,
        }
    }

    pub fn limits(&mut self, limits: RegistryLimits) -> Result<&mut Self, RegistryBuildError> {
        match limits.validate() {
            Ok(limits) => {
                self.limits = limits;
                Ok(self)
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    pub fn register_factory(
        &mut self,
        key: ProviderFactoryKey,
        factory: Arc<dyn ProviderFactory>,
    ) -> Result<&mut Self, RegistryBuildError> {
        let result = if self.factories.len() >= MAX_PROVIDER_REGISTRY_ENTRIES {
            Err(RegistryBuildError::BoundExceeded)
        } else {
            match self.factories.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(factory);
                    Ok(())
                }
                std::collections::btree_map::Entry::Occupied(_) => {
                    Err(RegistryBuildError::DuplicateFactory)
                }
            }
        };
        self.finish_step(result)
    }

    pub fn register_instance(
        &mut self,
        descriptor: ProviderDescriptor,
    ) -> Result<&mut Self, RegistryBuildError> {
        let result = self.try_register_instance(descriptor);
        self.finish_step(result)
    }

    fn try_register_instance(
        &mut self,
        descriptor: ProviderDescriptor,
    ) -> Result<(), RegistryBuildError> {
        descriptor.validate()?;
        if descriptor.registry_generation != self.generation {
            return Err(RegistryBuildError::GenerationMismatch);
        }
        if self.instances.len() >= MAX_PROVIDER_REGISTRY_ENTRIES {
            return Err(RegistryBuildError::BoundExceeded);
        }
        if self.instances.contains_key(&descriptor.provider_id) {
            return Err(RegistryBuildError::DuplicateProvider);
        }
        let key = ProviderFactoryKey {
            provider_type: descriptor.provider_type(),
            implementation_id: descriptor.implementation_id.clone(),
        };
        let factory = self
            .factories
            .get(&key)
            .ok_or(RegistryBuildError::MissingFactory)?;
        let instance = factory
            .construct(&descriptor)
            .map_err(RegistryBuildError::FactoryFailed)?;
        self.validate_instance(&descriptor, &instance)?;
        self.instances
            .insert(descriptor.provider_id.clone(), instance);
        Ok(())
    }

    pub fn register_constructed(
        &mut self,
        key: ProviderFactoryKey,
        instance: ProviderInstance,
    ) -> Result<&mut Self, RegistryBuildError> {
        let result = self.try_register_constructed(key, instance);
        self.finish_step(result)
    }

    fn try_register_constructed(
        &mut self,
        key: ProviderFactoryKey,
        instance: ProviderInstance,
    ) -> Result<(), RegistryBuildError> {
        if !self.factories.contains_key(&key) {
            return Err(RegistryBuildError::MissingFactory);
        }
        let descriptor = instance.descriptor();
        if key.provider_type != descriptor.provider_type()
            || key.implementation_id != descriptor.implementation_id
        {
            return Err(RegistryBuildError::DescriptorMismatch);
        }
        if descriptor.registry_generation != self.generation {
            return Err(RegistryBuildError::GenerationMismatch);
        }
        descriptor.validate()?;
        self.validate_instance(&descriptor, &instance)?;
        if self.instances.len() >= MAX_PROVIDER_REGISTRY_ENTRIES {
            return Err(RegistryBuildError::BoundExceeded);
        }
        if self.instances.contains_key(&descriptor.provider_id) {
            return Err(RegistryBuildError::DuplicateProvider);
        }
        self.instances.insert(descriptor.provider_id, instance);
        Ok(())
    }

    fn finish_step(
        &mut self,
        result: Result<(), RegistryBuildError>,
    ) -> Result<&mut Self, RegistryBuildError> {
        match result {
            Ok(()) if !self.failed => Ok(self),
            Ok(()) => Err(RegistryBuildError::TransactionAborted),
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    fn validate_instance(
        &self,
        descriptor: &ProviderDescriptor,
        instance: &ProviderInstance,
    ) -> Result<(), RegistryBuildError> {
        let actual_descriptor = instance.descriptor();
        if instance.provider_type() != descriptor.provider_type()
            || actual_descriptor != *descriptor
        {
            return Err(RegistryBuildError::DescriptorMismatch);
        }
        if instance.capabilities() != descriptor.capabilities {
            return Err(RegistryBuildError::CapabilityMismatch);
        }
        if !instance.validate_capability_dispatch() {
            return Err(RegistryBuildError::CapabilityMismatch);
        }
        Ok(())
    }

    pub fn finish(self) -> Result<ProviderRegistry, RegistryBuildError> {
        if self.failed {
            return Err(RegistryBuildError::TransactionAborted);
        }
        if self.instances.is_empty() {
            return Err(RegistryBuildError::EmptyRegistry);
        }
        let factory_keys: Vec<_> = self.factories.keys().cloned().collect();
        let descriptors: Vec<_> = self
            .instances
            .values()
            .map(ProviderInstance::descriptor)
            .collect();
        let axes = ProviderType::ALL
            .into_iter()
            .map(|provider_type| {
                let providers = descriptors
                    .iter()
                    .filter(|descriptor| descriptor.provider_type() == provider_type)
                    .map(|descriptor| descriptor.provider_id.clone())
                    .collect();
                Ok(ProviderRegistryAxis {
                    provider_type,
                    providers: BoundedVec::new(providers)
                        .map_err(|_| RegistryBuildError::BoundExceeded)?,
                })
            })
            .collect::<Result<Vec<_>, RegistryBuildError>>()?;
        let snapshot = ProviderRegistrySnapshot {
            schema_version: PROVIDER_SCHEMA_VERSION,
            generation: self.generation,
            configuration_fingerprint: self.configuration_fingerprint,
            published_at_unix_ms: self.published_at_unix_ms,
            lifecycle: RegistryLifecycle::Accepting,
            axes: BoundedVec::new(axes).map_err(|_| RegistryBuildError::BoundExceeded)?,
            factories: BoundedVec::new(factory_keys)
                .map_err(|_| RegistryBuildError::BoundExceeded)?,
            providers: BoundedVec::new(descriptors)
                .map_err(|_| RegistryBuildError::BoundExceeded)?,
        };
        snapshot.validate()?;
        Ok(ProviderRegistry {
            inner: Arc::new(RegistryInner {
                snapshot,
                instances: self.instances,
                lifecycle: AtomicU8::new(ACCEPTING),
                limits: self.limits,
                in_flight: Mutex::new(InFlightState {
                    total: 0,
                    by_provider: BTreeMap::new(),
                }),
                drained: Notify::new(),
                cancellation: CancellationToken::new(),
            }),
        })
    }
}

impl ProviderRegistry {
    pub fn lifecycle(&self) -> RegistryLifecycle {
        match self.inner.lifecycle.load(Ordering::Acquire) {
            ACCEPTING => RegistryLifecycle::Accepting,
            DRAINING => RegistryLifecycle::Draining,
            _ => RegistryLifecycle::Retired,
        }
    }

    pub fn snapshot(&self) -> ProviderRegistrySnapshot {
        let mut snapshot = self.inner.snapshot.clone();
        snapshot.lifecycle = self.lifecycle();
        snapshot
    }

    pub fn instance(&self, provider_id: &ProviderId) -> Option<ProviderInstance> {
        self.inner.instances.get(provider_id).cloned()
    }

    pub fn admit(
        &self,
        operation: ProviderOperationContext,
        options: AdmissionOptions,
    ) -> Result<AdmittedProvider, ProviderRuntimeError> {
        if self.lifecycle() != RegistryLifecycle::Accepting {
            return Err(ProviderRuntimeError::NotAccepting);
        }
        let instance = self
            .inner
            .instances
            .get(&operation.provider_id)
            .cloned()
            .ok_or(ProviderRuntimeError::UnknownProvider)?;
        let descriptor = instance.descriptor();
        operation.validate(&descriptor, options.now_unix_ms)?;
        if operation.method != options.expected_method
            || !instance
                .capabilities()
                .contains_method(options.expected_method)
        {
            return Err(ProviderRuntimeError::CapabilityDenied);
        }
        let permit = self.acquire(&descriptor.provider_id)?;
        let context = OwnedOperationContext::new_linked(
            operation,
            options.peer_role,
            options.service,
            options.deadline_after,
            vec![options.caller_cancellation, self.inner.cancellation.clone()],
        )?;
        Ok(AdmittedProvider {
            instance,
            context,
            _permit: permit,
        })
    }

    fn acquire(&self, provider_id: &ProviderId) -> Result<InFlightPermit, ProviderRuntimeError> {
        let mut state = self
            .inner
            .in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let provider_count = state.by_provider.get(provider_id).copied().unwrap_or(0);
        if state.total >= self.inner.limits.total_in_flight
            || provider_count >= self.inner.limits.per_provider_in_flight
        {
            return Err(ProviderRuntimeError::InFlightLimit);
        }
        state.total += 1;
        state
            .by_provider
            .insert(provider_id.clone(), provider_count + 1);
        if self.lifecycle() != RegistryLifecycle::Accepting {
            state.total -= 1;
            if provider_count == 0 {
                state.by_provider.remove(provider_id);
            } else {
                state
                    .by_provider
                    .insert(provider_id.clone(), provider_count);
            }
            return Err(ProviderRuntimeError::NotAccepting);
        }
        Ok(InFlightPermit {
            registry: self.inner.clone(),
            provider_id: provider_id.clone(),
        })
    }

    pub async fn shutdown(
        &self,
        policy: &RegistryDrainPolicy,
    ) -> Result<RegistryShutdownReport, ProviderRuntimeError> {
        policy.validate()?;
        self.inner
            .lifecycle
            .compare_exchange(ACCEPTING, DRAINING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ProviderRuntimeError::InvalidLifecycleTransition)?;
        self.inner.cancellation.cancel();

        let wait_for_drain =
            wait_until_drained(&self.inner.in_flight, &self.inner.drained, || ready(()));
        let drained = time::timeout(
            Duration::from_millis(u64::from(policy.drain_deadline_ms)),
            wait_for_drain,
        )
        .await
        .is_ok();
        let unresolved_in_flight = self
            .inner
            .in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .total;
        self.inner.lifecycle.store(RETIRED, Ordering::Release);
        Ok(RegistryShutdownReport {
            drained,
            unresolved_in_flight,
        })
    }
}

pub struct InFlightPermit {
    registry: Arc<RegistryInner>,
    provider_id: ProviderId,
}

impl fmt::Debug for InFlightPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InFlightPermit(<redacted>)")
    }
}

impl Drop for InFlightPermit {
    fn drop(&mut self) {
        let mut state = self
            .registry
            .in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.total = state.total.saturating_sub(1);
        if let Some(count) = state.by_provider.get_mut(&self.provider_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.by_provider.remove(&self.provider_id);
            }
        }
        if state.total == 0 {
            self.registry.drained.notify_waiters();
        }
    }
}

pub struct AdmittedProvider {
    pub instance: ProviderInstance,
    pub context: OwnedOperationContext,
    _permit: InFlightPermit,
}

impl fmt::Debug for AdmittedProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedProvider")
            .field("provider_type", &self.instance.provider_type())
            .field("context", &self.context)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct ProviderRegistryManager {
    current: Arc<RwLock<Arc<ProviderRegistry>>>,
}

impl ProviderRegistryManager {
    pub fn new(initial: ProviderRegistry) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(initial))),
        }
    }

    pub fn current(&self) -> Arc<ProviderRegistry> {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub async fn publish(
        &self,
        replacement: ProviderRegistry,
        policy: RegistryDrainPolicy,
    ) -> Result<RegistryShutdownReport, ProviderRuntimeError> {
        let old = self.current();
        let update = ProviderRegistryUpdate {
            from_generation: old.inner.snapshot.generation,
            from_configuration_fingerprint: old.inner.snapshot.configuration_fingerprint.clone(),
            replacement: replacement.snapshot(),
            drain_policy: policy.clone(),
        };
        update.validate(&old.snapshot())?;
        old.inner
            .lifecycle
            .compare_exchange(ACCEPTING, DRAINING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ProviderRuntimeError::InvalidLifecycleTransition)?;
        old.inner.cancellation.cancel();
        {
            let mut current = self.current.write().unwrap_or_else(|e| e.into_inner());
            *current = Arc::new(replacement);
        }
        old.finish_drain(&policy).await
    }
}

impl ProviderRegistry {
    async fn finish_drain(
        &self,
        policy: &RegistryDrainPolicy,
    ) -> Result<RegistryShutdownReport, ProviderRuntimeError> {
        let wait_for_drain =
            wait_until_drained(&self.inner.in_flight, &self.inner.drained, || ready(()));
        let drained = time::timeout(
            Duration::from_millis(u64::from(policy.drain_deadline_ms)),
            wait_for_drain,
        )
        .await
        .is_ok();
        let unresolved_in_flight = self
            .inner
            .in_flight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .total;
        self.inner.lifecycle.store(RETIRED, Ordering::Release);
        Ok(RegistryShutdownReport {
            drained,
            unresolved_in_flight,
        })
    }
}

async fn wait_until_drained<F, Fut>(
    in_flight: &Mutex<InFlightState>,
    drained: &Notify,
    mut before_await: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    loop {
        let notified = drained.notified();
        let total = in_flight
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .total;
        if total == 0 {
            break;
        }
        before_await().await;
        notified.await;
    }
}

#[cfg(test)]
mod tests {
    use super::{InFlightState, wait_until_drained};
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::{
        sync::{Barrier, Notify},
        time,
    };

    struct FinalPermit {
        in_flight: Arc<Mutex<InFlightState>>,
        drained: Arc<Notify>,
    }

    impl Drop for FinalPermit {
        fn drop(&mut self) {
            self.in_flight
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .total = 0;
            self.drained.notify_waiters();
        }
    }

    async fn prove_final_drop_between_check_and_await_completes() {
        let in_flight = Arc::new(Mutex::new(InFlightState {
            total: 1,
            by_provider: BTreeMap::new(),
        }));
        let drained = Arc::new(Notify::new());
        let barrier = Arc::new(Barrier::new(2));
        let permit = FinalPermit {
            in_flight: in_flight.clone(),
            drained: drained.clone(),
        };

        let waiter = {
            let barrier = barrier.clone();
            tokio::spawn(async move {
                wait_until_drained(&in_flight, &drained, move || {
                    let barrier = barrier.clone();
                    async move {
                        barrier.wait().await;
                        barrier.wait().await;
                    }
                })
                .await;
            })
        };

        barrier.wait().await;
        drop(permit);
        barrier.wait().await;
        time::timeout(Duration::from_millis(100), waiter)
            .await
            .expect("armed drain waiter must observe the final permit notification")
            .expect("drain waiter must not panic");
    }

    #[tokio::test]
    async fn shutdown_closes_final_permit_notify_race() {
        prove_final_drop_between_check_and_await_completes().await;
    }

    #[tokio::test]
    async fn finish_drain_closes_final_permit_notify_race() {
        prove_final_drop_between_check_and_await_completes().await;
    }
}
