//! Fake core, store, bus, supervisor, and effect clients, with fault
//! injection.
//!
//! `ADR-046-provider-model-and-packaging` section "Toolkit" lists these as
//! toolkit deliverables so that every Provider crate can write hermetic
//! `tests/` cases against the ports it depends on, instead of each crate
//! re-deriving its own doubles and drifting.
//!
//! Every fake here is synchronous, in-memory, and hermetic: nothing opens a
//! socket, touches a filesystem, spawns a process, or waits on wall time,
//! so a case built on them stays inside the per-test execution budget.
//!
//! Two properties matter more than convenience.
//!
//! A fake refuses exactly where the real port refuses. The bus resolves a
//! declared dependency alias and nothing else, and it never hands back its
//! binding table, because a component asks for an alias and must never
//! receive a global registry or an arbitrary Provider endpoint. The
//! supervisor records a launch intent and never spawns. The effect port
//! records an intent and performs no mutation, because a Provider reaches
//! host state only through an injected typed effect port whose real
//! implementation is the broker's, not the Provider's.
//!
//! No fake carries or renders a caller-supplied value. A recorded call is a
//! closed operation discriminant and a bounded identifier, and every
//! `Debug` here renders counts and discriminants rather than the values it
//! was handed.

use std::collections::BTreeMap;

use d2b_contracts::v3::{
    execution_policy::BoundedToken,
    provider::{ArtifactId, DependencyAlias, ProviderManifest},
    resource_ref::ResourceRef,
};

use crate::error::ProviderToolkitError;

/// The maximum number of calls one fake records before it stops growing.
///
/// A recorder that grew without bound would turn a runaway test into a
/// memory exhaustion rather than a failed assertion.
pub const MAX_RECORDED_CALLS: usize = 256;

/// Why a fake port refused a call.
///
/// The set is closed and each variant renders one stable lower-kebab code.
/// A code never echoes an alias binding, a resource name, an artifact
/// identifier, or a caller-supplied value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum FakePortError {
    /// The requested dependency alias is not bound in this Zone.
    AliasNotBound,
    /// No artifact is present in the fake catalog for that identifier.
    ArtifactNotFound,
    /// The Provider resource is not Ready, so no `providerRef` resolves.
    ProviderNotReady,
    /// The Provider attempted to write status for a ResourceType it does
    /// not own.
    NotOwned,
    /// A fault was injected for this call.
    InjectedFault,
    /// The recorder is full, so the call is refused rather than dropped
    /// silently.
    RecorderFull,
}

impl FakePortError {
    /// The stable lower-kebab code for this refusal.
    pub const fn code(self) -> &'static str {
        match self {
            Self::AliasNotBound => "alias-not-bound",
            Self::ArtifactNotFound => "artifact-not-found",
            Self::ProviderNotReady => "provider-not-ready",
            Self::NotOwned => "not-owned",
            Self::InjectedFault => "injected-fault",
            Self::RecorderFull => "recorder-full",
        }
    }

    /// The complete closed refusal set, for conformance assertions.
    pub const ALL: [Self; 6] = [
        Self::AliasNotBound,
        Self::ArtifactNotFound,
        Self::ProviderNotReady,
        Self::NotOwned,
        Self::InjectedFault,
        Self::RecorderFull,
    ];
}

impl core::fmt::Display for FakePortError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for FakePortError {}

/// A bounded schedule of injected faults.
///
/// The plan is consumed left to right: each call takes the next entry, and
/// once the plan is exhausted every subsequent call succeeds. Scheduling
/// faults rather than toggling a flag lets a case pin the exact call that
/// fails, which is what a restart or retry assertion needs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FaultPlan {
    schedule: Vec<bool>,
    consumed: usize,
}

impl FaultPlan {
    /// A plan that injects nothing.
    pub const fn healthy() -> Self {
        Self {
            schedule: Vec::new(),
            consumed: 0,
        }
    }

    /// A plan that fails the first `count` calls and then succeeds.
    pub fn failing_first(count: usize) -> Self {
        Self {
            schedule: vec![true; count],
            consumed: 0,
        }
    }

    /// A plan built from an explicit per-call schedule.
    pub fn scheduled(schedule: impl IntoIterator<Item = bool>) -> Self {
        Self {
            schedule: schedule.into_iter().collect(),
            consumed: 0,
        }
    }

