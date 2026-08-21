//! Supervisor-owned notification source and host-sink lifecycle contracts.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use d2b_contracts_resource::v3::{
    ResourceRef,
    ZoneId,
};
use sha2::{Digest, Sha256};

/// Exact, generation-bound identity for one notification Guest source.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NotificationSourceIdentity {
    zone: ZoneId,
    provider_ref: ResourceRef,
    source_ref: ResourceRef,
    source_generation: u64,
    display_generation: u64,
    endpoint_digest: String,
}

impl NotificationSourceIdentity {
    /// Construct a validated source lifecycle identity.
    pub fn new(
        zone: ZoneId,
        provider_ref: ResourceRef,
        source_ref: ResourceRef,
        source_generation: u64,
        display_generation: u64,
        endpoint_digest: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let endpoint_digest = endpoint_digest.into();
        if provider_ref.resource_type().as_str() != "Provider"
            || source_ref.resource_type().as_str() != "Guest"
            || source_generation == 0
            || display_generation == 0
            || endpoint_digest.is_empty()
            || endpoint_digest.len() > 128
        {
            return Err("notification-lifecycle-source-invalid");
        }
        Ok(Self {
            zone,
            provider_ref,
            source_ref,
            source_generation,
            display_generation,
            endpoint_digest,
        })
    }

    /// Borrow the source Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the owning Provider.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the exact Guest source reference.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }
}

impl core::fmt::Debug for NotificationSourceIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NotificationSourceIdentity(<redacted>)")
    }
}

/// Exact, generation-bound identity for the notification host sink.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NotificationHostSinkIdentity {
    zone: ZoneId,
    provider_ref: ResourceRef,
    host_execution_ref: ResourceRef,
    host_user_ref: ResourceRef,
    display_provider_ref: ResourceRef,
    display_generation: u64,
    controller_generation: u64,
}

impl NotificationHostSinkIdentity {
    /// Construct a validated host-sink lifecycle identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone: ZoneId,
        provider_ref: ResourceRef,
        host_execution_ref: ResourceRef,
        host_user_ref: ResourceRef,
        display_provider_ref: ResourceRef,
        display_generation: u64,
        controller_generation: u64,
    ) -> Result<Self, &'static str> {
        if provider_ref.resource_type().as_str() != "Provider"
            || host_execution_ref.resource_type().as_str() != "Host"
            || host_user_ref.resource_type().as_str() != "User"
            || display_provider_ref.resource_type().as_str() != "Provider"
            || display_generation == 0
            || controller_generation == 0
        {
            return Err("notification-lifecycle-host-sink-invalid");
        }
        Ok(Self {
            zone,
            provider_ref,
            host_execution_ref,
            host_user_ref,
            display_provider_ref,
            display_generation,
            controller_generation,
        })
    }

    /// Borrow the host-sink Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the owning Provider.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }
}

impl core::fmt::Debug for NotificationHostSinkIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NotificationHostSinkIdentity(<redacted>)")
    }
}

/// One complete notification lifecycle transition authorized by the Provider.
#[derive(Clone, PartialEq, Eq)]
pub struct NotificationLifecyclePlan {
    zone: ZoneId,
    provider_ref: ResourceRef,
    start_sources: Vec<NotificationSourceIdentity>,
    stop_sources: Vec<NotificationSourceIdentity>,
    start_host_sink: Option<NotificationHostSinkIdentity>,
    stop_host_sink: Option<NotificationHostSinkIdentity>,
}

