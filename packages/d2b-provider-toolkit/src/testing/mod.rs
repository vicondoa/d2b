//! The Provider test harness.
//!
//! The harness runs the provider's real base - the real declarations fence,
//! the real child-creation fence, the real operation envelope, the real
//! audit ring, and a deterministic clock - and replaces only the effect
//! ports with scripted fakes. A test therefore exercises the fences instead
//! of mocking them: an undeclared child creation is refused by the same code
//! that refuses it in production, and an ungranted operation invocation is
//! refused by the same envelope.
//!
//! Five kinds are drivable, one per standard test file:
//!
//! | Kind | Entry point | Asserts |
//! |------|-------------|---------|
//! | Reconcile | [`TestHarness::reconcile`] | status transitions, conditions, requeue class, created children |
//! | Operations | [`TestHarness::call_operation`] | result, audit records, deny-by-default |
//! | Creation fence | [`TestHarness::create_child`], [`TestHarness::expect_created`] | undeclared refused, declared realized |
//! | Fault | [`TestHarness::script_faults`], [`TestHarness::faults`] | scripted failures, retry classes |
//! | Conformance | [`conformance`] | the black-box contract, as a normal dependency |
//!
//! Every operation runs through [`crate::operations::OperationEnvelope`] and
//! every spec passes the admission fence. What the harness cannot run is
//! what it does not own: the payload schema of a committed `Operation` row
//! and the family's own decoder are validated by the broker envelope and the
//! resource runtime, neither of which is a toolkit dependency. The fence
//! here reports that boundary instead of pretending to cross it.

pub mod conformance;
pub mod fakes;
pub mod fixture;

pub use fakes::{
    FakeBus, FakeCoreClient, FakeEffectPort, FakePortError, FakeResourceStore, FakeSupervisor,
    FaultPlan, MAX_RECORDED_CALLS, RecordingManagerEndpoint, RecordingRequeue,
};
pub use fixture::{
    DeterministicClock, FIXTURE_NOW_UNIX_MS, FakeProvider, Fixture, SampleLeaseRequest,
    sample_lease_request,
};

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use tokio::sync::Mutex as AsyncMutex;

use async_trait::async_trait;
use d2b_contracts_resource::v3::resource_schema::{CanonicalJsonObject, CanonicalJsonValue};
use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use d2b_resource_types::{
    ChildCreation, DriverDescriptor, OperationDef, ProviderDeclaration, ServiceDecl, StartupStep,
    WellKnownType,
};

use crate::audit::ProviderAgentAuditLog;
use crate::base::{
    AttachError, DrainError, Lifecycle, ProviderBase, StartupError, StartupPlan,
    StartupStepExecutor,
};
use crate::operations::{OperationEnvelope, OperationFailure, OperationResult};
use crate::plane::{
    ChildCreationFailure, Clock, CreationRefusal, CreationTable, DrainDeadline, PlaneError,
    SharedClock, ZonePlaneHandle, ZonePlanePort,
};
use crate::plane::{
    CreateChild, ReconcileCause, ReconcileCtx, ReconcileOutcome, ReconcileRefusal, ReconcileTarget,
};

/// Drive a future to completion on the calling thread.
///
/// Every Provider effect port in this workspace is an async seam whose test
/// double is immediately ready, so a conformance suite needs a driver but
/// not an async runtime. Each Provider crate previously carried a private
/// copy of this function; this is that driver, once.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::hint::spin_loop(),
        }
    }
}

impl Clock for DeterministicClock {
    fn now_unix_ms(&self) -> u64 {
        DeterministicClock::now_unix_ms(self)
    }
}

/// The phase the harness publishes for one admitted row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowPhase {
    /// Admitted, never reconciled.
    Pending,
    /// The last reconcile pass was satisfied.
    Ready,
    /// The last reconcile pass asked for another one.
    NotYet,
    /// The last reconcile pass failed terminally.
    Failed,
}

impl RowPhase {
    /// The stable lower-kebab label for this phase.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::NotYet => "not-yet",
            Self::Failed => "failed",
        }
    }
}