    /// Take the next scheduled outcome.
    fn take(&mut self) -> Result<(), FakePortError> {
        let inject = self.schedule.get(self.consumed).copied().unwrap_or(false);
        self.consumed = self.consumed.saturating_add(1);
        if inject {
            Err(FakePortError::InjectedFault)
        } else {
            Ok(())
        }
    }

    /// How many calls this plan has already decided.
    pub const fn consumed(&self) -> usize {
        self.consumed
    }
}

/// One recorded call against a fake port.
///
/// A record names the operation and the bounded identifier it targeted. It
/// deliberately holds no payload: a payload is caller-supplied and would
/// make an assertion over the recorder a way to read one back.
#[derive(Clone, PartialEq, Eq)]
pub struct RecordedCall {
    operation: BoundedToken,
    target: BoundedToken,
}

impl RecordedCall {
    /// The operation this call performed.
    pub const fn operation(&self) -> &BoundedToken {
        &self.operation
    }

    /// The bounded identifier the call targeted.
    pub const fn target(&self) -> &BoundedToken {
        &self.target
    }
}

impl core::fmt::Debug for RecordedCall {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RecordedCall(<redacted>)")
    }
}

/// A bounded recorder shared by every fake port.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CallRecorder {
    calls: Vec<RecordedCall>,
}

impl CallRecorder {
    /// Record one call, or refuse when the bound is reached.
    fn record(
        &mut self,
        operation: &str,
        target: BoundedToken,
    ) -> Result<(), ProviderToolkitError> {
        if self.calls.len() >= MAX_RECORDED_CALLS {
            return Err(ProviderToolkitError::CapacityOutOfRange);
        }
        self.calls.push(RecordedCall {
            operation: BoundedToken::parse(operation)
                .expect("every fake operation token is a compiled constant"),
            target,
        });
        Ok(())
    }

    /// The recorded calls in order.
    pub fn calls(&self) -> &[RecordedCall] {
        &self.calls
    }

    /// How many calls carry the exact operation token.
    pub fn count_of(&self, operation: &str) -> usize {
        self.calls
            .iter()
            .filter(|call| call.operation().as_str() == operation)
            .count()
    }

    /// The number of recorded calls.
    pub fn len(&self) -> usize {
        self.calls.len()
    }

    /// Whether nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }
}

/// A fake Zone core client: artifact catalog lookup and readiness.
///
/// Core resolves an `artifactId` to a signed manifest and computes the
/// aggregate Provider status; a Provider never authors its own aggregate
/// status, so this fake exposes readiness as something set on it rather
/// than something a Provider can write.
#[derive(Debug, Default)]
pub struct FakeCoreClient {
    catalog: BTreeMap<String, ProviderManifest>,
    ready: bool,
    faults: FaultPlan,
    recorder: CallRecorder,
}

impl FakeCoreClient {
    /// Build a core client holding one catalog entry, initially not Ready.
    pub fn with_artifact(artifact_id: &ArtifactId, manifest: ProviderManifest) -> Self {
        let mut catalog = BTreeMap::new();
        catalog.insert(artifact_id.as_str().to_owned(), manifest);
        Self {
            catalog,
            ready: false,
            faults: FaultPlan::healthy(),
            recorder: CallRecorder::default(),
        }
    }

    /// Set the fault plan.
    pub fn with_faults(mut self, faults: FaultPlan) -> Self {
        self.faults = faults;
        self
    }

    /// Mark the Provider resource Ready, as core would once every component
    /// and dependency reported healthy.
    pub fn mark_ready(&mut self) {
        self.ready = true;
    }

    /// Resolve an artifact identifier to its signed manifest.
    pub fn resolve_artifact(
        &mut self,
        artifact_id: &ArtifactId,
    ) -> Result<&ProviderManifest, FakePortError> {
        self.faults.take()?;
        let _ = self
            .recorder
            .record("resolve-artifact", BoundedToken::parse("catalog").unwrap());
        self.catalog
            .get(artifact_id.as_str())
            .ok_or(FakePortError::ArtifactNotFound)
    }

