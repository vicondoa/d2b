//! Fake core, store, bus, supervisor, manager-endpoint, requeue-scheduler,
//! and effect clients, with fault injection.
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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_provider::v3::{DependencyAlias, ProviderManifest};
use d2b_contracts_resource::v3::ArtifactId;
use d2b_contracts_resource::v3::{ResourceRef, execution_policy::BoundedToken};
use d2b_resource_runtime::context::{
    ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, WatchId, WatchRegistration,
};
use d2b_resource_runtime::error::ResourceError;
use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
use d2b_resource_runtime::manager::ResourceView;
use d2b_resource_runtime::resource::ResourceStatus;
use d2b_resource_runtime::spec_store::EnsureOutcome;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::base::error::ProviderToolkitError;

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
    pub(crate) fn take_next(&mut self) -> Result<(), FakePortError> {
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
        self.faults.take_next()?;
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
        self.faults.take_next()?;
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
        self.faults.take_next()?;
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
        self.faults.take_next()?;
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
        self.faults.take_next()?;
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
        self.faults.take_next()?;
        let _ = self.recorder.record("apply-effect", effect.clone());
        Ok(())
    }

    /// The recorded calls.
    pub const fn recorder(&self) -> &CallRecorder {
        &self.recorder
    }
}

/// A recording [`ManagerEndpoint`] double: committed child rows plus the
/// ordered call log the assertions read.
///
/// `ensure_child` commits the child row (Created/Unchanged/Updated by spec
/// comparison) and records `ensure:<type>/<name>` before the commit and
/// `spawned:<type>/<name>` after it, so a test can pin commit-before-spawn
/// (F1). `delete` records `delete:<type>/<name>` and removes the row;
/// `register_watch` records `watch:<type>/<name>` and returns a
/// monotonically increasing [`WatchId`]. While [`Self::set_fail_reads`] is
/// on, every read (`get`, `view`, `list_owned`) answers `ManagerRpc`, so a
/// test can pin the retryable defer of an unanswerable manager.
///
/// The double is cloneable and shares its state through `Arc`s, so a test
/// can hand [`Self::log_handle`] to a sibling effect double and assert one
/// ordered sequence across both.
#[derive(Clone)]
pub struct RecordingManagerEndpoint {
    zone: String,
    owner_uid: [u8; 16],
    log: Arc<Mutex<Vec<String>>>,
    rows: Arc<Mutex<Vec<StoredDesiredResource>>>,
    views: Arc<Mutex<Vec<(ResourceKey, ResourceView)>>>,
    watch_targets: Arc<Mutex<Vec<ResourceKey>>>,
    next_uid: Arc<AtomicU64>,
    fail_reads: Arc<AtomicBool>,
    fail_ensures: Arc<AtomicBool>,
    fail_deletes: Arc<AtomicBool>,
    children_ready: Arc<AtomicBool>,
}