/// The status one admitted row carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowStatus {
    phase: RowPhase,
    /// The closed reason of the last pass, when it had one.
    pub reason: Option<&'static str>,
    /// The bounded requeue delay of the last pass, when it asked for one.
    pub requeue_after_ms: Option<u64>,
    /// The cause the last pass carried.
    pub cause: Option<ReconcileCause>,
}

impl RowStatus {
    /// The published phase.
    pub const fn phase(&self) -> RowPhase {
        self.phase
    }

    /// Whether the row is ready.
    pub const fn is_ready(&self) -> bool {
        matches!(self.phase, RowPhase::Ready)
    }
}

/// One committed spec row in the harness store.
pub struct AdmittedRow {
    resource_type: WellKnownType,
    name: String,
    spec: CanonicalJsonObject,
    status: AsyncMutex<RowStatus>,
}

impl std::fmt::Debug for AdmittedRow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdmittedRow")
            .field("resource_type", &self.resource_type)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl AdmittedRow {
    /// The resource type this row commits.
    pub const fn resource_type(&self) -> WellKnownType {
        self.resource_type
    }

    /// The resource name this row commits.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The committed canonical spec.
    pub const fn spec(&self) -> &CanonicalJsonObject {
        &self.spec
    }

    /// The `Type/name` reference of this row.
    pub fn resource_ref(&self) -> ResourceRef {
        ResourceRef::parse(&self.reference_key())
            .expect("an admitted row's reference is a valid resource reference")
    }

    /// The `Type/name` key this row is stored under.
    pub fn reference_key(&self) -> String {
        format!(
            "{}/{}",
            self.resource_type.to_resource_type_name().as_str(),
            self.name
        )
    }

    /// Snapshot the published status.
    pub fn status(&self) -> RowStatus {
        self.status
            .try_lock()
            .map(|status| status.clone())
            .unwrap_or(RowStatus {
                phase: RowPhase::Pending,
                reason: None,
                requeue_after_ms: None,
                cause: None,
            })
    }

    fn publish(&self, cause: &ReconcileCause, outcome: &ReconcileOutcome) {
        if let Ok(mut status) = self.status.try_lock() {
            status.phase = match outcome {
                ReconcileOutcome::Ready => RowPhase::Ready,
                ReconcileOutcome::NotYet { .. } => RowPhase::NotYet,
                ReconcileOutcome::Failed { .. } => RowPhase::Failed,
            };
            status.reason = match outcome {
                ReconcileOutcome::Failed { code } => Some(code),
                ReconcileOutcome::Ready | ReconcileOutcome::NotYet { .. } => None,
            };
            status.requeue_after_ms = match outcome {
                ReconcileOutcome::NotYet { .. } => Some(outcome.requeue_after_ms()),
                ReconcileOutcome::Ready | ReconcileOutcome::Failed { .. } => None,
            };
            status.cause = Some(cause.clone());
        }
    }
}

/// Why the admission fence refused a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionRefusal {
    /// The resource type has no declared driver, so the harness cannot admit
    /// a row of it.
    UndeclaredType {
        /// The undeclared type.
        resource_type: WellKnownType,
    },
    /// A row with this type and name is already committed.
    DuplicateRow {
        /// The `Type/name` reference.
        reference: String,
    },
    /// A `<semantic>Ref` field names a row the store does not hold.
    UnresolvedRef {
        /// The dotted field path that carries the reference.
        field: String,
        /// The unresolved reference.
        reference: String,
    },
}

impl AdmissionRefusal {
    /// The stable lower-kebab code for this refusal.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UndeclaredType { .. } => "undeclared-type",
            Self::DuplicateRow { .. } => "duplicate-row",
            Self::UnresolvedRef { .. } => "unresolved-ref",
        }
    }
}

impl core::fmt::Display for AdmissionRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AdmissionRefusal {}

/// One call the harness's plane port recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaneCall {
    /// A declared storage root was claimed.
    StorageRoot(String),
    /// A declared plane adapter was deployed.
    Adapter(String),
    /// A declared service was published.
    Service(String),
}