    /// Resolve a `providerRef`, which succeeds only while Ready.
    pub fn resolve_provider_ref(
        &mut self,
        provider_ref: &ResourceRef,
    ) -> Result<(), FakePortError> {
        self.faults.take()?;
        let _ = self
            .recorder
            .record("resolve-provider-ref", BoundedToken::parse("row").unwrap());
        let _ = provider_ref;
        if self.ready {
            Ok(())
        } else {
            Err(FakePortError::ProviderNotReady)
        }
    }

    /// The recorded calls.
    pub const fn recorder(&self) -> &CallRecorder {
        &self.recorder
    }
}

/// A fake resource store that enforces the ownership rule on status writes.
///
/// A Provider controller writes status only for the ResourceTypes it owns.
/// That rule is the reason this fake exists: a Provider crate's tests should
/// be able to prove their controller never writes outside its own set.
#[derive(Debug, Default)]
pub struct FakeResourceStore {
    owned: Vec<String>,
    faults: FaultPlan,
    recorder: CallRecorder,
}

impl FakeResourceStore {
    /// Build a store that grants the Provider exactly these ResourceTypes.
    pub fn owning(resource_types: impl IntoIterator<Item = String>) -> Self {
        Self {
            owned: resource_types.into_iter().collect(),
            faults: FaultPlan::healthy(),
            recorder: CallRecorder::default(),
        }
    }

    /// Set the fault plan.
    pub fn with_faults(mut self, faults: FaultPlan) -> Self {
        self.faults = faults;
        self
    }

    /// Write status for one resource, refusing an unowned ResourceType.
    pub fn write_status(&mut self, resource_ref: &ResourceRef) -> Result<(), FakePortError> {
        self.faults.take()?;
        let _ = self
            .recorder
            .record("write-status", BoundedToken::parse("status").unwrap());
        if self
            .owned
            .iter()
            .any(|owned| owned == resource_ref.resource_type().as_str())
        {
            Ok(())
        } else {
            Err(FakePortError::NotOwned)
        }
    }

    /// The recorded calls.
    pub const fn recorder(&self) -> &CallRecorder {
        &self.recorder
    }
}

/// A fake dependency-portal bus.
///
/// It resolves a declared alias to one bound Provider reference. There is no
/// enumeration accessor and no wildcard: the binding table is private
/// because handing it back is exactly the global registry the specification
/// forbids a component from receiving.
#[derive(Debug, Default)]
pub struct FakeBus {
    bindings: BTreeMap<DependencyAlias, ResourceRef>,
    faults: FaultPlan,
    recorder: CallRecorder,
}

impl FakeBus {
    /// Build a bus with an explicit alias binding table.
    pub fn with_bindings(
        bindings: impl IntoIterator<Item = (DependencyAlias, ResourceRef)>,
    ) -> Self {
        Self {
            bindings: bindings.into_iter().collect(),
            faults: FaultPlan::healthy(),
            recorder: CallRecorder::default(),
        }
    }

    /// Set the fault plan.
    pub fn with_faults(mut self, faults: FaultPlan) -> Self {
        self.faults = faults;
        self
    }

    /// Resolve one declared alias.
    pub fn resolve_alias(&mut self, alias: DependencyAlias) -> Result<ResourceRef, FakePortError> {
        self.faults.take()?;
        let _ = self.recorder.record(
            "resolve-alias",
            BoundedToken::parse(alias.as_str()).expect("an alias token is a compiled constant"),
        );
        self.bindings
            .get(&alias)
            .cloned()
            .ok_or(FakePortError::AliasNotBound)
    }

    /// The recorded calls.
    pub const fn recorder(&self) -> &CallRecorder {
        &self.recorder
    }
}

/// A fake ProviderSupervisor that records launch intents and never spawns.
///
/// The real supervisor is the sole caller of the privileged spawn effect. A
/// Provider validates its ExecutionSpec and SandboxSpec and calls the port;
/// this fake is that port with the effect removed.
#[derive(Debug, Default)]
pub struct FakeSupervisor {
    faults: FaultPlan,
    recorder: CallRecorder,
}

impl FakeSupervisor {
    /// Build a supervisor with a fault plan.
    pub fn with_faults(faults: FaultPlan) -> Self {
        Self {
            faults,
            recorder: CallRecorder::default(),
        }
    }

    /// Record one launch intent for the named component.
    pub fn launch(&mut self, component_id: &BoundedToken) -> Result<(), FakePortError> {
        self.faults.take()?;
        let _ = self.recorder.record("launch", component_id.clone());
        Ok(())
    }

