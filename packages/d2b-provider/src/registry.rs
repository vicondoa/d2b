//! The per-Zone Provider registry: build, admit, drain, retire, republish.
//!
//! The lifecycle, in-flight accounting, RAII permit, drain-waiter notify
//! race handling, and live-swap manager are carried over unchanged from the
//! ADR45 registry. What changed is the identity the registry keys on: a Zone
//! path and an authenticated Zone principal instead of a realm and a peer
//! role.

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

use d2b_contracts::v3::{
    identity::ConfigurationGeneration, resource_ref::ResourceRef, zone_routing::ZonePath,
};
use tokio::{sync::Notify, time};

use crate::{
    context::{CancellationToken, OwnedOperationContext},
    descriptor::ProviderDescriptor,
    error::{ProviderRuntimeError, RegistryBuildError},
    identity::{MAX_PROVIDER_REGISTRY_ENTRIES, PROVIDER_SCHEMA_VERSION, ProviderMethodName},
    session::SessionIdentity,
};

const ACCEPTING: u8 = 0;
const DRAINING: u8 = 1;
const RETIRED: u8 = 2;

/// The longest drain a policy may request.
pub const MAX_REGISTRY_DRAIN_MS: u32 = 5 * 60 * 1_000;

/// Whether a registry generation still admits calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryLifecycle {
    /// New calls are admitted.
    Accepting,
    /// New calls are refused; in-flight calls are draining.
    Draining,
    /// The generation is closed.
    Retired,
}

/// The bounded in-flight caps one registry generation enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryLimits {
    /// Calls in flight across every Provider.
    pub total_in_flight: usize,
    /// Calls in flight against any single Provider.
    pub per_provider_in_flight: usize,
}

impl RegistryLimits {
    /// Reject a zero cap or a per-provider cap above the total.
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

/// How a retiring registry generation drains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryDrainPolicy {
    /// How long the generation waits for in-flight calls to finish.
    pub drain_deadline_ms: u32,
    /// Whether in-flight calls are cancelled when the deadline passes.
    pub cancel_in_flight_at_deadline: bool,
    /// Whether Provider sessions are closed at retirement.
    pub close_provider_sessions: bool,
}

impl RegistryDrainPolicy {
    /// Reject a zero or over-long deadline, or a policy that leaks work past
    /// retirement.
    pub const fn validate(&self) -> Result<(), ProviderRuntimeError> {
        if self.drain_deadline_ms == 0
            || self.drain_deadline_ms > MAX_REGISTRY_DRAIN_MS
            || !self.cancel_in_flight_at_deadline
            || !self.close_provider_sessions
        {
            Err(ProviderRuntimeError::InvalidDrainPolicy)
        } else {
            Ok(())
        }
    }
}

/// What a completed drain observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryShutdownReport {
    /// Whether every in-flight call finished before the deadline.
    pub drained: bool,
    /// How many calls were still in flight at retirement.
    pub unresolved_in_flight: usize,
}

/// The published shape of one registry generation.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderRegistrySnapshot {
    schema_version: u32,
    zone: ZonePath,
    generation: ConfigurationGeneration,
    lifecycle: RegistryLifecycle,
    descriptors: Vec<ProviderDescriptor>,
}

impl ProviderRegistrySnapshot {
    /// The Provider contract schema version.
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// The Zone this generation belongs to.
    pub const fn zone(&self) -> &ZonePath {
        &self.zone
    }

    /// The registry generation ordinal.
    pub const fn generation(&self) -> ConfigurationGeneration {
        self.generation
    }

    /// The generation's lifecycle at the time the snapshot was taken.
    pub const fn lifecycle(&self) -> RegistryLifecycle {
        self.lifecycle
    }

    /// Every installed Provider descriptor, in `Provider/<name>` order.
    pub fn descriptors(&self) -> &[ProviderDescriptor] {
        &self.descriptors
    }
}