impl NotificationLifecyclePlan {
    /// Construct a complete, Zone-local source and sink transition.
    pub fn new(
        zone: ZoneId,
        provider_ref: ResourceRef,
        mut start_sources: Vec<NotificationSourceIdentity>,
        mut stop_sources: Vec<NotificationSourceIdentity>,
        start_host_sink: Option<NotificationHostSinkIdentity>,
        stop_host_sink: Option<NotificationHostSinkIdentity>,
    ) -> Result<Self, &'static str> {
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err("notification-lifecycle-provider-invalid");
        }
        start_sources.sort();
        stop_sources.sort();
        if start_sources
            .iter()
            .any(|source| source.zone() != &zone || source.provider_ref() != &provider_ref)
            || stop_sources
                .iter()
                .any(|source| source.zone() != &zone || source.provider_ref() != &provider_ref)
            || start_sources
                .windows(2)
                .any(|sources| sources[0].source_ref() == sources[1].source_ref())
            || stop_sources
                .windows(2)
                .any(|sources| sources[0].source_ref() == sources[1].source_ref())
            || start_sources.iter().any(|source| {
                stop_sources
                    .iter()
                    .any(|stopped| stopped.source_ref() == source.source_ref() && stopped == source)
            })
            || start_host_sink
                .as_ref()
                .is_some_and(|sink| sink.zone() != &zone || sink.provider_ref() != &provider_ref)
            || stop_host_sink
                .as_ref()
                .is_some_and(|sink| sink.zone() != &zone || sink.provider_ref() != &provider_ref)
        {
            return Err("notification-lifecycle-plan-invalid");
        }
        Ok(Self {
            zone,
            provider_ref,
            start_sources,
            stop_sources,
            start_host_sink,
            stop_host_sink,
        })
    }

    /// Borrow the exact Zone the transition belongs to.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the exact Provider the transition belongs to.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the exact source starts in this transition.
    pub fn start_sources(&self) -> &[NotificationSourceIdentity] {
        &self.start_sources
    }

    /// Borrow the exact source stops in this transition.
    pub fn stop_sources(&self) -> &[NotificationSourceIdentity] {
        &self.stop_sources
    }

    /// Borrow the host-sink start, when present.
    pub const fn start_host_sink(&self) -> Option<&NotificationHostSinkIdentity> {
        self.start_host_sink.as_ref()
    }

    /// Borrow the host-sink stop, when present.
    pub const fn stop_host_sink(&self) -> Option<&NotificationHostSinkIdentity> {
        self.stop_host_sink.as_ref()
    }

    fn digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"d2b-notification-lifecycle-plan-v1");
        digest.update(self.zone.as_str().as_bytes());
        digest.update([0]);
        digest.update(self.provider_ref.to_canonical_string().as_bytes());
        for source in &self.start_sources {
            digest.update([1]);
            digest.update(source.endpoint_digest.as_bytes());
            digest.update(source.source_generation.to_be_bytes());
            digest.update(source.display_generation.to_be_bytes());
        }
        for source in &self.stop_sources {
            digest.update([2]);
            digest.update(source.endpoint_digest.as_bytes());
            digest.update(source.source_generation.to_be_bytes());
            digest.update(source.display_generation.to_be_bytes());
        }
        for sink in [&self.start_host_sink, &self.stop_host_sink] {
            match sink {
                Some(sink) => {
                    digest.update([3]);
                    digest.update(sink.host_execution_ref.to_canonical_string().as_bytes());
                    digest.update([0]);
                    digest.update(sink.host_user_ref.to_canonical_string().as_bytes());
                    digest.update([0]);
                    digest.update(sink.display_provider_ref.to_canonical_string().as_bytes());
                    digest.update(sink.display_generation.to_be_bytes());
                    digest.update(sink.controller_generation.to_be_bytes());
                }
                None => digest.update([4]),
            }
        }
        digest.finalize().into()
    }
}

impl core::fmt::Debug for NotificationLifecyclePlan {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NotificationLifecyclePlan(<redacted>)")
    }
}

/// Trusted observation used to adopt source and host-sink ownership.
#[derive(Clone, Default)]
pub struct NotificationLifecycleObservation {
    sources: Vec<NotificationSourceIdentity>,
    host_sink: Option<NotificationHostSinkIdentity>,
}

impl NotificationLifecycleObservation {
    /// Build one bounded trusted observation.
    pub fn new(
        sources: Vec<NotificationSourceIdentity>,
        host_sink: Option<NotificationHostSinkIdentity>,
    ) -> Self {
        Self { sources, host_sink }
    }
}

/// Host-owned lifecycle operations for notification sources and the sink.
pub trait NotificationLifecycleBackend: Send + Sync + 'static {
    /// Start one generation-bound Guest source.
    fn start_source(&self, source: &NotificationSourceIdentity) -> Result<(), &'static str>;
    /// Stop one exact adopted Guest source.
    fn stop_source(&self, source: &NotificationSourceIdentity) -> Result<(), &'static str>;
    /// Start one exact host sink.
    fn start_host_sink(&self, sink: &NotificationHostSinkIdentity) -> Result<(), &'static str>;
    /// Stop one exact adopted host sink.
    fn stop_host_sink(&self, sink: &NotificationHostSinkIdentity) -> Result<(), &'static str>;
    /// Observe adoptable source and sink ownership after a supervisor restart.
    fn observe(
        &self,
        zone: &ZoneId,
        provider_ref: &ResourceRef,
    ) -> Result<NotificationLifecycleObservation, &'static str>;
}

#[derive(Default)]
struct LifecycleState {
    sources: BTreeMap<ResourceRef, NotificationSourceIdentity>,
    host_sink: Option<NotificationHostSinkIdentity>,
}

/// Opaque supervisor-issued confirmation for a complete lifecycle plan.
pub struct NotificationLifecycleReceipt {
    plan_digest: [u8; 32],
    acknowledgements: usize,
}

impl NotificationLifecycleReceipt {
    /// Confirm this receipt was issued for the exact complete plan.
    pub fn matches(&self, plan: &NotificationLifecyclePlan) -> bool {
        self.plan_digest == plan.digest()
            && self.acknowledgements
                == plan.start_sources.len()
                    + plan.stop_sources.len()
                    + usize::from(plan.start_host_sink.is_some())
                    + usize::from(plan.stop_host_sink.is_some())
    }
}

impl core::fmt::Debug for NotificationLifecycleReceipt {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NotificationLifecycleReceipt(<redacted>)")
    }
}

/// Core-owned lifecycle supervisor that issues receipts only after host effects.
pub struct NotificationLifecycleSupervisor<B: NotificationLifecycleBackend> {
    backend: Arc<B>,
    state: Mutex<LifecycleState>,
}