/// The recording zone-plane port a harness attaches through.
#[derive(Debug, Default)]
pub struct RecordingPlanePort {
    calls: AsyncMutex<Vec<PlaneCall>>,
    refuse: AtomicBool,
}

impl RecordingPlanePort {
    /// Every call this port accepted, in order.
    pub fn calls(&self) -> Vec<PlaneCall> {
        self.calls
            .try_lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }

    /// Refuse every further call.
    pub fn refuse(&self) {
        self.refuse.store(true, Ordering::Release);
    }

    fn record(&self, call: PlaneCall) -> Result<(), PlaneError> {
        if self.refuse.load(Ordering::Acquire) {
            return Err(PlaneError::Refused);
        }
        self.calls
            .try_lock()
            .map(|mut calls| calls.push(call))
            .map_err(|_| PlaneError::Refused)
    }
}

#[async_trait]
impl ZonePlanePort for RecordingPlanePort {
    async fn claim_storage_root(
        &self,
        _provider_ref: &'static str,
        root: &d2b_resource_types::StorageRoot,
    ) -> Result<(), PlaneError> {
        self.record(PlaneCall::StorageRoot(root.path.to_owned()))
    }

    async fn deploy_adapter(
        &self,
        _provider_ref: &'static str,
        adapter: &d2b_resource_types::PlaneAdapter,
    ) -> Result<(), PlaneError> {
        self.record(PlaneCall::Adapter(adapter.id.to_owned()))
    }

    async fn publish_service(
        &self,
        _provider_ref: &'static str,
        service: &d2b_resource_types::ServiceDecl,
    ) -> Result<(), PlaneError> {
        self.record(PlaneCall::Service(service.id.to_owned()))
    }
}

/// A cloneable handle onto one harness fault plan.
#[derive(Clone, Default)]
pub struct FaultInjector {
    plan: Arc<AsyncMutex<FaultPlan>>,
}

impl FaultInjector {
    /// Take the next scheduled outcome for one effect-port call.
    pub fn check(&self) -> Result<(), FakePortError> {
        self.plan
            .try_lock()
            .map(|mut plan| plan.take_next())
            .unwrap_or(Ok(()))
    }
}

impl std::fmt::Debug for FaultInjector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FaultInjector(<scripted>)")
    }
}

/// The declaration rows a harness runs its fences over.
///
/// A provider's drivers carry these rows in production. A test that exercises
/// the fences without a driver factory - `DriverDescriptor` names the runtime's
/// decoder and factory, which the toolkit does not depend on - states the same
/// rows here, and the harness derives one set of tables from either source.
#[derive(Clone, Copy)]
pub struct HarnessDeclarations {
    /// The resource types the provider's drivers own. A child another
    /// provider serves is not listed here: `create_child` commits it through
    /// the declaring driver's own `creations` row.
    pub owned_types: &'static [WellKnownType],
    /// One row per declaring driver: the children it may create.
    pub creations: &'static [(WellKnownType, &'static [ChildCreation])],
    /// The services the drivers declared, whose methods carry the contract
    /// facets and resolve the operations (U7).
    pub services: &'static [ServiceDecl],
    /// The operations the drivers declared, with their handlers.
    pub operations: &'static [OperationDef],
    /// The startup steps the drivers declared.
    pub startup: &'static [(WellKnownType, &'static [StartupStep])],
}

impl std::fmt::Debug for HarnessDeclarations {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HarnessDeclarations")
            .field("owned_type_count", &self.owned_types.len())
            .field("declaring_driver_count", &self.creations.len())
            .field("operation_count", &self.operations.len())
            .field("startup_driver_count", &self.startup.len())
            .finish_non_exhaustive()
    }
}