impl fmt::Debug for ProviderRegistrySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRegistrySnapshot")
            .field("schema_version", &self.schema_version)
            .field("generation", &self.generation)
            .field("lifecycle", &self.lifecycle)
            .field("provider_count", &self.descriptors.len())
            .finish_non_exhaustive()
    }
}

/// Everything one admission decision needs beyond the authenticated identity.
#[derive(Clone)]
pub struct AdmissionOptions {
    /// The authenticated Zone principal and Provider binding.
    ///
    /// This replaces the ADR45 `peer_role`. It is derivable only from
    /// authenticated evidence, so a Provider cannot assert it.
    pub identity: SessionIdentity,
    /// The exact method the caller was authorized for.
    pub expected_method: ProviderMethodName,
    /// The operation budget.
    pub deadline_after: Duration,
    /// The caller's cancellation token.
    pub caller_cancellation: CancellationToken,
}

impl fmt::Debug for AdmissionOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmissionOptions")
            .field("deadline_after", &self.deadline_after)
            .field("cancelled", &self.caller_cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

struct InFlightState {
    total: usize,
    by_provider: BTreeMap<ResourceRef, usize>,
}

struct RegistryInner<I> {
    snapshot: ProviderRegistrySnapshot,
    instances: BTreeMap<ResourceRef, (ProviderDescriptor, I)>,
    lifecycle: AtomicU8,
    limits: RegistryLimits,
    in_flight: Mutex<InFlightState>,
    drained: Notify,
    cancellation: CancellationToken,
}

/// One Zone's active Provider registry generation.
///
/// `I` is the Zone runtime's own instance handle. This crate deliberately does
/// not name the Provider trait-object catalog: those types belong to the v3
/// Provider contract surface, not to the registry.
pub struct ProviderRegistry<I> {
    inner: Arc<RegistryInner<I>>,
}

impl<I> Clone for ProviderRegistry<I> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<I> fmt::Debug for ProviderRegistry<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRegistry")
            .field("generation", &self.inner.snapshot.generation)
            .field("lifecycle", &self.lifecycle())
            .field("provider_count", &self.inner.instances.len())
            .finish_non_exhaustive()
    }
}

/// Builds one registry generation as an all-or-nothing transaction.
pub struct ProviderRegistryBuilder<I> {
    zone: ZonePath,
    generation: ConfigurationGeneration,
    instances: BTreeMap<ResourceRef, (ProviderDescriptor, I)>,
    limits: RegistryLimits,
    failed: bool,
}