impl RecordingManagerEndpoint {
    /// A fresh double in zone `work` whose committed children carry the
    /// owner uid `[0x42; 16]`, with its own ordered log.
    pub fn new() -> Self {
        Self {
            zone: "work".to_owned(),
            owner_uid: [0x42; 16],
            log: Arc::new(Mutex::new(Vec::new())),
            rows: Arc::new(Mutex::new(Vec::new())),
            views: Arc::new(Mutex::new(Vec::new())),
            watch_targets: Arc::new(Mutex::new(Vec::new())),
            next_uid: Arc::new(AtomicU64::new(1)),
            fail_reads: Arc::new(AtomicBool::new(false)),
            fail_ensures: Arc::new(AtomicBool::new(false)),
            fail_deletes: Arc::new(AtomicBool::new(false)),
            children_ready: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The zone committed child rows are keyed under.
    pub fn with_zone(mut self, zone: impl Into<String>) -> Self {
        self.zone = zone.into();
        self
    }

    /// The owner uid committed child rows carry.
    pub fn with_owner_uid(mut self, owner_uid: [u8; 16]) -> Self {
        self.owner_uid = owner_uid;
        self
    }

    /// Share the caller's ordered log, so manager calls and a sibling
    /// effect double read as one sequence.
    pub fn with_log(log: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            log,
            ..Self::new()
        }
    }

    /// Seed the parent row the binding declares: `Volume/data` in this
    /// double's zone, owned by nobody.
    pub fn with_parent(self, parent_uid: [u8; 16], spec: &[u8]) -> Self {
        self.rows.lock().push(StoredDesiredResource {
            key: ResourceKey::new(&self.zone, "Volume", "data"),
            uid: parent_uid,
            generation: 2,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: spec.to_vec(),
            metadata: Vec::new(),
            created_at: 0,
        });
        self
    }

    /// Seed one pre-existing row.
    pub fn with_row(self, row: StoredDesiredResource) -> Self {
        self.rows.lock().push(row);
        self
    }

    /// Seed a pre-existing owned-row set.
    pub fn with_rows(self, rows: Vec<StoredDesiredResource>) -> Self {
        self.rows.lock().extend(rows);
        self
    }

    /// Seed one owned child row (drift the driver must retire).
    pub fn seed_owned(&self, key: ResourceKey) {
        self.rows.lock().push(StoredDesiredResource {
            key,
            uid: [0x77; 16],
            generation: 1,
            owner_uid: Some(self.owner_uid),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: Vec::new(),
            metadata: Vec::new(),
            created_at: 0,
        });
    }

    /// Seed one row without a published view.
    pub fn seed(&self, row: StoredDesiredResource) {
        self.rows.lock().push(row);
    }

    /// Seed one row together with its live view at the given status.
    pub fn add(&self, row: StoredDesiredResource, status: ResourceStatus) {
        let view = Self::project(&row, status);
        self.rows.lock().push(row);
        self.views.lock().push((view.key.clone(), view));
    }

    /// Project one committed row to the live view a driver's read sees.
    /// The `deleting` flag follows the row: a seeded row carries its caller's
    /// flag, and an `ensure_child`-committed row is never deleting.
    fn project(row: &StoredDesiredResource, status: ResourceStatus) -> ResourceView {
        ResourceView {
            key: row.key.clone(),
            uid: row.uid,
            generation: row.generation,
            deleting: row.deleting,
            provenance: row.provenance,
            spec: row.spec.clone(),
            metadata: row.metadata.clone(),
            owner_key: None,
            status: Some(status),
            status_generation: Some(row.generation),
            status_projection: None,
        }
    }

    /// Seed one owned row together with its published view.
    pub fn add_owned(&self, row: StoredDesiredResource, view: ResourceView) {
        self.views.lock().push((view.key.clone(), view));
        self.rows.lock().push(row);
    }

    /// Seed one published view without a row.
    pub fn add_view(&self, view: ResourceView) {
        self.views.lock().push((view.key.clone(), view));
    }

    /// Remove one row and its published view.
    pub fn drop_row(&self, key: &ResourceKey) {
        self.rows.lock().retain(|row| row.key != *key);
        self.views.lock().retain(|(view_key, _)| view_key != key);
    }

    /// The published view of one key, if any.
    pub fn view_of(&self, key: &ResourceKey) -> Option<ResourceView> {
        self.views
            .lock()
            .iter()
            .find(|(view_key, _)| view_key == key)
            .map(|(_, view)| view.clone())
    }

    /// Make every read answer `ManagerRpc` (the unanswerable plane).
    pub fn set_fail_reads(&self, fail: bool) {
        self.fail_reads.store(fail, Ordering::SeqCst);
    }

    /// Make every child ensure answer `ManagerRpc` (the refusing plane).
    pub fn set_fail_ensures(&self, fail: bool) {
        self.fail_ensures.store(fail, Ordering::SeqCst);
    }

    /// Make every child delete answer `ManagerRpc` (the refusing plane).
    pub fn set_fail_deletes(&self, fail: bool) {
        self.fail_deletes.store(fail, Ordering::SeqCst);
    }

    /// The phase ensured children publish: Ready when on, Pending when off.
    pub fn set_children_ready(&self, ready: bool) {
        self.children_ready.store(ready, Ordering::SeqCst);
    }

    /// The shared ordered log, for a sibling double to append to.
    pub fn log_handle(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.log)
    }

    /// The recorded calls in order.
    pub fn call_order(&self) -> Vec<String> {
        self.log.lock().clone()
    }

    /// The recorded `ensure:` calls in order.
    pub fn ensure_order(&self) -> Vec<String> {
        self.log
            .lock()
            .iter()
            .filter(|call| call.starts_with("ensure:"))
            .cloned()
            .collect()
    }

    /// The committed rows.
    pub fn rows(&self) -> Vec<StoredDesiredResource> {
        self.rows.lock().clone()
    }