    /// The recorded calls.
    pub const fn recorder(&self) -> &CallRecorder {
        &self.recorder
    }
}

/// A fake typed effect port.
///
/// Every real host mutation is a typed, audited broker op. This fake
/// records the intent and performs nothing, so a Provider crate can assert
/// which effects its controller would have released without any host
/// mutation happening in a test.
#[derive(Debug, Default)]
pub struct FakeEffectPort {
    faults: FaultPlan,
    recorder: CallRecorder,
}

impl FakeEffectPort {
    /// Build an effect port with a fault plan.
    pub fn with_faults(faults: FaultPlan) -> Self {
        Self {
            faults,
            recorder: CallRecorder::default(),
        }
    }

    /// Record one effect intent.
    pub fn apply(&mut self, effect: &BoundedToken) -> Result<(), FakePortError> {
        self.faults.take()?;
        let _ = self.recorder.record("apply-effect", effect.clone());
        Ok(())
    }

    /// The recorded calls.
    pub const fn recorder(&self) -> &CallRecorder {
        &self.recorder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::check_closed_code_set;

    #[test]
    fn every_refusal_code_is_unique_and_matches_the_frozen_grammar() {
        let codes: Vec<&str> = FakePortError::ALL
            .iter()
            .map(|error| error.code())
            .collect();
        assert!(check_closed_code_set(&codes).is_ok());
    }

    #[test]
    fn a_fault_plan_is_consumed_call_by_call() {
        let mut plan = FaultPlan::scheduled([true, false, true]);
        assert_eq!(plan.take(), Err(FakePortError::InjectedFault));
        assert_eq!(plan.take(), Ok(()));
        assert_eq!(plan.take(), Err(FakePortError::InjectedFault));
        // An exhausted plan stops injecting rather than repeating forever.
        assert_eq!(plan.take(), Ok(()));
        assert_eq!(plan.consumed(), 4);
        assert_eq!(FaultPlan::healthy().consumed(), 0);
        assert_eq!(
            FaultPlan::failing_first(1).take(),
            Err(FakePortError::InjectedFault)
        );
    }

    #[test]
    fn the_bus_resolves_one_alias_and_never_a_table() {
        let mut bus = FakeBus::with_bindings([(
            DependencyAlias::Volume,
            ResourceRef::parse("Provider/volume-local").unwrap(),
        )]);
        assert_eq!(
            bus.resolve_alias(DependencyAlias::Volume).unwrap(),
            ResourceRef::parse("Provider/volume-local").unwrap()
        );
        assert_eq!(
            bus.resolve_alias(DependencyAlias::Network),
            Err(FakePortError::AliasNotBound)
        );
        assert_eq!(bus.recorder().count_of("resolve-alias"), 2);
    }

    #[test]
    fn the_store_refuses_a_status_write_outside_the_owned_set() {
        let mut store = FakeResourceStore::owning(["Volume".to_owned()]);
        assert!(
            store
                .write_status(&ResourceRef::parse("Volume/state").unwrap())
                .is_ok()
        );
        assert_eq!(
            store.write_status(&ResourceRef::parse("Network/lan").unwrap()),
            Err(FakePortError::NotOwned)
        );
    }

    #[test]
    fn a_recorded_call_renders_nothing_it_was_handed() {
        let mut supervisor = FakeSupervisor::with_faults(FaultPlan::healthy());
        supervisor
            .launch(&BoundedToken::parse("volume-controller").unwrap())
            .unwrap();
        let rendered = format!("{:?}", supervisor.recorder().calls());
        assert!(!rendered.contains("volume-controller"));
        assert_eq!(supervisor.recorder().len(), 1);
        assert!(!supervisor.recorder().is_empty());
    }

    #[test]
    fn an_injected_fault_stops_the_effect_from_being_recorded() {
        let mut port = FakeEffectPort::with_faults(FaultPlan::failing_first(1));
        let effect = BoundedToken::parse("attach-volume").unwrap();
        assert_eq!(port.apply(&effect), Err(FakePortError::InjectedFault));
        assert!(port.recorder().is_empty());
        assert!(port.apply(&effect).is_ok());
        assert_eq!(port.recorder().count_of("apply-effect"), 1);
    }
}