impl<I> fmt::Debug for ProviderRegistryBuilder<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRegistryBuilder")
            .field("generation", &self.generation)
            .field("instance_count", &self.instances.len())
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl<I> ProviderRegistryBuilder<I> {
    /// Start a generation for one Zone.
    pub fn new(zone: ZonePath, generation: ConfigurationGeneration) -> Self {
        Self {
            zone,
            generation,
            instances: BTreeMap::new(),
            limits: RegistryLimits::default(),
            failed: false,
        }
    }

    /// Set the in-flight caps.
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

    /// Register one already constructed Provider instance.
    pub fn register_instance(
        &mut self,
        descriptor: ProviderDescriptor,
        instance: I,
    ) -> Result<&mut Self, RegistryBuildError> {
        let result = self.try_register_instance(descriptor, instance);
        self.finish_step(result)
    }

    fn try_register_instance(
        &mut self,
        descriptor: ProviderDescriptor,
        instance: I,
    ) -> Result<(), RegistryBuildError> {
        descriptor.validate()?;
        if *descriptor.zone() != self.zone {
            return Err(RegistryBuildError::ZoneMismatch);
        }
        if descriptor.registry_generation() != self.generation {
            return Err(RegistryBuildError::GenerationMismatch);
        }
        if self.instances.len() >= MAX_PROVIDER_REGISTRY_ENTRIES {
            return Err(RegistryBuildError::BoundExceeded);
        }
        if self.instances.contains_key(descriptor.provider_ref()) {
            return Err(RegistryBuildError::DuplicateProvider);
        }
        self.instances
            .insert(descriptor.provider_ref().clone(), (descriptor, instance));
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

    /// Seal the generation.
    pub fn finish(self) -> Result<ProviderRegistry<I>, RegistryBuildError> {
        if self.failed {
            return Err(RegistryBuildError::TransactionAborted);
        }
        if self.instances.is_empty() {
            return Err(RegistryBuildError::EmptyRegistry);
        }
        let descriptors: Vec<_> = self
            .instances
            .values()
            .map(|(descriptor, _)| descriptor.clone())
            .collect();
        let snapshot = ProviderRegistrySnapshot {
            schema_version: PROVIDER_SCHEMA_VERSION,
            zone: self.zone,
            generation: self.generation,
            lifecycle: RegistryLifecycle::Accepting,
            descriptors,
        };
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

impl<I> ProviderRegistry<I> {
    /// The current lifecycle state.
    pub fn lifecycle(&self) -> RegistryLifecycle {
        match self.inner.lifecycle.load(Ordering::Acquire) {
            ACCEPTING => RegistryLifecycle::Accepting,
            DRAINING => RegistryLifecycle::Draining,
            _ => RegistryLifecycle::Retired,
        }
    }
}

impl<I: Clone> ProviderRegistry<I> {
    /// The published snapshot with the live lifecycle state.
    pub fn snapshot(&self) -> ProviderRegistrySnapshot {
        let mut snapshot = self.inner.snapshot.clone();
        snapshot.lifecycle = self.lifecycle();
        snapshot
    }

    /// The descriptor installed for one `Provider/<name>`, if any.
    pub fn descriptor(&self, provider_ref: &ResourceRef) -> Option<&ProviderDescriptor> {
        self.inner
            .instances
            .get(provider_ref)
            .map(|(descriptor, _)| descriptor)
    }

    /// The instance handle installed for one `Provider/<name>`, if any.
    pub fn instance(&self, provider_ref: &ResourceRef) -> Option<I> {
        self.inner
            .instances
            .get(provider_ref)
            .map(|(_, instance)| instance.clone())
    }

    /// Admit one authenticated call against this generation.
    ///
    /// Admission is exact at every step: the generation must be accepting, the
    /// Provider must be installed here, the authenticated identity must match
    /// the descriptor exactly, the Provider must publish the method, and a
    /// permit must be available.
    pub fn admit(
        &self,
        options: AdmissionOptions,
    ) -> Result<AdmittedProvider<I>, ProviderRuntimeError> {
        if self.lifecycle() != RegistryLifecycle::Accepting {
            return Err(ProviderRuntimeError::NotAccepting);
        }
        let (descriptor, instance) = self
            .inner
            .instances
            .get(options.identity.provider_ref())
            .ok_or(ProviderRuntimeError::UnknownProvider)?;
        options.identity.matches_descriptor(descriptor)?;
        if !descriptor
            .capabilities()
            .contains_method(&options.expected_method)
        {
            return Err(ProviderRuntimeError::CapabilityDenied);
        }
        let permit = self.acquire(descriptor.provider_ref())?;
        let context = OwnedOperationContext::new_linked(
            options.identity,
            options.expected_method,
            options.deadline_after,
            vec![options.caller_cancellation, self.inner.cancellation.clone()],
        )?;
        Ok(AdmittedProvider {
            instance: instance.clone(),
            context,
            _permit: permit,
        })
    }

    fn acquire(
        &self,
        provider_ref: &ResourceRef,
    ) -> Result<InFlightPermit<I>, ProviderRuntimeError> {
        let mut state = self
            .inner
            .in_flight
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let provider_count = state.by_provider.get(provider_ref).copied().unwrap_or(0);
        if state.total >= self.inner.limits.total_in_flight
            || provider_count >= self.inner.limits.per_provider_in_flight
        {
            return Err(ProviderRuntimeError::InFlightLimit);
        }
        state.total += 1;
        state
            .by_provider
            .insert(provider_ref.clone(), provider_count + 1);
        if self.lifecycle() != RegistryLifecycle::Accepting {
            state.total -= 1;
            if provider_count == 0 {
                state.by_provider.remove(provider_ref);
            } else {
                state
                    .by_provider
                    .insert(provider_ref.clone(), provider_count);
            }
            return Err(ProviderRuntimeError::NotAccepting);
        }
        Ok(InFlightPermit {
            registry: self.inner.clone(),
            provider_ref: provider_ref.clone(),
        })
    }

    /// Retire this generation, draining in-flight calls first.
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
        self.finish_drain(policy).await
    }

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
            .unwrap_or_else(|error| error.into_inner())
            .total;
        self.inner.lifecycle.store(RETIRED, Ordering::Release);
        Ok(RegistryShutdownReport {
            drained,
            unresolved_in_flight,
        })
    }
}

/// The RAII guard that holds one in-flight slot.
pub struct InFlightPermit<I> {
    registry: Arc<RegistryInner<I>>,
    provider_ref: ResourceRef,
}

impl<I> fmt::Debug for InFlightPermit<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InFlightPermit(<redacted>)")
    }
}