    /// One committed row by key.
    pub fn row(&self, key: &ResourceKey) -> Option<StoredDesiredResource> {
        self.rows.lock().iter().find(|row| row.key == *key).cloned()
    }

    /// The registered watch targets in order.
    pub fn watch_targets(&self) -> Vec<ResourceKey> {
        self.watch_targets.lock().clone()
    }
}

impl Default for RecordingManagerEndpoint {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ManagerEndpoint for RecordingManagerEndpoint {
    async fn ensure_child(
        &self,
        _parent: &ResourceKey,
        child: ChildEnsure,
    ) -> Result<EnsureOutcome, ResourceError> {
        let id = format!("{}/{}", child.type_name.as_str(), child.name);
        self.log.lock().push(format!("ensure:{id}")); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        if self.fail_ensures.load(Ordering::SeqCst) {
            return Err(ResourceError::ManagerRpc("scripted ensure failure".into()));
        }
        let next = self.next_uid.fetch_add(1, Ordering::SeqCst);
        let mut uid = [0u8; 16];
        uid[..8].copy_from_slice(&next.to_be_bytes());
        let row = StoredDesiredResource {
            key: ResourceKey::new(&self.zone, child.type_name.as_str(), &child.name),
            uid,
            generation: 1,
            owner_uid: Some(self.owner_uid),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: child.spec,
            metadata: child.metadata,
            created_at: 0,
        };
        let mut rows = self.rows.lock(); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        let outcome = match rows.iter_mut().find(|existing| existing.key == row.key) {
            Some(existing) if existing.spec == row.spec => EnsureOutcome::Unchanged(existing.clone()),
            Some(existing) => {
                *existing = row.clone();
                EnsureOutcome::Updated(row.clone())
            }
            None => {
                rows.push(row.clone());
                EnsureOutcome::Created(row.clone())
            }
        };
        drop(rows);
        // Publish the live view the child's phase reads; the children_ready
        // switch is what a test flips to converge the child gate.
        if !matches!(outcome, EnsureOutcome::Unchanged(_)) {
            let status = if self.children_ready.load(Ordering::SeqCst) {
                ResourceStatus::Ready
            } else {
                ResourceStatus::Pending
            };
            self.views.lock().retain(|(view_key, _)| view_key != &row.key); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            self.views.lock().push(( // async-gate-allow: synchronous lock acquisition, no await while the guard is held
                row.key.clone(),
                Self::project(&row, status),
            ));
        }
        // Spawn notification only after the commit (F1, AE1).
        self.log.lock().push(format!("spawned:{id}")); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        Ok(outcome)
    }

    async fn get(
        &self,
        key: &ResourceKey,
    ) -> Result<Option<StoredDesiredResource>, ResourceError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(ResourceError::ManagerRpc("scripted read failure".into()));
        }
        Ok(self.rows.lock().iter().find(|row| row.key == *key).cloned()) // async-gate-allow: synchronous lock acquisition, no await while the guard is held
    }

    async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
        self.log.lock().push(format!("view:{}/{}", key.type_name, key.name)); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(ResourceError::ManagerRpc("scripted read failure".into()));
        }
        Ok(self.view_of(key))
    }

    async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
        self.log.lock().push(format!("delete:{}/{}", key.type_name, key.name)); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        if self.fail_deletes.load(Ordering::SeqCst) {

            return Err(ResourceError::ManagerRpc("scripted delete failure".into()));
        }
        self.rows.lock().retain(|row| row.key != *key); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        self.views.lock().retain(|(view_key, _)| view_key != key); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        Ok(())
    }

    async fn list_owned(
        &self,
        owner_uid: [u8; 16],
    ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(ResourceError::ManagerRpc("scripted read failure".into()));
        }
        Ok(self
            .rows
            .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            .iter()
            .filter(|row| row.owner_uid == Some(owner_uid))
            .cloned()
            .collect())
    }

    async fn register_watch(
        &self,
        _subscriber: &ResourceKey,
        registration: WatchRegistration,
    ) -> Result<WatchId, ResourceError> {
        self.log.lock().push(format!( // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            "watch:{}/{}",
            registration.target.type_name, registration.target.name
        ));
        let mut targets = self.watch_targets.lock(); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        targets.push(registration.target.clone());
        Ok(WatchId(targets.len() as u64))
    }

    async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
        Ok(())
    }
}