/// The provider test harness.
pub struct TestHarness<P: ProviderBase> {
    lifecycle: Lifecycle<P>,
    clock: Arc<DeterministicClock>,
    zone: ZoneId,
    port: Arc<RecordingPlanePort>,
    creations: CreationTable,
    owned_types: Vec<WellKnownType>,
    declared_startup: Option<&'static [(WellKnownType, &'static [StartupStep])]>,
    rows: AsyncMutex<BTreeMap<String, Arc<AdmittedRow>>>,
    children: AsyncMutex<Vec<Arc<AdmittedRow>>>,
    cause_log: AsyncMutex<Vec<(String, ReconcileCause)>>,
    audit: Arc<Mutex<ProviderAgentAuditLog>>,
    envelope: OperationEnvelope,
    faults: FaultInjector,
    attached: AtomicBool,
}

impl<P: ProviderBase> TestHarness<P> {
    /// Build the harness over one provider.
    ///
    /// The harness takes the provider's real declaration and drivers, so the
    /// fences it runs are the provider's own.
    pub fn new(provider: P) -> Self {
        Self::with_clock(
            provider,
            Arc::new(DeterministicClock::new(FIXTURE_NOW_UNIX_MS)),
        )
    }

    /// Build the harness over one provider and one clock.
    pub fn with_clock(provider: P, clock: Arc<DeterministicClock>) -> Self {
        let lifecycle = Lifecycle::with_clock(provider, Arc::clone(&clock) as SharedClock);
        let owned_types = lifecycle
            .drivers()
            .iter()
            .map(|driver| driver.resource_type)
            .collect();
        let creations = CreationTable::over(lifecycle.drivers());
        Self::assemble(lifecycle, clock, owned_types, creations, None)
    }

    /// Build the harness over explicit declaration rows.
    pub fn with_declarations(provider: P, declarations: HarnessDeclarations) -> Self {
        Self::with_declarations_at(
            provider,
            Arc::new(DeterministicClock::new(FIXTURE_NOW_UNIX_MS)),
            declarations,
        )
    }

    /// Build the harness over explicit declaration rows and one clock.
    pub fn with_declarations_at(
        provider: P,
        clock: Arc<DeterministicClock>,
        declarations: HarnessDeclarations,
    ) -> Self {
        let lifecycle = Lifecycle::with_clock(provider, Arc::clone(&clock) as SharedClock);
        Self::assemble(
            lifecycle,
            clock,
            declarations.owned_types.to_vec(),
            CreationTable::declare(declarations.creations),
            Some(declarations),
        )
    }

    fn assemble(
        lifecycle: Lifecycle<P>,
        clock: Arc<DeterministicClock>,
        owned_types: Vec<WellKnownType>,
        creations: CreationTable,
        declarations: Option<HarnessDeclarations>,
    ) -> Self {
        let audit = Arc::new(Mutex::new(ProviderAgentAuditLog::new()));
        let zone = ZoneId::parse("dev").expect("the harness zone label is valid");
        let provider_ref = ResourceRef::parse(&format!(
            "Provider/{}",
            lifecycle.declaration().provider_ref
        ))
        .expect("a declared provider reference is a valid resource reference");
        let envelope = match declarations {
            Some(declarations) => OperationEnvelope::from_operations(
                zone.clone(),
                provider_ref,
                declarations.services,
                declarations.operations,
                Arc::clone(&audit),
            ),
            None => OperationEnvelope::over(
                zone.clone(),
                provider_ref,
                lifecycle.drivers(),
                Arc::clone(&audit),
            ),
        }
        .expect("the harness zone label forms a valid zone path");
        Self {
            lifecycle,
            clock,
            zone,
            port: Arc::new(RecordingPlanePort::default()),
            creations,
            owned_types,
            declared_startup: declarations.map(|declarations| declarations.startup),
            rows: AsyncMutex::new(BTreeMap::new()),
            children: AsyncMutex::new(Vec::new()),
            cause_log: AsyncMutex::new(Vec::new()),
            audit,
            envelope,
            faults: FaultInjector::default(),
            attached: AtomicBool::new(false),
        }
    }

    /// Borrow the provider under test.
    pub fn provider(&self) -> &P {
        self.lifecycle.provider()
    }