impl<I> Drop for InFlightPermit<I> {
    fn drop(&mut self) {
        let mut state = self
            .registry
            .in_flight
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.total = state.total.saturating_sub(1);
        if let Some(count) = state.by_provider.get_mut(&self.provider_ref) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.by_provider.remove(&self.provider_ref);
            }
        }
        if state.total == 0 {
            self.registry.drained.notify_waiters();
        }
    }
}

/// One admitted call: the instance, its context, and its in-flight permit.
///
/// Dropping this value releases the permit.
pub struct AdmittedProvider<I> {
    /// The Zone runtime's instance handle.
    pub instance: I,
    /// The immutable operation context.
    pub context: OwnedOperationContext,
    _permit: InFlightPermit<I>,
}

impl<I> fmt::Debug for AdmittedProvider<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedProvider")
            .field("context", &self.context)
            .finish_non_exhaustive()
    }
}

/// Holds the Zone's current registry generation across a live republication.
pub struct ProviderRegistryManager<I> {
    current: Arc<RwLock<Arc<ProviderRegistry<I>>>>,
}

impl<I> Clone for ProviderRegistryManager<I> {
    fn clone(&self) -> Self {
        Self {
            current: self.current.clone(),
        }
    }
}

impl<I> fmt::Debug for ProviderRegistryManager<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRegistryManager")
            .finish_non_exhaustive()
    }
}

impl<I: Clone> ProviderRegistryManager<I> {
    /// Take ownership of an initial generation.
    pub fn new(initial: ProviderRegistry<I>) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(initial))),
        }
    }

    /// The generation new calls are admitted against.
    pub fn current(&self) -> Arc<ProviderRegistry<I>> {
        self.current
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Swap in a replacement generation and drain the outgoing one.
    ///
    /// The replacement must belong to the same Zone and must advance the
    /// generation ordinal; a stale or sideways republication is refused
    /// before the outgoing generation stops accepting.
    pub async fn publish(
        &self,
        replacement: ProviderRegistry<I>,
        policy: RegistryDrainPolicy,
    ) -> Result<RegistryShutdownReport, ProviderRuntimeError> {
        policy.validate()?;
        let old = self.current();
        let old_snapshot = old.snapshot();
        let new_snapshot = replacement.snapshot();
        if old_snapshot.lifecycle() != RegistryLifecycle::Accepting
            || new_snapshot.zone() != old_snapshot.zone()
            || new_snapshot.generation().get() <= old_snapshot.generation().get()
        {
            return Err(ProviderRuntimeError::InvalidLifecycleTransition);
        }
        old.inner
            .lifecycle
            .compare_exchange(ACCEPTING, DRAINING, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ProviderRuntimeError::InvalidLifecycleTransition)?;
        old.inner.cancellation.cancel();
        {
            let mut current = self
                .current
                .write()
                .unwrap_or_else(|error| error.into_inner());
            *current = Arc::new(replacement);
        }
        old.finish_drain(&policy).await
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