/// A recording [`RequeueScheduler`] double: every schedule is recorded with
/// its key and exact delay, and each schedule returns a monotonically
/// increasing [`RequeueId`].
///
/// [`Self::new`] additionally returns the delivery receiver: each scheduled
/// id is sent there after the backoff elapses (over tokio paused time), so a
/// test can pin the timer firing without touching the scheduler trait's
/// synchronous seam. [`Self::default`] records without delivering.
///
/// The double is cloneable and shares its state through an `Arc`, so a test
/// can hand one clone to the driver and keep another to read the log.
#[derive(Clone)]
pub struct RecordingRequeue {
    inner: Arc<Mutex<RecordingRequeueInner>>,
}

/// The lock-guarded state behind [`RecordingRequeue`].
struct RecordingRequeueInner {
    calls: Vec<(ResourceKey, Duration)>,
    next: u64,
    delivered_tx: Option<mpsc::UnboundedSender<u64>>,
}

impl Default for RecordingRequeue {
    /// A plain recorder: schedules are recorded, ids are never delivered.
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RecordingRequeueInner {
                calls: Vec::new(),
                next: 1,
                delivered_tx: None,
            })),
        }
    }
}

impl RecordingRequeue {
    /// Create the recorder plus the receiver its scheduled ids are delivered
    /// on, one id per schedule, after each backoff elapses.
    pub fn new() -> (Self, mpsc::UnboundedReceiver<u64>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Self {
                inner: Arc::new(Mutex::new(RecordingRequeueInner {
                    calls: Vec::new(),
                    next: 1,
                    delivered_tx: Some(tx),
                })),
            },
            rx,
        )
    }

    /// The scheduled delays, in arrival order.
    pub fn scheduled(&self) -> Vec<Duration> {
        self.inner
            .lock()
            .calls
            .iter()
            .map(|(_, after)| *after)
            .collect()
    }
}

impl RequeueScheduler for RecordingRequeue {
    fn schedule(&self, key: ResourceKey, after: Duration) -> RequeueId {
        let mut inner = self.inner.lock();
        let id = inner.next;
        inner.next += 1;
        inner.calls.push((key, after));
        if let Some(tx) = inner.delivered_tx.clone() {
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                let _ = tx.send(id);
            });
        }
        RequeueId(id)
    }

    fn cancel(&self, _id: RequeueId) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::conformance::check_closed_code_set;

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
        assert_eq!(plan.take_next(), Err(FakePortError::InjectedFault));
        assert_eq!(plan.take_next(), Ok(()));
        assert_eq!(plan.take_next(), Err(FakePortError::InjectedFault));
        // An exhausted plan stops injecting rather than repeating forever.
        assert_eq!(plan.take_next(), Ok(()));
        assert_eq!(plan.consumed(), 4);
        assert_eq!(FaultPlan::healthy().consumed(), 0);
        assert_eq!(
            FaultPlan::failing_first(1).take_next(),
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

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_manager_endpoint_commits_before_spawn_and_retires_on_delete() {
        let manager = RecordingManagerEndpoint::new();
        let child = ChildEnsure {
            type_name: d2b_resource_runtime::identity::ResourceTypeName::new("Process"),
            name: "worker-0".to_owned(),
            spec: b"spec".to_vec(),
            metadata: Vec::new(),
        };
        let parent = ResourceKey::new("work", "Volume", "data");
        let created = manager
            .ensure_child(&parent, child.clone())
            .await
            .expect("ensure");
        assert!(matches!(created, EnsureOutcome::Created(_)));
        let unchanged = manager
            .ensure_child(&parent, child.clone())
            .await
            .expect("re-ensure");
        assert!(matches!(unchanged, EnsureOutcome::Unchanged(_)));
        let order = manager.call_order();
        let ensure_at = order.iter().position(|entry| entry == "ensure:Process/worker-0");
        let spawned_at = order.iter().position(|entry| entry == "spawned:Process/worker-0");
        assert!(ensure_at < spawned_at, "commit-before-spawn (F1): {order:?}");
        assert_eq!(manager.rows().len(), 1, "one committed row, no duplicate");

        let key = ResourceKey::new("work", "Process", "worker-0");
        manager.delete(&key).await.expect("delete");
        assert!(manager.row(&key).is_none(), "delete removes the committed row");
        assert_eq!(
            manager.call_order(),
            vec![
                "ensure:Process/worker-0".to_owned(),
                "spawned:Process/worker-0".to_owned(),
                "ensure:Process/worker-0".to_owned(),
                "spawned:Process/worker-0".to_owned(),
                "delete:Process/worker-0".to_owned(),
            ]
        );
    }
}