    /// Borrow the provider's declaration.
    pub fn declaration(&self) -> &ProviderDeclaration {
        self.lifecycle.declaration()
    }

    /// Borrow the provider's declarations.
    pub fn drivers(&self) -> &[DriverDescriptor] {
        self.lifecycle.drivers()
    }

    /// Borrow the deterministic clock.
    ///
    /// The clock is the harness's own, so a test advances time explicitly
    /// instead of waiting for it.
    pub fn clock(&self) -> &Arc<DeterministicClock> {
        &self.clock
    }

    /// Borrow the real audit ring.
    pub fn audit(&self) -> &Mutex<ProviderAgentAuditLog> {
        &self.audit
    }

    /// Snapshot the audit ring.
    pub fn audit_events(&self) -> Vec<crate::audit::ProviderAgentAuditEvent> {
        self.audit
            .try_lock()
            .map(|audit| audit.events().cloned().collect())
            .unwrap_or_default()
    }

    /// Borrow the operation envelope.
    pub fn envelope(&self) -> &OperationEnvelope {
        &self.envelope
    }

    /// Every cause the harness ran, in order.
    pub fn causes(&self) -> Vec<(String, ReconcileCause)> {
        self.cause_log
            .try_lock()
            .map(|log| log.clone())
            .unwrap_or_default()
    }

    /// Every child the fence let through, in creation order.
    pub fn created_children(&self) -> Vec<Arc<AdmittedRow>> {
        self.children
            .try_lock()
            .map(|children| children.clone())
            .unwrap_or_default()
    }

    /// Every plane call the recording port accepted.
    pub fn plane_calls(&self) -> Vec<PlaneCall> {
        self.port.calls()
    }

    /// Commit a row the provider does not own - a Zone, Provider, Role, the
    /// child row a declared creation commits, or any other row a declared
    /// reference may point at.
    pub fn commit(
        &self,
        resource_type: WellKnownType,
        name: &str,
        spec: CanonicalJsonObject,
    ) -> Result<Arc<AdmittedRow>, AdmissionRefusal> {
        self.commit_row(resource_type, name, spec, false)
    }

    /// Admit a row of a resource type the provider declares.
    ///
    /// The fence is the declaration itself: a type no driver declares cannot
    /// be admitted, and a `<semantic>Ref` field that names an uncommitted
    /// row refuses the spec.
    pub fn admit(
        &self,
        resource_type: WellKnownType,
        name: &str,
        spec: CanonicalJsonObject,
    ) -> Result<Arc<AdmittedRow>, AdmissionRefusal> {
        self.commit_row(resource_type, name, spec, true)
    }

    fn commit_row(
        &self,
        resource_type: WellKnownType,
        name: &str,
        spec: CanonicalJsonObject,
        require_declared: bool,
    ) -> Result<Arc<AdmittedRow>, AdmissionRefusal> {
        if require_declared && !self.owned_types.contains(&resource_type) {
            return Err(AdmissionRefusal::UndeclaredType { resource_type });
        }
        let reference = format!(
            "{}/{}",
            resource_type.to_resource_type_name().as_str(),
            name
        );
        let row = Arc::new(AdmittedRow {
            resource_type,
            name: name.to_owned(),
            spec,
            status: AsyncMutex::new(RowStatus {
                phase: RowPhase::Pending,
                reason: None,
                requeue_after_ms: None,
                cause: None,
            }),
        });
        let mut rows = self
            .rows
            .try_lock()
            .map_err(|_| AdmissionRefusal::DuplicateRow {
                reference: reference.clone(),
            })?;
        if rows.contains_key(&reference) {
            return Err(AdmissionRefusal::DuplicateRow { reference });
        }
        for (field, target) in collect_references(&row.spec) {
            if !rows.contains_key(&target) {
                return Err(AdmissionRefusal::UnresolvedRef {
                    field,
                    reference: target,
                });
            }
        }
        rows.insert(reference, Arc::clone(&row));
        Ok(row)
    }