impl<B: NotificationLifecycleBackend> NotificationLifecycleSupervisor<B> {
    /// Construct one lifecycle supervisor over an authoritative host backend.
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            state: Mutex::new(LifecycleState::default()),
        }
    }

    /// Rehydrate adopted source and sink ownership from the host boundary.
    pub fn recover(
        &self,
        zone: &ZoneId,
        provider_ref: &ResourceRef,
    ) -> Result<usize, &'static str> {
        let observation = self.backend.observe(zone, provider_ref)?;
        let mut sources = BTreeMap::new();
        for source in observation.sources {
            if source.zone() != zone
                || source.provider_ref() != provider_ref
                || sources
                    .insert(source.source_ref().clone(), source)
                    .is_some()
            {
                return Err("notification-lifecycle-adoption-invalid");
            }
        }
        if observation
            .host_sink
            .as_ref()
            .is_some_and(|sink| sink.zone() != zone || sink.provider_ref() != provider_ref)
        {
            return Err("notification-lifecycle-adoption-invalid");
        }
        let count = sources.len() + usize::from(observation.host_sink.is_some());
        let mut state = self
            .state
            .lock()
            .map_err(|_| "notification-lifecycle-state-unavailable")?;
        state.sources = sources;
        state.host_sink = observation.host_sink;
        Ok(count)
    }

    /// Apply one plan and issue a receipt only after every host effect succeeds.
    pub fn apply(
        &self,
        plan: &NotificationLifecyclePlan,
    ) -> Result<NotificationLifecycleReceipt, &'static str> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "notification-lifecycle-state-unavailable")?;
        let mut stopped_sources = Vec::new();
        let mut stopped_host_sink = None;
        let mut started_sources = Vec::new();
        let mut started_host_sink = None;
        let result = (|| {
            for source in &plan.stop_sources {
                if state.sources.get(source.source_ref()) != Some(source) {
                    return Err("notification-lifecycle-source-adoption-mismatch");
                }
                self.backend.stop_source(source)?;
                state.sources.remove(source.source_ref());
                stopped_sources.push(source.clone());
            }
            if let Some(sink) = &plan.stop_host_sink {
                if state.host_sink.as_ref() != Some(sink) {
                    return Err("notification-lifecycle-host-sink-adoption-mismatch");
                }
                self.backend.stop_host_sink(sink)?;
                state.host_sink = None;
                stopped_host_sink = Some(sink.clone());
            }
            for source in &plan.start_sources {
                if state
                    .sources
                    .get(source.source_ref())
                    .is_some_and(|active| active != source)
                {
                    return Err("notification-lifecycle-source-already-active");
                }
                self.backend.start_source(source)?;
                state
                    .sources
                    .insert(source.source_ref().clone(), source.clone());
                started_sources.push(source.clone());
            }
            if let Some(sink) = &plan.start_host_sink {
                if state
                    .host_sink
                    .as_ref()
                    .is_some_and(|active| active != sink)
                {
                    return Err("notification-lifecycle-host-sink-already-active");
                }
                self.backend.start_host_sink(sink)?;
                state.host_sink = Some(sink.clone());
                started_host_sink = Some(sink.clone());
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut compensation_failed = false;
            if let Some(sink) = started_host_sink {
                compensation_failed |= self.backend.stop_host_sink(&sink).is_err();
                if state.host_sink.as_ref() == Some(&sink) {
                    state.host_sink = None;
                }
            }
            for source in started_sources.into_iter().rev() {
                compensation_failed |= self.backend.stop_source(&source).is_err();
                if state.sources.get(source.source_ref()) == Some(&source) {
                    state.sources.remove(source.source_ref());
                }
            }
            if let Some(sink) = stopped_host_sink {
                compensation_failed |= self.backend.start_host_sink(&sink).is_err();
                if !compensation_failed {
                    state.host_sink = Some(sink);
                }
            }
            for source in stopped_sources.into_iter().rev() {
                compensation_failed |= self.backend.start_source(&source).is_err();
                if !compensation_failed {
                    state.sources.insert(source.source_ref().clone(), source);
                }
            }
            if compensation_failed {
                drop(state);
                self.recover(plan.zone(), plan.provider_ref())?;
                return Err("notification-lifecycle-recovery-required");
            }
            return Err(error);
        }
        let acknowledgements = plan.start_sources.len()
            + plan.stop_sources.len()
            + usize::from(plan.start_host_sink.is_some())
            + usize::from(plan.stop_host_sink.is_some());
        Ok(NotificationLifecycleReceipt {
            plan_digest: plan.digest(),
            acknowledgements,
        })
    }

    /// Return whether no source or host-sink ownership remains.
    pub fn is_drained(&self) -> Result<bool, &'static str> {
        let state = self
            .state
            .lock()
            .map_err(|_| "notification-lifecycle-state-unavailable")?;
        Ok(state.sources.is_empty() && state.host_sink.is_none())
    }
}

impl<B: NotificationLifecycleBackend> core::fmt::Debug for NotificationLifecycleSupervisor<B> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("NotificationLifecycleSupervisor(<redacted>)")
    }
}