    /// Run one cause-carrying reconcile pass over a driver.
    pub async fn reconcile(
        &self,
        target: &dyn ReconcileTarget,
        row: &AdmittedRow,
        cause: ReconcileCause,
    ) -> Result<ReconcileOutcome, ReconcileRefusal> {
        let resource = row.resource_ref();
        let resource_key = row.reference_key();
        let known = self
            .rows
            .lock()
            .await
            .contains_key(&row.reference_key());
        if !known {
            return Err(ReconcileRefusal::UnadmittedRow(resource));
        }
        match &cause {
            ReconcileCause::TargetChanged(reference)
            | ReconcileCause::DependentChanged(reference) => {
                if !self.holds(reference) {
                    return Err(ReconcileRefusal::UnresolvedTarget(reference.clone()));
                }
            }
            ReconcileCause::OwnedSpecChanged
            | ReconcileCause::OwnedStatusChanged
            | ReconcileCause::Requeue
            | ReconcileCause::WatchFired => {}
        }
        let targets = self.targeted_by(&resource);
        let creations = HarnessCreations {
            harness: self,
            declaring: row.resource_type(),
        };
        let ctx = ReconcileCtx {
            zone: &self.zone,
            resource: &resource,
            spec: row.spec(),
            targets: &targets,
            creations: &creations,
            now_unix_ms: self.clock().now_unix_ms(),
        };
        let outcome = target.reconcile(ctx, &cause).await;
        row.publish(&cause, &outcome);
        self.cause_log.lock().await.push((resource_key, cause));
        Ok(outcome)
    }

    /// Every committed row whose declared references point at `reference`.
    pub fn targeted_by(&self, reference: &ResourceRef) -> Vec<ResourceRef> {
        let wanted = reference_key(reference);
        self.rows
            .try_lock()
            .map(|rows| {
                rows.values()
                    .filter(|row| {
                        collect_references(row.spec())
                            .iter()
                            .any(|(_, target)| *target == wanted)
                    })
                    .map(|row| row.resource_ref())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn holds(&self, reference: &ResourceRef) -> bool {
        self.rows
            .try_lock()
            .map(|rows| rows.contains_key(&reference_key(reference)))
            .unwrap_or(false)
    }

    /// Authorize one declared creation, then commit the child row.
    ///
    /// The creation runs through the provider's own declarations, so an
    /// undeclared or controller-owned creation is refused terminally. The
    /// declaration handle is the license to commit the child as well: a
    /// declared child is normally a type another provider serves (a Guest
    /// creates Volume, Process, and Endpoint rows; a VolumeBinding creates a
    /// Process), so the row commits beside the provider's own rows instead of
    /// through the owned-type fence [`TestHarness::admit`] enforces.
    pub fn create_child(
        &self,
        declaring: WellKnownType,
        declaration: &ChildCreation,
        name: &str,
        spec: CanonicalJsonObject,
    ) -> Result<Arc<AdmittedRow>, ChildCreationFailure> {
        self.creations
            .fence(declaring)
            .authorize(declaration)
            .map_err(ChildCreationFailure::Declaration)?;
        let child = self
            .commit(declaration.child, name, spec)
            .map_err(|refusal| ChildCreationFailure::Spec(refusal.code()))?;
        if let Ok(mut children) = self.children.try_lock() {
            children.push(Arc::clone(&child));
        }
        Ok(child)
    }

    /// Check a creation the driver should have performed, without performing
    /// it: the declared counterpart of the fence above.
    pub fn expect_created(
        &self,
        declaring: WellKnownType,
        child: WellKnownType,
        provider_ref: &'static str,
    ) -> Result<&'static ChildCreation, CreationRefusal> {
        let found = self.creations.declarations().find(|(owner, declaration)| {
            *owner == declaring
                && declaration.child == child
                && declaration.provider_ref == provider_ref
        });
        match found {
            Some((_, declaration)) => Ok(declaration),
            None => Err(CreationRefusal::Undeclared {
                declaring,
                child,
                provider_ref,
            }),
        }
    }

    /// Commit one operation grant.
    pub fn grant(&self, caller: &ResourceRef, operation: &ResourceRef) {
        self.envelope.commit_grant(caller, operation);
    }

    /// Invoke one operation through the real envelope.
    pub async fn call_operation(
        &self,
        caller: &ResourceRef,
        operation: &ResourceRef,
        payload: CanonicalJsonObject,
    ) -> Result<OperationResult, OperationFailure> {
        self.envelope.call(caller, operation, payload).await
    }

    /// Script the faults the provider's effect ports will see.
    pub fn script_faults(&self, plan: FaultPlan) {
        if let Ok(mut scripted) = self.faults.plan.try_lock() {
            *scripted = plan;
        }
    }

    /// Borrow the fault injector the fake effect ports consult.
    pub fn faults(&self) -> FaultInjector {
        self.faults.clone()
    }

    /// Build the zone-plane handle this provider attaches through.
    pub fn plane_handle(&self) -> ZonePlaneHandle<'_> {
        self.lifecycle.plane_handle(
            self.zone.clone(),
            Arc::clone(&self.port) as Arc<dyn ZonePlanePort>,
        )
    }

    /// Attach to the recorded zone plane.
    pub async fn attach(&self) -> Result<(), AttachError> {
        let handle = self.plane_handle();
        let result = self.lifecycle.attach(&handle).await;
        if result.is_ok() {
            self.attached.store(true, Ordering::Release);
        }
        result
    }

    /// Whether the provider attached through this harness.
    pub fn attached(&self) -> bool {
        self.attached.load(Ordering::Acquire)
    }

    /// The derived startup plan, from the provider's drivers or from the
    /// declared rows.
    pub fn startup_plan(&self) -> Result<StartupPlan, StartupError> {
        match self.declared_startup {
            Some(rows) => StartupPlan::declare(rows).map_err(StartupError::Plan),
            None => self.lifecycle.startup_plan().map_err(StartupError::Plan),
        }
    }

    /// Execute the derived startup plan in order.
    pub async fn run_startup(
        &self,
        executor: Option<&dyn StartupStepExecutor>,
    ) -> Result<(), StartupError> {
        let plan = self.startup_plan()?;
        if plan.is_empty() {
            return Ok(());
        }
        let Some(executor) = executor else {
            return Err(StartupError::ExecutorMissing);
        };
        for step in plan.steps() {
            executor
                .execute(step.declaration)
                .await
                .map_err(StartupError::Step)?;
        }
        Ok(())
    }

    /// The derived startup order.
    pub fn startup_order(&self) -> Result<Vec<&'static str>, StartupError> {
        self.startup_plan().map(|plan| plan.ids())
    }

    /// Open a bounded drain against the deterministic clock.
    pub fn drain_deadline(&self, budget_ms: u64) -> DrainDeadline {
        self.lifecycle.drain_deadline(budget_ms)
    }

    /// Drain the provider within the deadline.
    pub async fn drain(&self, budget_ms: u64) -> Result<(), DrainError> {
        self.lifecycle.drain(budget_ms).await
    }
}

/// The harness's child-creation seam, wired to the same fence the production
/// `create_child` call runs through.
struct HarnessCreations<'a, P: ProviderBase> {
    harness: &'a TestHarness<P>,
    declaring: WellKnownType,
}

#[async_trait]
impl<P: ProviderBase> CreateChild for HarnessCreations<'_, P> {
    async fn create_child(
        &self,
        declaration: &ChildCreation,
        name: &str,
        spec: CanonicalJsonObject,
    ) -> Result<(), ChildCreationFailure> {
        self.harness
            .create_child(self.declaring, declaration, name, spec)
            .map(|_| ())
    }
}

/// The `Type/name` key a resource reference is stored under.
fn reference_key(reference: &ResourceRef) -> String {
    format!(
        "{}/{}",
        reference.resource_type().as_str(),
        reference.name().as_str()
    )
}

/// Every `<semantic>Ref` and `<semantic>Refs` field a canonical spec carries,
/// as `(field path, reference)` pairs.
fn collect_references(spec: &CanonicalJsonObject) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for field in spec.keys() {
        if let Some(value) = spec.get(field) {
            collect_value(field, value, &mut found);
        }
    }
    found
}

fn collect_value(field: &str, value: &CanonicalJsonValue, found: &mut Vec<(String, String)>) {
    match value {
        CanonicalJsonValue::String(reference) if field.ends_with("Ref") => {
            found.push((field.to_owned(), reference.clone()));
        }
        CanonicalJsonValue::Array(values) if field.ends_with("Refs") => {
            for entry in values {
                if let CanonicalJsonValue::String(reference) = entry {
                    found.push((field.to_owned(), reference.clone()));
                }
            }
        }
        CanonicalJsonValue::Object(values) => {
            for (nested_field, nested) in values {
                collect_value(&format!("{field}.{nested_field}"), nested, found);
            }
        }
        CanonicalJsonValue::Array(values) => {
            for nested in values {
                collect_value(field, nested, found);
            }
        }
        CanonicalJsonValue::Null
        | CanonicalJsonValue::Bool(_)
        | CanonicalJsonValue::Integer(_)
        | CanonicalJsonValue::String(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_resource_types::{Cardinality, IsolationPosture, ProviderDeclaration};

    const DECLARATION: ProviderDeclaration = ProviderDeclaration {
        provider_ref: "harness",
        self_bindings: &[],
        required: false,
        cardinality: Cardinality::AtMostOne,
        isolation_posture: IsolationPosture::Standard,
        plane_adapters: &[],
        principals: &[],
        storage_roots: &[],
    };

    struct EmptyProvider;

    #[async_trait]
    impl ProviderBase for EmptyProvider {
        fn declaration(&self) -> &ProviderDeclaration {
            &DECLARATION
        }

        fn drivers(&self) -> &'static [DriverDescriptor] {
            &[]
        }

        async fn attach(&self, _zone: &ZonePlaneHandle<'_>) -> Result<(), AttachError> {
            Ok(())
        }

        async fn drain(&self, _deadline: DrainDeadline) -> Result<(), DrainError> {
            Ok(())
        }
    }

    #[test]
    fn a_ready_future_completes_on_the_calling_thread() {
        assert_eq!(block_on(async { 7_u8 }), 7);
    }

    #[test]
    fn an_undeclared_type_cannot_be_admitted() {
        let harness = TestHarness::new(EmptyProvider);
        assert_eq!(
            harness
                .admit(WellKnownType::VOLUME, "media", CanonicalJsonObject::empty())
                .unwrap_err()
                .code(),
            "undeclared-type"
        );
        assert!(harness.plane_calls().is_empty());
    }

    #[test]
    fn a_declared_reference_must_resolve_before_the_spec_is_committed() {
        let harness = TestHarness::new(EmptyProvider);
        let spec =
            CanonicalJsonObject::parse(br#"{"volumeRef":"Volume/media"}"#).expect("canonical spec");
        let refusal = harness
            .commit(WellKnownType::VOLUME_BINDING, "media-bind", spec)
            .unwrap_err();
        assert_eq!(refusal.code(), "unresolved-ref");
        harness
            .commit(WellKnownType::VOLUME, "media", CanonicalJsonObject::empty())
            .expect("the referenced row resolves once committed");
        let spec =
            CanonicalJsonObject::parse(br#"{"volumeRef":"Volume/media"}"#).expect("canonical spec");
        let row = harness
            .commit(WellKnownType::VOLUME_BINDING, "media-bind", spec)
            .expect("the reference resolves");
        assert_eq!(row.status().phase(), RowPhase::Pending);
        assert_eq!(
            harness.targeted_by(&row.resource_ref()).len(),
            0,
            "no committed row points at the binding itself"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn attaching_claims_only_declared_roots() {
        let harness = TestHarness::new(EmptyProvider);
        harness
            .attach()
            .await
            .expect("the empty declaration attaches");
        assert!(harness.attached());
        assert!(harness.plane_calls().is_empty());
    }
}
