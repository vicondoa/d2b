//! Guest runtime-provider family driver (U12 wave 2): the v3 `ResourceDriver`
//! conversion of the U6 shared Runner leg for the four Guest runtime
//! Providers - cloud-hypervisor, qemu-media, azure-container-apps, and
//! azure-virtual-machine (R3, R4, R30).
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`GuestDriverFactory`] registration under `Guest`.
//! - `validate_spec` -> [`ResourceDriver::validate`]: the spec decodes and
//!   names a runtime Provider this factory owns.
//! - `observe` -> [`ResourceDriver::recover`]: owned-child / provider
//!   adoption evidence.
//! - finalizer enrollment + `plan`/`reconcile`/`execute_effect` ->
//!   [`ResourceDriver::reconcile`]: the kind's desired child set is ensured
//!   through the manager child API (committed before the child actor exists,
//!   F1), owned children the desired set no longer derives are retired in
//!   the family's preserved order, the typed Provider effect runs behind
//!   [`GuestDriverEffects`], and the in-memory status projection is
//!   published with `ctx.set_status` + `ctx.set_status_projection` (R11)
//!   plus a self-`requeue_after` while the family is not converged.
//! - `prepare_finalize`/`execute_finalize`/`finalize` ->
//!   [`ResourceDriver::delete`]: the kind's preserved teardown stage
//!   (Cloud Hypervisor's controller-owned finalize, the framework
//!   controllers' per-kind finalize) runs until the Provider's internal
//!   finalizer gate clears, then the owned children retire.
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11); the Cloud
//!   Hypervisor controller's layered Guest status is captured by the effect
//!   call and published as the row's `status.resource` projection - the row's
//!   actor owns it, never the old durable store.
//!
//! The Cloud Hypervisor kind is the real host path: its children
//! (`Process/<guest>-vmm`, `Endpoint/<guest>-ch-api`,
//! `Endpoint/<guest>-guest-control`, `Volume/<guest>-system`) are committed
//! by the controller session through the plane's child bridge, so the driver
//! neither ensures nor retires them - the session's own ordering owns them.
//! The qemu-media and azure-container-apps kinds own their child rows through
//! the manager child API; azure-virtual-machine owns none.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
};
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, RowLookup, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName, StoredDesiredResource};
use d2b_resource_runtime::spec_store::EnsureOutcome;
use serde_json::{Value, json};

/// The one ResourceType this factory serves.
pub(crate) const GUEST_TYPE_NAME: &str = "Guest";

/// Canonical Host execution target (the old shared Runner's Host ref).
pub(crate) const HOST_REF: &str = "Host/host-system";

/// Preserved reconcile self-resync for Guests whose Provider is not
/// converged (old shared Runner repair interval).
pub(crate) const GUEST_RESYNC: Duration = Duration::from_secs(30);

/// One runtime-Provider row of the converted Guest family.
///
/// The table pins the provider identity every effect call binds: the
/// controller reference and the Provider reference (the row selector the old
/// registration carried). The descriptors themselves are gone with the old
/// Runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuestRegistration {
    pub(crate) kind: GuestKind,
    pub(crate) controller_ref: &'static str,
    pub(crate) provider_ref: &'static str,
    /// Preserved resync cadence (old shared Runner repair interval).
    pub(crate) resync: Duration,
}

/// The four Guest runtime-Provider registrations, in the preserved order.
pub(crate) const GUEST_REGISTRATIONS: [GuestRegistration; 4] = [
    GuestRegistration {
        kind: GuestKind::CloudHypervisor,
        controller_ref: "Process/cloud-hypervisor-controller",
        provider_ref: "Provider/runtime-cloud-hypervisor",
        resync: GUEST_RESYNC,
    },
    GuestRegistration {
        kind: GuestKind::QemuMedia,
        controller_ref: "Process/runtime-qemu-media-controller",
        provider_ref: "Provider/runtime-qemu-media",
        resync: GUEST_RESYNC,
    },
    GuestRegistration {
        kind: GuestKind::AzureContainerApps,
        controller_ref: "Process/aca-controller",
        provider_ref: "Provider/runtime-azure-container-apps",
        resync: GUEST_RESYNC,
    },
    GuestRegistration {
        kind: GuestKind::AzureVirtualMachine,
        controller_ref: "Process/azure-vm-controller-process",
        provider_ref: "Provider/runtime-azure-virtual-machine",
        resync: GUEST_RESYNC,
    },
];

/// Closed runtime-Provider handler set served by this family (old
/// `SharedProviderResourceKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum GuestKind {
    CloudHypervisor,
    QemuMedia,
    AzureContainerApps,
    AzureVirtualMachine,
}

impl GuestKind {
    /// Resolve the family row for one (ResourceType, Provider) pair.
    ///
    /// The old Runner selected the row by its registration tuple; the new
    /// plane knows only the key, so the row's Provider reference - carried by
    /// the spec and fenced by `validate` - selects the Provider handler. A
    /// Provider this factory does not own is refused, never guessed.
    pub(crate) fn from_type_and_provider(
        resource_type: &str,
        provider_ref: Option<&str>,
    ) -> Result<Self, GuestEffectError> {
        if resource_type != GUEST_TYPE_NAME {
            return Err(GuestEffectError::InvalidResource);
        }
        let Some(provider_ref) = provider_ref else {
            return Err(GuestEffectError::InvalidResource);
        };
        GUEST_REGISTRATIONS
            .iter()
            .find(|registration| registration.provider_ref == provider_ref)
            .map(|registration| registration.kind)
            .ok_or(GuestEffectError::InvalidResource)
    }

    pub(crate) const fn registration(self) -> GuestRegistration {
        GUEST_REGISTRATIONS[self.index()]
    }

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::CloudHypervisor => 0,
            Self::QemuMedia => 1,
            Self::AzureContainerApps => 2,
            Self::AzureVirtualMachine => 3,
        }
    }

    pub(crate) const fn effect_id(self) -> &'static str {
        match self {
            Self::CloudHypervisor => "runtime-cloud-hypervisor-guest",
            Self::QemuMedia => "runtime-qemu-media-guest",
            Self::AzureContainerApps => "runtime-azure-container-apps-guest",
            Self::AzureVirtualMachine => "runtime-azure-virtual-machine-guest",
        }
    }

    pub(crate) const fn provider_ref(self) -> &'static str {
        self.registration().provider_ref
    }

    pub(crate) const fn controller_ref(self) -> &'static str {
        self.registration().controller_ref
    }

    pub(crate) const fn resync(self) -> Duration {
        self.registration().resync
    }

    /// Whether this kind's owned children are committed by its Provider
    /// controller's session instead of by the driver. The Cloud Hypervisor
    /// controller commits its fixed child roles through the plane's child
    /// bridge; a driver that also ensured (or retired) them would race the
    /// session that owns their ordering.
    pub(crate) const fn children_owned_by_controller(self) -> bool {
        matches!(self, Self::CloudHypervisor)
    }
}

// ---------------------------------------------------------------------------
// Typed Provider effect boundary (old `SharedProviderEffectExecutor` rows)
// ---------------------------------------------------------------------------

/// Phase returned by one typed Provider effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuestEffectPhase {
    Ready,
    Pending,
}

/// One Provider effect outcome: the phase the old effect returned plus the
/// layered `status.resource` projection the Provider published for the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuestEffectOutcome {
    pub(crate) phase: GuestEffectPhase,
    pub(crate) resource_projection: Option<Value>,
}

impl GuestEffectOutcome {
    pub(crate) const fn phase(phase: GuestEffectPhase) -> Self {
        Self {
            phase,
            resource_projection: None,
        }
    }

    pub(crate) fn projection(phase: GuestEffectPhase, resource_projection: Value) -> Self {
        Self {
            phase,
            resource_projection: Some(resource_projection),
        }
    }
}

/// Outcome of one Provider teardown stage (old `execute_finalize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuestFinalizeStage {
    /// Cleanup finished; the owned children may retire.
    Complete,
    /// Cleanup is progressing; the owner is re-entered (old `Pending`).
    Pending,
}

/// Closed failure surface for Guest Provider adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuestEffectError {
    /// Cleanup is progressing and the owner should be re-entered.
    Pending,
    /// The Provider path is not currently available and should retry.
    Unavailable,
    /// Fresh resource or assignment evidence failed closed.
    InvalidResource,
}

impl core::fmt::Display for GuestEffectError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Pending => "guest-effect-pending",
            Self::Unavailable => "guest-effect-unavailable",
            Self::InvalidResource => "guest-resource-invalid",
        })
    }
}

impl std::error::Error for GuestEffectError {}

// ---------------------------------------------------------------------------
// Driver error classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuestDriverErrorKind {
    /// The durable spec did not decode or names a Provider outside the row's
    /// ResourceType: terminal, retrying cannot change the stored spec.
    SpecInvalid,
    /// A manager child mutation failed (retryable: the manager owns retries).
    ChildMutation,
    /// The Provider path is temporarily unavailable.
    ProviderUnavailable,
    /// A Provider teardown stage is still progressing.
    FinalizePending,
}

impl GuestDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::SpecInvalid => FailureClass::Terminal,
            Self::ChildMutation | Self::ProviderUnavailable | Self::FinalizePending => {
                FailureClass::Retryable
            }
        }
    }

    /// The registered failure kind this classification reports (issue #508).
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::GUEST_SPEC_INVALID,
            Self::ChildMutation => FailureKinds::GUEST_CHILD_MUTATION,
            Self::ProviderUnavailable => FailureKinds::GUEST_PROVIDER_UNAVAILABLE,
            Self::FinalizePending => FailureKinds::GUEST_FINALIZE_PENDING,
        }
    }

    const fn code(self) -> &'static str {
        self.failure_kind().code()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GuestDriverError {
    kind: GuestDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl GuestDriverError {
    const fn new(kind: GuestDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op, detail: FailureDetail::new() }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }

    /// Return the closed failure class this error is reported with.
    pub(crate) const fn class(&self) -> FailureClass {
        self.kind.class()
    }

    /// Return the operation that failed.
    pub(crate) const fn op(&self) -> DriverOp {
        self.op
    }
}

impl core::fmt::Display for GuestDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.kind.code())
    }
}

impl std::error::Error for GuestDriverError {}

/// Typed in-memory status projection (R11: never persisted). Carries the
/// closed phase the old status candidate published plus the Provider's
/// layered `status.resource` projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuestDriverStatus {
    pub(crate) phase: GuestEffectPhase,
    pub(crate) resource: Option<Value>,
}

impl GuestDriverStatus {
    pub(crate) const fn phase(&self) -> &'static str {
        match self.phase {
            GuestEffectPhase::Ready => "Ready",
            GuestEffectPhase::Pending => "Pending",
        }
    }
}

// ---------------------------------------------------------------------------
// Spec decode (manager-wired)
// ---------------------------------------------------------------------------

/// Decoded Guest spec envelope: the exact stored spec bytes and the canonical
/// spec document the family's Provider handlers read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuestSpecEnvelope {
    /// The exact stored spec bytes; never rewritten by this driver.
    raw: Vec<u8>,
    value: Value,
}

impl GuestSpecEnvelope {
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    pub(crate) fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The spec's Provider reference (the family row selector).
    pub(crate) fn provider_ref(&self) -> Option<&str> {
        self.value.get("providerRef").and_then(Value::as_str)
    }
}

/// Closed decode error for a Guest spec envelope.
#[derive(Debug, thiserror::Error)]
#[error("guest spec must be a JSON object")]
pub(crate) struct GuestSpecDecodeError;

/// The manager-wired decode hook for `Guest`.
pub(crate) fn guest_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        let value =
            serde_json::from_slice::<Value>(bytes).map_err(|_| GuestSpecDecodeError)?;
        if !value.is_object() {
            return Err(GuestSpecDecodeError);
        }
        Ok(GuestSpecEnvelope {
            raw: bytes.to_vec(),
            value,
        })
    })
}

/// Decode one row's metadata envelope (empty metadata is the empty object).
pub(crate) fn decode_metadata(raw: &[u8]) -> Result<Value, GuestEffectError> {
    if raw.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice::<Value>(raw).map_err(|_| GuestEffectError::InvalidResource)
}

/// Convert one durable 16-byte uid to its canonical identity (the manager
/// persists the uid as bytes; the Provider effects key on the canonical
/// string).
pub(crate) fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, GuestEffectError> {
    let mut bytes = *bytes;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).map_err(|_| GuestEffectError::InvalidResource)
}

// ---------------------------------------------------------------------------
// Manager-routed child surface
// ---------------------------------------------------------------------------

/// One owned child row as the Provider effects observe it: the row identity
/// plus the live phase its actor published (R11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuestChildObservation {
    pub(crate) key: ResourceKey,
    pub(crate) deleting: bool,
    /// Live phase from the manager view (`None` = nothing published yet).
    pub(crate) phase: Option<&'static str>,
}

impl GuestChildObservation {
    /// Whether this child is converged (old `Ready | Succeeded`).
    pub(crate) fn ready(&self) -> bool {
        matches!(self.phase, Some("Ready" | "Succeeded"))
    }
}

/// Manager-routed child reads and mutations handed to one Provider effect
/// call.
///
/// The driver owns this surface (it is the only holder of the resource's
/// [`ResourceContext`]); the Provider effects use it to read the child set
/// they gate on. Every mutation rides [`ResourceContext::ensure_child`] /
/// [`ResourceContext::delete`], so the manager commits the child row BEFORE
/// the child actor exists (F1, AE1).
#[async_trait]
pub(crate) trait GuestChildSurface: Send + Sync {
    /// Every child row the calling resource owns, with its live phase.
    async fn owned(&self) -> Result<Vec<GuestChildObservation>, GuestEffectError>;
}

/// [`GuestChildSurface`] over the calling resource's context.
///
/// The context is held behind an async mutex because every Provider
/// controller drives its child reads through `&self` ports while the driver
/// keeps the only `&mut ResourceContext`; the guard is never held across
/// another effect call, so the single-threaded drive order is preserved.
pub(crate) struct ContextChildSurface<'a> {
    ctx: tokio::sync::Mutex<&'a mut ResourceContext>,
}

impl<'a> ContextChildSurface<'a> {
    pub(crate) fn new(ctx: &'a mut ResourceContext) -> Self {
        Self {
            ctx: tokio::sync::Mutex::new(ctx),
        }
    }
}

#[async_trait]
impl GuestChildSurface for ContextChildSurface<'_> {
    async fn owned(&self) -> Result<Vec<GuestChildObservation>, GuestEffectError> {
        let mut ctx = self.ctx.lock().await;
        let children = ctx
            .children()
            .await
            .map_err(|_| GuestEffectError::Unavailable)?;
        let mut observations = Vec::with_capacity(children.len());
        for child in children {
            let phase = match ctx.lookup_view(&child.key).await {
                RowLookup::Present { row: view, .. } => Some(view_phase(&view)),
                RowLookup::Absent { .. } => None,
                RowLookup::Unavailable { .. } | RowLookup::Error { .. } => {
                    return Err(GuestEffectError::Unavailable)
                }
            };
            observations.push(GuestChildObservation {
                key: child.key,
                deleting: child.deleting,
                phase,
            });
        }
        Ok(observations)
    }
}

/// The row's live phase as the old effects read `/status/phase`: the
/// canonical wire phase of the row's observed status (issue #515).
/// `ResourceStatus::wire_phase` owns the vocabulary (`Deleting` renders as
/// the `Deleted` tombstone); a status published for another generation is
/// not observed state of the current row, so it reads `Pending`, never a
/// stale `Ready`.
pub(crate) fn view_phase(view: &d2b_resource_runtime::manager::ResourceView) -> &'static str {
    use d2b_resource_runtime::resource::ResourceStatus;
    view.observed_status()
        .as_ref()
        .map(ResourceStatus::wire_phase)
        .unwrap_or("Pending")
}

/// The sink one Cloud Hypervisor Guest effect call captures the Provider
/// controller's status write into: the converted Guest's status is
/// actor-local, so the effect call that drives the controller is the only
/// place it can be observed (R11: no dual-write into any store).
pub(crate) type GuestStatusSink = Arc<parking_lot::Mutex<Option<Value>>>;

/// A fresh, empty Guest status sink.
pub(crate) fn guest_status_sink() -> GuestStatusSink {
    Arc::new(parking_lot::Mutex::new(None))
}

// ---------------------------------------------------------------------------
// Effect request
// ---------------------------------------------------------------------------

/// Everything one Provider effect call may read from the driver.
pub(crate) struct GuestEffectRequest<'a> {
    pub(crate) zone: ZoneId,
    /// The Guest resource reference (the Provider handlers key on it).
    pub(crate) target: ResourceRef,
    pub(crate) key: ResourceKey,
    /// The row's durable uid (the Provider effects key on it).
    pub(crate) uid: ResourceUid,
    pub(crate) generation: ResourceGeneration,
    /// Controller generation every effect call binds (KTD7).
    pub(crate) controller_generation: ControllerGeneration,
    /// Runtime-only operation id (never persisted).
    pub(crate) operation_id: String,
    /// Canonical spec document of the row.
    pub(crate) spec: Value,
    /// Decoded metadata envelope of the row (`ownerRef`, ...).
    pub(crate) metadata: Value,
    /// The Provider row's spec for this Guest's `providerRef`, when the
    /// manager holds it (the framework kinds read their `/config` here).
    pub(crate) provider_spec: Option<Value>,
    /// The driver's last published `status.resource` projection, when one
    /// was published: the Provider status is sticky, exactly as the old
    /// durable row's was.
    pub(crate) status: Option<Value>,
    /// Manager-routed child reads.
    pub(crate) children: &'a dyn GuestChildSurface,
    /// Capture point for the Cloud Hypervisor controller's status write.
    pub(crate) status_sink: GuestStatusSink,
}

impl GuestEffectRequest<'_> {
    /// The owner reference the old effects read from `/metadata/ownerRef`.
    pub(crate) fn owner_ref(&self) -> Result<ResourceRef, GuestEffectError> {
        self.metadata
            .get("ownerRef")
            .and_then(Value::as_str)
            .and_then(|value| ResourceRef::parse(value).ok())
            .ok_or(GuestEffectError::InvalidResource)
    }
}

/// Typed Provider effect boundary owned by the d2bd composition root.
///
/// This is the family's dyn-erased port: the driver sees only these closed,
/// typed calls, and the production implementation owns the Provider
/// controllers, the Cloud Hypervisor controller session, and the framework
/// state machines.
#[async_trait]
pub(crate) trait GuestDriverEffects: Send + Sync + 'static {
    /// Reconcile one Guest through its selected runtime Provider.
    async fn reconcile(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
    ) -> Result<GuestEffectOutcome, GuestEffectError>;

    /// Advance one Guest's Provider teardown stage (old `execute_finalize`).
    async fn finalize(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
    ) -> Result<GuestFinalizeStage, GuestEffectError>;
}

/// Explicit unavailable adapter used only before production composition
/// supplies the daemon-owned typed effect boundary.
pub(crate) struct UnavailableGuestDriverEffects;

#[async_trait]
impl GuestDriverEffects for UnavailableGuestDriverEffects {
    async fn reconcile(
        &self,
        _kind: GuestKind,
        _request: &GuestEffectRequest<'_>,
    ) -> Result<GuestEffectOutcome, GuestEffectError> {
        Err(GuestEffectError::Unavailable)
    }

    async fn finalize(
        &self,
        _kind: GuestKind,
        _request: &GuestEffectRequest<'_>,
    ) -> Result<GuestFinalizeStage, GuestEffectError> {
        Err(GuestEffectError::Unavailable)
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Construction arguments shared by every driver of the family.
#[derive(Clone)]
pub(crate) struct GuestDriverArgs {
    pub(crate) zone: String,
    pub(crate) controller_generation: ControllerGeneration,
    pub(crate) effects: Arc<dyn GuestDriverEffects>,
}

/// Factory for the `Guest` ResourceType.
///
/// Construction is infallible by contract (R3).
pub(crate) struct GuestDriverFactory {
    types: Vec<ResourceTypeName>,
    args: GuestDriverArgs,
}

impl GuestDriverFactory {
    pub(crate) fn new(args: GuestDriverArgs) -> Self {
        Self {
            types: vec![ResourceTypeName::new(GUEST_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait]
impl ResourceDriverFactory for GuestDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(GuestDriver::new(self.args.clone()))
    }
}

/// One desired Guest resource.
pub(crate) struct GuestDriver {
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    effects: Arc<dyn GuestDriverEffects>,
    /// Dependency keys already watched (R12: exactly once per target).
    watched: Vec<ResourceKey>,
}

impl GuestDriver {
    pub(crate) fn new(args: GuestDriverArgs) -> Self {
        let zone = ZoneId::parse(args.zone).expect("driver zone was validated at construction");
        Self {
            zone,
            controller_generation: args.controller_generation,
            effects: args.effects,
            watched: Vec::new(),
        }
    }

    /// The controller generation every effect call binds (KTD7).
    pub(crate) fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }

    fn error(&self, kind: GuestDriverErrorKind, op: DriverOp) -> GuestDriverError {
        GuestDriverError::new(kind, op)
    }

    /// The decoded spec envelope of the row being driven.
    fn envelope(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<GuestSpecEnvelope, GuestDriverError> {
        ctx.spec::<GuestSpecEnvelope>()
            .cloned()
            .map_err(|_| self.error(GuestDriverErrorKind::SpecInvalid, op))
    }

    /// Decoded metadata envelope of the row being driven.
    fn metadata(&self, ctx: &ResourceContext, op: DriverOp) -> Result<Value, GuestDriverError> {
        decode_metadata(ctx.metadata())
            .map_err(|_| self.error(GuestDriverErrorKind::SpecInvalid, op))
    }

    /// The family row this row is served by: ResourceType plus the spec's
    /// Provider reference (old `from_registration`).
    fn kind(
        &self,
        ctx: &ResourceContext,
        envelope: &GuestSpecEnvelope,
        op: DriverOp,
    ) -> Result<GuestKind, GuestDriverError> {
        if ctx.key().zone != self.zone.as_str() {
            return Err(self.error(GuestDriverErrorKind::SpecInvalid, op).with_detail(
                FailureDetail::at("kind/zone").comparison(FailureComparison::new(
                    "resource.zone",
                    self.zone.as_str(),
                    ctx.key().zone.as_str(),
                )),
            ));
        }
        GuestKind::from_type_and_provider(&ctx.key().type_name, envelope.provider_ref())
            .map_err(|_| {
                self.error(GuestDriverErrorKind::SpecInvalid, op).with_detail(
                    FailureDetail::at("kind/provider").comparison(FailureComparison::new(
                        "spec.providerRef",
                        format!("a {} provider this family owns", ctx.key().type_name),
                        envelope
                            .provider_ref()
                            .map(str::to_owned)
                            .unwrap_or_else(|| "absent".to_owned()),
                    )),
                )
            })
    }

    /// Register one internal dependency watch exactly once per target (R12).
    ///
    /// Best-effort by design: a dependency that is still an unconverted row
    /// has no actor to watch yet, and the resync schedule re-evaluates it.
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) || target == *ctx.key() {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    /// The runtime-only operation id of one pass (never persisted).
    fn operation_id(&self, ctx: &ResourceContext, kind: GuestKind) -> String {
        format!(
            "{}-{}-g{}",
            kind.effect_id(),
            ctx.uid()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            ctx.generation(),
        )
    }

    fn child_key(&self, target: &ResourceRef) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        )
    }

    /// The Provider row's spec for this Guest's `providerRef`, read through
    /// the manager when it holds the row. A Provider the manager does not
    /// serve (or does not hold) answers `None`; the effect then refuses
    /// closed exactly as the old fence did.
    async fn provider_spec(
        &self,
        ctx: &mut ResourceContext,
        kind: GuestKind,
        op: DriverOp,
    ) -> Result<Option<Value>, GuestDriverError> {
        let provider_ref = ResourceRef::parse(kind.provider_ref())
            .map_err(|_| self.error(GuestDriverErrorKind::SpecInvalid, op))?;
        let key = self.child_key(&provider_ref);
        let lookup = ctx.lookup(&key).await;
        match lookup {
            RowLookup::Present { row, .. } => serde_json::from_slice::<Value>(&row.spec)
                .map(Some)
                .map_err(|error| {
                    self.error(GuestDriverErrorKind::SpecInvalid, op)
                        .with_detail(
                            FailureDetail::at("provider/decode")
                                .comparison(FailureComparison::new(
                                    "provider.row",
                                    "a decodable spec",
                                    "decode failed",
                                ))
                                .with_note(error.to_string()),
                        )
                }),
            RowLookup::Absent { .. } => Ok(None),
            RowLookup::Unavailable { .. } | RowLookup::Error { .. } => {
                let mut detail = FailureDetail::at("provider/lookup");
                if let Some(comparison) = lookup.failure_comparison("provider.row", "present") {
                    detail = detail.comparison(comparison);
                }
                if let Some(error) = lookup.error_detail() {
                    detail = detail.with_note(error);
                }
                Err(self
                    .error(GuestDriverErrorKind::ProviderUnavailable, op)
                    .with_detail(detail))
            }
        }
    }

    /// The desired child set of one reconcile pass, as manager child rows.
    ///
    /// The Cloud Hypervisor and AzureVM kinds commit no children through this
    /// path: the Cloud Hypervisor controller session owns its fixed child
    /// roles (the plane's child bridge commits them), and the AzureVM kind
    /// owns none. The qemu-media and azure-container-apps kinds declare the
    /// child rows the old effects materialized.
    async fn desired_children(
        &self,
        ctx: &mut ResourceContext,
        kind: GuestKind,
        envelope: &GuestSpecEnvelope,
        provider_spec: Option<&Value>,
    ) -> Result<Vec<ChildEnsure>, GuestDriverError> {
        let op = DriverOp::Reconcile;
        let owner = key_ref(ctx.key());
        let spec = envelope.value();
        match kind {
            GuestKind::QemuMedia => {
                let provider = provider_spec.ok_or_else(|| {
                    self.error(GuestDriverErrorKind::ProviderUnavailable, op)
                })?;
                qemu_child_ensures(spec, provider, &owner, op)
            }
            GuestKind::AzureContainerApps => aca_child_ensures(&owner, op),
            GuestKind::CloudHypervisor | GuestKind::AzureVirtualMachine => Ok(Vec::new()),
        }
    }

    /// Retire owned children the desired set no longer derives, in the
    /// family's preserved order (endpoint-first, process-last; R9/F3).
    ///
    /// The Cloud Hypervisor kind never retires anything here: its child rows
    /// are the controller session's, and the session's own teardown ordering
    /// (ch-api -> guest-control -> vmm -> system) owns their retirement.
    async fn retire_obsolete_children(
        &self,
        ctx: &mut ResourceContext,
        desired: &[ChildEnsure],
        op: DriverOp,
    ) -> Result<bool, GuestDriverError> {
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(GuestDriverErrorKind::ChildMutation, op))?;
        let mut obsolete = owned
            .iter()
            .filter(|row| {
                !row.deleting
                    && !desired
                        .iter()
                        .any(|child| owned_child_matches_child_ensure(row, child))
            })
            .collect::<Vec<_>>();
        obsolete.sort_by_key(|row| (teardown_rank(&row.key.type_name), row.key.name.clone()));
        let mut mutated = false;
        for row in obsolete {
            ctx.delete(&row.key)
                .await
                .map_err(|_| self.error(GuestDriverErrorKind::ChildMutation, op))?;
            mutated = true;
        }
        Ok(mutated)
    }

    fn effect_error(&self, error: GuestEffectError, op: DriverOp) -> GuestDriverError {
        match error {
            GuestEffectError::InvalidResource => {
                self.error(GuestDriverErrorKind::SpecInvalid, op)
            }
            GuestEffectError::Pending => self.error(GuestDriverErrorKind::FinalizePending, op),
            GuestEffectError::Unavailable => self
                .error(GuestDriverErrorKind::ProviderUnavailable, op)
                .with_detail(FailureDetail::at("effect/provider").with_note(error.to_string())),
        }
    }

    /// Whether every declared dependency of this row is live-Ready, read
    /// through the effect port (the old descriptor's dependency hold).
    async fn run_effect(
        &self,
        kind: GuestKind,
        request: &GuestEffectRequest<'_>,
    ) -> Result<GuestEffectOutcome, GuestDriverError> {
        self.effects
            .reconcile(kind, request)
            .await
            .map_err(|error| self.effect_error(error, DriverOp::Reconcile))
    }
}

impl core::fmt::Debug for GuestDriver {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GuestDriver")
            .field("zone", &self.zone)
            .finish_non_exhaustive()
    }
}

impl core::fmt::Debug for GuestDriverFactory {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GuestDriverFactory")
            .field("types", &self.types)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ResourceDriver for GuestDriver {
    type Error = GuestDriverError;

    fn classify_error(&self, error: &GuestDriverError) -> DriverFailure {
        let failure = match error.kind {
            GuestDriverErrorKind::SpecInvalid => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            GuestDriverErrorKind::FinalizePending => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            GuestDriverErrorKind::ChildMutation | GuestDriverErrorKind::ProviderUnavailable => {
                DriverFailure::error(error.op, error.kind.failure_kind(), FailureClass::Retryable)
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Structural validation (old `validate_spec`): the stored spec decodes
    /// and names a runtime Provider this family owns.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let envelope = self.envelope(ctx, DriverOp::Validate)?;
        let _ = self.kind(ctx, &envelope, DriverOp::Validate)?;
        Ok(())
    }

    /// Discovery and adoption on the realization target (F2): the kinds that
    /// own manager children adopt when their complete desired child set is
    /// already present and current; the Cloud Hypervisor kind adopts when its
    /// committed VMM child row is live (the controller session's evidence);
    /// the AzureVM kind realizes nothing through resource rows and adopts.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let envelope = self.envelope(ctx, op)?;
        let kind = self.kind(ctx, &envelope, op)?;
        if kind.children_owned_by_controller() {
            let vmm_ref = d2b_provider_runtime_cloud_hypervisor::deterministic_child_ref(
                &key_ref(ctx.key()),
                d2b_provider_runtime_cloud_hypervisor::ChildRole::VmmProcess,
            )
            .map_err(|_| self.error(GuestDriverErrorKind::SpecInvalid, op))?;
            let owned = ctx
                .children()
                .await
                .map_err(|_| self.error(GuestDriverErrorKind::ChildMutation, op))?;
            let present = owned.iter().any(|row| {
                row.key.type_name == vmm_ref.resource_type().as_str()
                    && row.key.name == vmm_ref.name().as_str()
                    && !row.deleting
            });
            return Ok(if present {
                RecoveryOutcome::Adopted
            } else {
                RecoveryOutcome::Missing
            });
        }
        let provider_spec = self.provider_spec(ctx, kind, op).await?;
        let desired = self
            .desired_children(ctx, kind, &envelope, provider_spec.as_ref())
            .await?;
        if desired.is_empty() {
            return Ok(RecoveryOutcome::Adopted);
        }
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(GuestDriverErrorKind::ChildMutation, op))?;
        let current = desired.iter().all(|child| {
            owned
                .iter()
                .any(|row| owned_child_matches_child_ensure(row, child) && !row.deleting)
        });
        Ok(if current {
            RecoveryOutcome::Adopted
        } else {
            RecoveryOutcome::Missing
        })
    }

    /// One reconcile pass (old `plan` + `reconcile` + `execute_effect`).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let envelope = self.envelope(ctx, op)?;
        let kind = self.kind(ctx, &envelope, op)?;
        let metadata = self.metadata(ctx, op)?;

        // Dependency edges (R12/R17): the resources this family's effects
        // read are watched so their readiness or death wakes this actor.
        for dependency in declared_dependency_refs(envelope.value()) {
            self.watch_once(ctx, self.child_key(&dependency)).await;
        }

        let provider_spec = self.provider_spec(ctx, kind, op).await?;

        // Desired child set through the manager child API (F1): every row is
        // committed before its actor exists. The Cloud Hypervisor kind owns
        // no manager children (its controller session commits them).
        let desired = self
            .desired_children(ctx, kind, &envelope, provider_spec.as_ref())
            .await?;
        let mut mutated = false;
        for child in &desired {
            match ctx.ensure_child(child.clone()).await {
                Ok(EnsureOutcome::Created(_)) | Ok(EnsureOutcome::Updated(_)) => mutated = true,
                Ok(EnsureOutcome::Unchanged(_)) => {}
                Err(_) => {
                    return Err(self.error(GuestDriverErrorKind::ChildMutation, op));
                }
            }
        }
        if !kind.children_owned_by_controller() {
            mutated |= self.retire_obsolete_children(ctx, &desired, op).await?;
        }
        for child in &desired {
            self.watch_once(
                ctx,
                ResourceKey::new(self.zone.as_str(), child.type_name.as_str(), child.name.as_str()),
            )
            .await;
        }

        let operation_id = self.operation_id(ctx, kind);
        let target = key_ref(ctx.key());
        let key = ctx.key().clone();
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(GuestDriverErrorKind::SpecInvalid, op))?;
        let generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(GuestDriverErrorKind::SpecInvalid, op))?;
        let status = ctx
            .status::<GuestDriverStatus>()
            .and_then(|status| status.resource.clone());
        let status_sink = guest_status_sink();
        // The surface takes the only mutable borrow for the effect call; every
        // read above was already taken from the context.
        let outcome = {
            let surface = ContextChildSurface::new(ctx);
            // Old effect gate: a kind whose declared children are not
            // converged reports Pending whatever its Provider controller
            // says, so the Guest cannot go Ready ahead of its own children.
            let children_ready = if desired.is_empty() {
                true
            } else {
                let owned = surface.owned().await.map_err(|_| {
                    self.error(GuestDriverErrorKind::ChildMutation, DriverOp::Reconcile)
                })?;
                desired.iter().all(|child| {
                    owned.iter().any(|row| {
                        row.key.type_name == child.type_name.as_str()
                            && row.key.name == child.name
                            && row.ready()
                    })
                })
            };
            let request = GuestEffectRequest {
                zone: self.zone.clone(),
                target,
                key,
                uid,
                generation,
                controller_generation: self.controller_generation,
                operation_id,
                spec: envelope.value().clone(),
                metadata,
                provider_spec,
                status,
                children: &surface,
                status_sink: Arc::clone(&status_sink),
            };
            let outcome = self.run_effect(kind, &request).await?;
            if !children_ready {
                GuestEffectOutcome {
                    phase: GuestEffectPhase::Pending,
                    resource_projection: outcome.resource_projection,
                }
            } else {
                outcome
            }
        };

        let projected = outcome
            .resource_projection
            .clone()
            .or_else(|| {
                // The Provider status is sticky: a pass that published no new
                // status keeps the last one, exactly as the old durable row
                // did.
                ctx.status::<GuestDriverStatus>()
                    .and_then(|status| status.resource.clone())
            });
        ctx.set_status(GuestDriverStatus {
            phase: outcome.phase,
            resource: projected.clone(),
        });
        if let Some(projection) = projected {
            ctx.set_status_projection(projection);
        }
        if outcome.phase != GuestEffectPhase::Ready || mutated {
            ctx.requeue_after(kind.resync());
        }
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown, and before the Provider's teardown stage in
    /// [`ResourceDriver::delete`]. Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        let Ok(envelope) = self.envelope(ctx, op) else {
            return Ok(());
        };
        let kind = self.kind(ctx, &envelope, op)?;
        if kind.children_owned_by_controller() {
            // The Cloud Hypervisor controller session owns its children's
            // teardown ordering; the drain step has nothing to nudge.
            return Ok(());
        }
        ctx.finalize_owned_resources().await.map_err(|_| {
            self.error(GuestDriverErrorKind::FinalizePending, DriverOp::Delete)
        })?;
        Ok(())
    }

    /// Teardown (old `prepare_finalize` + `execute_finalize` + `finalize`):
    /// the Provider's teardown stage runs first, then the owned children
    /// retire. Idempotent under retry (R10).
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        let Ok(envelope) = self.envelope(ctx, op) else {
            return Ok(());
        };
        let kind = self.kind(ctx, &envelope, op)?;
        let metadata = self.metadata(ctx, op)?;
        let provider_spec = self.provider_spec(ctx, kind, op).await?;
        let operation_id = self.operation_id(ctx, kind);
        let target = key_ref(ctx.key());
        let key = ctx.key().clone();
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(GuestDriverErrorKind::SpecInvalid, op))?;
        let generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(GuestDriverErrorKind::SpecInvalid, op))?;
        let status = ctx
            .status::<GuestDriverStatus>()
            .and_then(|status| status.resource.clone());
        let status_sink = guest_status_sink();
        let stage = {
            let surface = ContextChildSurface::new(ctx);
            let request = GuestEffectRequest {
                zone: self.zone.clone(),
                target,
                key,
                uid,
                generation,
                controller_generation: self.controller_generation,
                operation_id,
                spec: envelope.value().clone(),
                metadata,
                provider_spec,
                status,
                children: &surface,
                status_sink,
            };
            self.effects
                .finalize(kind, &request)
                .await
                .map_err(|error| self.effect_error(error, op))?
        };
        if stage == GuestFinalizeStage::Pending {
            return Err(self.error(GuestDriverErrorKind::FinalizePending, op));
        }
        if kind.children_owned_by_controller() {
            // The controller session retires its own child rows in the
            // preserved ordering (ch-api -> guest-control -> vmm -> system).
            return Ok(());
        }
        self.retire_obsolete_children(ctx, &[], op).await?;
        Ok(())
    }
}

/// The resource reference of one manager key.
pub(crate) fn key_ref(key: &ResourceKey) -> ResourceRef {
    ResourceRef::parse(&format!("{}/{}", key.type_name, key.name))
        .expect("manager keys carry canonical resource references")
}

/// The child row key a `ChildEnsure` derives (manager identity rules).
fn owned_child_matches_child_ensure(row: &StoredDesiredResource, child: &ChildEnsure) -> bool {
    row.key.type_name == child.type_name.as_str() && row.key.name == child.name
}

/// Old `BindingChildKind` teardown ranks: endpoints retire before their
/// producing processes.
fn teardown_rank(resource_type: &str) -> u8 {
    match resource_type {
        "Endpoint" => 0,
        "EphemeralProcess" => 1,
        _ => 2,
    }
}

/// Every resource reference one Guest spec declares, in document order: the
/// old descriptor's dependency set (the Runner watched each converted type
/// and the effect gated on any dependency reference the spec mentions).
pub(crate) fn declared_dependency_refs(spec: &Value) -> Vec<ResourceRef> {
    fn walk(value: &Value, refs: &mut Vec<ResourceRef>) {
        match value {
            Value::String(value) => {
                if let Ok(reference) = ResourceRef::parse(value) {
                    if !refs.contains(&reference) {
                        refs.push(reference);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    walk(value, refs);
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    walk(value, refs);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    let mut refs = Vec::new();
    walk(spec, &mut refs);
    refs
}

/// One authored child row from the driver's own spec construction: the
/// canonical envelope metadata the old effects rendered.
fn child_metadata(owner: &ResourceRef) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "ownerRef": owner.to_canonical_string(),
        "labels": {},
        "annotations": {},
    }))
    .expect("child metadata renders")
}

/// The qemu-media kind's desired children (old `qemu_guest_children`,
/// unchanged): the runtime Volume and the VMM Process that consumes it.
fn qemu_child_ensures(
    spec: &Value,
    provider: &Value,
    owner: &ResourceRef,
    op: DriverOp,
) -> Result<Vec<ChildEnsure>, GuestDriverError> {
    let error = |kind: GuestDriverErrorKind| GuestDriverError::new(kind, op);
    let config = serde_json::from_value::<d2b_provider_runtime_qemu_media::ProviderConfig>(
        provider
            .pointer("/config")
            .cloned()
            .ok_or_else(|| error(GuestDriverErrorKind::ProviderUnavailable))?,
    )
    .map_err(|_| error(GuestDriverErrorKind::SpecInvalid))?;
    let volume_ref = ResourceRef::parse(&format!("Volume/{}-runtime", owner.name().as_str()))
        .map_err(|_| error(GuestDriverErrorKind::SpecInvalid))?;
    let device_ref = spec
        .pointer("/deviceAttachments")
        .and_then(Value::as_array)
        .and_then(|attachments| attachments.first())
        .and_then(|attachment| attachment.get("deviceRef"))
        .and_then(Value::as_str)
        .map(ResourceRef::parse)
        .transpose()
        .map_err(|_| error(GuestDriverErrorKind::SpecInvalid))?;
    let network_refs = spec
        .pointer("/networkAttachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|attachment| attachment.get("networkRef").and_then(Value::as_str))
        .map(ResourceRef::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| error(GuestDriverErrorKind::SpecInvalid))?;
    let process = d2b_provider_runtime_qemu_media::build_process_spec(
        config.controller_execution_ref.clone(),
        volume_ref.clone(),
        device_ref,
        network_refs,
    )
    .map_err(|_| error(GuestDriverErrorKind::SpecInvalid))?;
    let mut process_spec =
        serde_json::to_value(process).map_err(|_| error(GuestDriverErrorKind::SpecInvalid))?;
    process_spec
        .as_object_mut()
        .ok_or_else(|| error(GuestDriverErrorKind::SpecInvalid))?
        .insert(
            "providerRef".to_owned(),
            Value::String("Provider/system-minijail".to_owned()),
        );
    let process_ref = ResourceRef::parse(&format!("Process/{}-qemu", owner.name().as_str()))
        .map_err(|_| error(GuestDriverErrorKind::SpecInvalid))?;
    let volume_spec = json!({
        "providerRef": "Provider/volume-local",
        "source": {
            "executionRef": config.controller_execution_ref.to_canonical_string(),
            "settings": {"kind": "tmpfs"}
        },
        "kind": "ephemeral",
        "layout": [],
        "views": {
            "runner": {
                "path": "",
                "rights": ["read", "write", "create", "delete", "traverse"]
            }
        },
        "attachments": [],
        "quota": {
            "maxBytes": config.runtime_tmpfs_quota_bytes,
            "maxInodes": config.runtime_tmpfs_quota_inodes,
            "enforcement": "hard"
        }
    });
    Ok(vec![
        ChildEnsure {
            type_name: ResourceTypeName::new("Volume"),
            name: volume_ref.name().as_str().to_owned(),
            spec: serde_json::to_vec(&volume_spec)
                .map_err(|_| error(GuestDriverErrorKind::SpecInvalid))?,
            metadata: child_metadata(owner),
        },
        ChildEnsure {
            type_name: ResourceTypeName::new("Process"),
            name: process_ref.name().as_str().to_owned(),
            spec: serde_json::to_vec(&process_spec)
                .map_err(|_| error(GuestDriverErrorKind::SpecInvalid))?,
            metadata: child_metadata(owner),
        },
    ])
}

/// The azure-container-apps kind's desired child (old `aca_guest_children`,
/// unchanged): the sandbox-agent control Endpoint.
fn aca_child_ensures(
    owner: &ResourceRef,
    op: DriverOp,
) -> Result<Vec<ChildEnsure>, GuestDriverError> {
    let error = |kind: GuestDriverErrorKind| GuestDriverError::new(kind, op);
    let target = ResourceRef::parse(&format!("Endpoint/{}-sandbox-agent", owner.name().as_str()))
        .map_err(|_| error(GuestDriverErrorKind::SpecInvalid))?;
    let spec = json!({
        "providerRef": d2b_provider_runtime_azure_container_apps::PROVIDER_REF,
        "producerRef": owner.to_canonical_string(),
        "endpointClass": "control",
        "transport": "opaque-carriage",
        "purpose": "aca-sandbox-agent",
        "locality": "cross-domain",
        "visibility": "provider",
        "attachmentPolicy": {
            "supported": false,
            "maxAttachments": 0
        },
        "consumerPolicy": {
            "allowedSubjects": [d2b_provider_runtime_azure_container_apps::PROVIDER_REF],
            "allowedOperations": ["resolve"]
        },
        "lifecyclePolicy": "recycle-with-producer"
    });
    Ok(vec![ChildEnsure {
        type_name: ResourceTypeName::new("Endpoint"),
        name: target.name().as_str().to_owned(),
        spec: serde_json::to_vec(&spec).map_err(|_| error(GuestDriverErrorKind::SpecInvalid))?,
        metadata: child_metadata(owner),
    }])
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_contracts_resource::v3::ControllerGeneration;
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, WatchId,
        WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::manager::ResourceView;
    use d2b_resource_runtime::resource::ResourceStatus;
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;

    use super::{
        GUEST_REGISTRATIONS, GUEST_TYPE_NAME, GuestDriver, GuestDriverArgs, GuestDriverEffects,
        GuestDriverFactory, GuestDriverStatus, GuestEffectError, GuestEffectOutcome,
        GuestEffectPhase, GuestEffectRequest, GuestFinalizeStage, GuestKind, guest_spec_decoder,
        view_phase,
    };

    /// The Guest row uid `[0x11; …]`, UUIDv4-shaped once mapped.
    const GUEST_UID_BYTES: [u8; 16] = [
        0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x41, 0x11, 0x81, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
        0x11,
    ];
    const CHILD_UID_BYTES: [u8; 16] = [
        0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x42, 0x22, 0x82, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
        0x22,
    ];

    // -- fakes ---------------------------------------------------------------

    /// One `reconcile` call as the scripted effect observed it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct EffectObservation {
        kind: GuestKind,
        provider_spec: Option<serde_json::Value>,
        status: Option<serde_json::Value>,
        children: Vec<(String, String, bool)>,
    }

    /// Scripted Provider effect port: records every call and answers with the
    /// configured outcome.
    struct ScriptedEffects {
        calls: parking_lot::Mutex<Vec<String>>,
        /// Optional shared order log (the recording manager's), so tests can
        /// compare the provider stage against the child mutations.
        shared: Option<Arc<parking_lot::Mutex<Vec<String>>>>,
        observations: parking_lot::Mutex<Vec<EffectObservation>>,
        phase: parking_lot::Mutex<GuestEffectPhase>,
        projection: parking_lot::Mutex<Option<serde_json::Value>>,
        finalize: parking_lot::Mutex<GuestFinalizeStage>,
    }

    impl ScriptedEffects {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: parking_lot::Mutex::new(Vec::new()),
                shared: None,
                observations: parking_lot::Mutex::new(Vec::new()),
                phase: parking_lot::Mutex::new(GuestEffectPhase::Ready),
                projection: parking_lot::Mutex::new(None),
                finalize: parking_lot::Mutex::new(GuestFinalizeStage::Complete),
            })
        }

        fn with_shared_log(log: Arc<parking_lot::Mutex<Vec<String>>>) -> Arc<Self> {
            let mut effects = Arc::into_inner(Self::new()).expect("fresh effects");
            effects.shared = Some(log);
            Arc::new(effects)
        }

        fn record(&self, entry: String) {
            if let Some(shared) = &self.shared {
                shared.lock().push(entry.clone());
            }
            self.calls.lock().push(entry);
        }

        fn set_phase(&self, phase: GuestEffectPhase) {
            *self.phase.lock() = phase;
        }

        fn set_projection(&self, projection: Option<serde_json::Value>) {
            *self.projection.lock() = projection;
        }

        fn set_finalize(&self, stage: GuestFinalizeStage) {
            *self.finalize.lock() = stage;
        }

        fn call_order(&self) -> Vec<String> {
            self.calls.lock().clone()
        }

        fn observations(&self) -> Vec<EffectObservation> {
            self.observations.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl GuestDriverEffects for ScriptedEffects {
        async fn reconcile(
            &self,
            kind: GuestKind,
            request: &GuestEffectRequest<'_>,
        ) -> Result<GuestEffectOutcome, GuestEffectError> {
            self.record(format!("reconcile:{}", kind.effect_id()));
            let children = request.children.owned().await?;
            self.observations.lock().push(EffectObservation {
                kind,
                provider_spec: request.provider_spec.clone(),
                status: request.status.clone(),
                children: children
                    .iter()
                    .map(|child| {
                        (
                            child.key.type_name.clone(),
                            child.key.name.clone(),
                            child.ready(),
                        )
                    })
                    .collect(),
            });
            Ok(GuestEffectOutcome {
                phase: *self.phase.lock(),
                resource_projection: self.projection.lock().clone(),
            })
        }

        async fn finalize(
            &self,
            kind: GuestKind,
            _request: &GuestEffectRequest<'_>,
        ) -> Result<GuestFinalizeStage, GuestEffectError> {
            self.record(format!("finalize:{}", kind.effect_id()));
            Ok(*self.finalize.lock())
        }
    }

    /// Recording manager over a scripted row/view set. `ensure_child` commits
    /// (or keeps) the child row and its live view; the child's published
    /// phase is the `children_ready` switch.
    struct RecordingManager {
        calls: Arc<parking_lot::Mutex<Vec<String>>>,
        rows: parking_lot::Mutex<Vec<StoredDesiredResource>>,
        views: parking_lot::Mutex<Vec<(ResourceKey, ResourceView)>>,
        children_ready: std::sync::atomic::AtomicBool,
        fail_reads: std::sync::atomic::AtomicBool,
    }

    impl RecordingManager {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Arc::new(parking_lot::Mutex::new(Vec::new())),
                rows: parking_lot::Mutex::new(Vec::new()),
                views: parking_lot::Mutex::new(Vec::new()),
                children_ready: std::sync::atomic::AtomicBool::new(false),
                fail_reads: std::sync::atomic::AtomicBool::new(false),
            })
        }

        /// The shared order log: effect calls that were constructed over it
        /// append to the same sequence.
        fn log_handle(&self) -> Arc<parking_lot::Mutex<Vec<String>>> {
            Arc::clone(&self.calls)
        }

        fn set_children_ready(&self, ready: bool) {
            self.children_ready.store(ready, std::sync::atomic::Ordering::SeqCst);
        }

        /// Make every read answer `ManagerRpc` (the unanswerable plane).
        fn set_fail_reads(&self, fail: bool) {
            self.fail_reads.store(fail, std::sync::atomic::Ordering::SeqCst);
        }

        fn add(&self, row: StoredDesiredResource, status: ResourceStatus) {
            let view = ResourceView {
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
            };
            self.rows.lock().push(row);
            self.views.lock().push((view.key.clone(), view));
        }

        fn drop_row(&self, key: &ResourceKey) {
            self.rows.lock().retain(|row| row.key != *key);
            self.views.lock().retain(|(view_key, _)| view_key != key);
        }

        fn call_order(&self) -> Vec<String> {
            self.calls.lock().clone()
        }

        fn ensure_order(&self) -> Vec<String> {
            self.calls
                .lock()
                .iter()
                .filter(|call| call.starts_with("ensure:"))
                .cloned()
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            self.calls
                .lock()
                .push(format!("ensure:{}/{}", child.type_name.as_str(), child.name));
            let key = ResourceKey::new(
                parent.zone.clone(),
                child.type_name.as_str(),
                child.name.as_str(),
            );
            let mut rows = self.rows.lock();
            if let Some(existing) = rows.iter().find(|row| row.key == key).cloned() {
                if existing.spec == child.spec {
                    return Ok(EnsureOutcome::Unchanged(existing));
                }
                rows.retain(|row| row.key != key);
                let mut updated = existing;
                updated.spec = child.spec.clone();
                updated.metadata = child.metadata.clone();
                updated.generation += 1;
                rows.push(updated.clone());
                drop(rows);
                self.views.lock().retain(|(view_key, _)| view_key != &key);
                self.views.lock().push((
                    key,
                    ResourceView {
                        key: updated.key.clone(),
                        uid: updated.uid,
                        generation: updated.generation,
                        deleting: false,
                        provenance: updated.provenance,
                        spec: updated.spec.clone(),
                        metadata: updated.metadata.clone(),
                        owner_key: None,
                        status: Some(if self.children_ready.load(std::sync::atomic::Ordering::SeqCst) {
                            ResourceStatus::Ready
                        } else {
                            ResourceStatus::Pending
                        }),
                        status_generation: Some(updated.generation),
                        status_projection: None,
                    },
                ));
                return Ok(EnsureOutcome::Updated(updated));
            }
            let row = StoredDesiredResource {
                key: key.clone(),
                uid: CHILD_UID_BYTES,
                generation: 1,
                owner_uid: Some(GUEST_UID_BYTES),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: child.spec,
                metadata: child.metadata,
                created_at: 0,
            };
            rows.push(row.clone());
            drop(rows);
            self.views.lock().push((
                key,
                ResourceView {
                    key: row.key.clone(),
                    uid: row.uid,
                    generation: row.generation,
                    deleting: false,
                    provenance: row.provenance,
                    spec: row.spec.clone(),
                    metadata: row.metadata.clone(),
                    owner_key: None,
                    status: Some(if self.children_ready.load(std::sync::atomic::Ordering::SeqCst) {
                        ResourceStatus::Ready
                    } else {
                        ResourceStatus::Pending
                    }),
                    status_generation: Some(row.generation),
                    status_projection: None,
                },
            ));
            Ok(EnsureOutcome::Created(row))
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            self.calls.lock().push(format!("get:{}/{}", key.type_name, key.name));
            if self.fail_reads.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ResourceError::ManagerRpc("scripted read failure".into()));
            }
            Ok(self
                .rows
                .lock()
                .iter()
                .find(|row| row.key == *key)
                .cloned())
        }

        async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
            self.calls.lock().push(format!("view:{}/{}", key.type_name, key.name));
            if self.fail_reads.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ResourceError::ManagerRpc("scripted read failure".into()));
            }
            Ok(self
                .views
                .lock()
                .iter()
                .find(|(view_key, _)| view_key == key)
                .map(|(_, view)| view.clone()))
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.calls.lock().push(format!("delete:{}/{}", key.type_name, key.name));
            self.drop_row(key);
            Ok(())
        }

        async fn list_owned(
            &self,
            owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            self.calls.lock().push("list-owned".to_owned());
            if self.fail_reads.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ResourceError::ManagerRpc("scripted read failure".into()));
            }
            Ok(self
                .rows
                .lock()
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
            self.calls.lock().push(format!(
                "watch:{}/{}",
                registration.target.type_name, registration.target.name
            ));
            Ok(WatchId(1))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            self.calls.lock().push("cancel-watch".to_owned());
            Ok(())
        }
    }

    struct RecordingRequeue {
        scheduled: parking_lot::Mutex<Vec<(ResourceKey, std::time::Duration)>>,
    }

    impl RecordingRequeue {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                scheduled: parking_lot::Mutex::new(Vec::new()),
            })
        }

        fn count(&self) -> usize {
            self.scheduled.lock().len()
        }
    }

    impl RequeueScheduler for RecordingRequeue {
        fn schedule(&self, key: ResourceKey, after: std::time::Duration) -> RequeueId {
            self.scheduled.lock().push((key, after));
            RequeueId(0)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    // -- fixtures ------------------------------------------------------------

    fn spec_bytes(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).expect("spec bytes")
    }

    fn metadata_bytes() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "ownerRef": "Zone/work" })).expect("metadata")
    }

    fn guest_row(name: &str, spec: serde_json::Value) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", GUEST_TYPE_NAME, name),
            uid: GUEST_UID_BYTES,
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting: false,
            spec: spec_bytes(spec),
            metadata: metadata_bytes(),
            created_at: 0,
        }
    }

    fn provider_row(name: &str, spec: serde_json::Value) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", "Provider", name),
            uid: CHILD_UID_BYTES,
            generation: 3,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting: false,
            spec: spec_bytes(spec),
            metadata: metadata_bytes(),
            created_at: 0,
        }
    }

    fn owned_row(type_name: &str, name: &str, spec: serde_json::Value) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", type_name, name),
            uid: CHILD_UID_BYTES,
            generation: 1,
            owner_uid: Some(GUEST_UID_BYTES),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: spec_bytes(spec),
            metadata: metadata_bytes(),
            created_at: 0,
        }
    }

    fn context(
        target: StoredDesiredResource,
        manager: Arc<RecordingManager>,
        requeue: Arc<RecordingRequeue>,
    ) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            target,
            TargetHandle::Host,
            guest_spec_decoder(),
            manager,
            requeue,
            effects_tx,
            notify_tx,
        )
    }

    fn driver(effects: Arc<ScriptedEffects>) -> GuestDriver {
        GuestDriver::new(GuestDriverArgs {
            zone: "work".to_owned(),
            controller_generation: ControllerGeneration::new(3).expect("generation"),
            effects,
        })
    }

    fn guest_status(ctx: &ResourceContext) -> GuestDriverStatus {
        ctx.status::<GuestDriverStatus>()
            .cloned()
            .expect("status published")
    }

    fn qemu_guest_spec() -> serde_json::Value {
        serde_json::json!({
            "providerRef": "Provider/runtime-qemu-media",
            "deviceAttachments": [{ "deviceRef": "Device/host-kvm" }],
            "networkAttachments": [],
        })
    }

    fn qemu_provider_spec() -> serde_json::Value {
        serde_json::json!({
            "config": serde_json::to_value(
                d2b_provider_runtime_qemu_media::ProviderConfig::new(
                    "Host/host-system",
                    "qemu-system-x86-64",
                    "Provider/network-local",
                    "Provider/volume-local",
                    None,
                )
                .expect("qemu provider config"),
            )
            .expect("provider config document"),
        })
    }

    fn qemu_fixture() -> (ResourceContext, Arc<ScriptedEffects>, Arc<RecordingManager>) {
        let effects = ScriptedEffects::new();
        let manager = RecordingManager::new();
        manager.add(
            provider_row("runtime-qemu-media", qemu_provider_spec()),
            ResourceStatus::Ready,
        );
        let ctx = context(
            guest_row("work-vm", qemu_guest_spec()),
            Arc::clone(&manager),
            RecordingRequeue::new(),
        );
        (ctx, effects, manager)
    }

    // -- registration --------------------------------------------------------

    #[test]
    fn factory_registers_the_guest_type() {
        let factory = GuestDriverFactory::new(GuestDriverArgs {
            zone: "work".to_owned(),
            controller_generation: ControllerGeneration::new(1).expect("generation"),
            effects: Arc::new(super::UnavailableGuestDriverEffects),
        });
        assert_eq!(factory.resource_types().len(), 1);
        assert_eq!(factory.resource_types()[0].as_str(), GUEST_TYPE_NAME);
    }

    /// The table is the family fence: four Provider rows, one kind each,
    /// Provider-scoped resolution, and everything else refused closed.
    #[test]
    fn registrations_are_closed_and_provider_scoped() {
        assert_eq!(GUEST_REGISTRATIONS.len(), 4);
        let mut providers = std::collections::BTreeSet::new();
        let mut controllers = std::collections::BTreeSet::new();
        for (index, registration) in GUEST_REGISTRATIONS.iter().enumerate() {
            assert_eq!(registration.kind.index(), index);
            assert_eq!(registration.kind.registration(), *registration);
            assert_eq!(registration.kind.provider_ref(), registration.provider_ref);
            assert_eq!(registration.kind.controller_ref(), registration.controller_ref);
            assert!(providers.insert(registration.provider_ref));
            assert!(controllers.insert(registration.controller_ref));
            assert_eq!(
                GuestKind::from_type_and_provider(GUEST_TYPE_NAME, Some(registration.provider_ref)),
                Ok(registration.kind),
            );
        }
        assert_eq!(
            GuestKind::from_type_and_provider("Process", Some(GUEST_REGISTRATIONS[0].provider_ref)),
            Err(GuestEffectError::InvalidResource),
        );
        assert_eq!(
            GuestKind::from_type_and_provider(GUEST_TYPE_NAME, None),
            Err(GuestEffectError::InvalidResource),
        );
        assert_eq!(
            GuestKind::from_type_and_provider(GUEST_TYPE_NAME, Some("Provider/not-a-runtime")),
            Err(GuestEffectError::InvalidResource),
        );
    }

    // -- validate ------------------------------------------------------------

    #[tokio::test]
    async fn validate_admits_every_registered_provider() {
        for registration in GUEST_REGISTRATIONS {
            let manager = RecordingManager::new();
            let mut ctx = context(
                guest_row(
                    "work-vm",
                    serde_json::json!({ "providerRef": registration.provider_ref }),
                ),
                Arc::clone(&manager),
                RecordingRequeue::new(),
            );
            let mut driver = driver(ScriptedEffects::new());
            driver.validate(&mut ctx).await.expect("registered provider");
        }
    }

    #[tokio::test]
    async fn validate_refuses_an_unregistered_provider_as_terminal() {
        let manager = RecordingManager::new();
        let mut ctx = context(
            guest_row(
                "work-vm",
                serde_json::json!({ "providerRef": "Provider/not-a-runtime" }),
            ),
            Arc::clone(&manager),
            RecordingRequeue::new(),
        );
        let mut driver = driver(ScriptedEffects::new());
        let failure = driver.validate(&mut ctx).await.expect_err("refused provider");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Validate);
        assert_eq!(format!("{failure}"), "guest-spec-invalid");
    }

    // -- reconcile -----------------------------------------------------------

    /// The qemu kind's authored child graph is committed to the manager
    /// before the effect runs, and the Provider's status is published on the
    /// row's in-memory status with its projection.
    #[tokio::test]
    async fn reconcile_ensures_the_qemu_child_graph_and_publishes_the_status() {
        let (mut ctx, effects, manager) = qemu_fixture();
        manager.set_children_ready(true);
        let projection = serde_json::json!({ "phase": "Ready", "runtimeReady": true });
        effects.set_projection(Some(projection.clone()));
        let mut driver = driver(Arc::clone(&effects));

        let outcome = driver.reconcile(&mut ctx).await.expect("reconcile");
        assert_eq!(outcome, ReconcileOutcome::Satisfied);
        assert_eq!(
            manager.ensure_order(),
            vec![
                "ensure:Volume/work-vm-runtime".to_owned(),
                "ensure:Process/work-vm-qemu".to_owned(),
            ],
            "the qemu child graph is the runtime Volume then the VMM Process",
        );
        let observation = effects.observations().pop().expect("effect call");
        assert_eq!(observation.kind, GuestKind::QemuMedia);
        assert_eq!(observation.provider_spec, Some(qemu_provider_spec()));
        assert_eq!(
            observation.children,
            vec![
                ("Volume".to_owned(), "work-vm-runtime".to_owned(), true),
                ("Process".to_owned(), "work-vm-qemu".to_owned(), true),
            ],
        );
        let status = guest_status(&ctx);
        assert_eq!(status.phase, GuestEffectPhase::Ready);
        assert_eq!(status.resource, Some(projection.clone()));
        assert_eq!(ctx.take_status_projection(), Some(projection));
    }

    /// The old effect gate: a Provider that reports Ready ahead of its own
    /// children keeps the Guest Pending until every desired child is live.
    #[tokio::test]
    async fn reconcile_pends_the_guest_until_its_children_converge() {
        let (mut ctx, effects, manager) = qemu_fixture();
        manager.set_children_ready(false);
        let mut driver = driver(Arc::clone(&effects));

        driver.reconcile(&mut ctx).await.expect("reconcile");
        assert_eq!(guest_status(&ctx).phase, GuestEffectPhase::Pending);
        assert_eq!(effects.call_order(), vec!["reconcile:runtime-qemu-media-guest".to_owned()]);
    }

    /// A qemu Guest whose Provider row the manager does not hold refuses
    /// closed (the old fence) and never reaches the effect.
    #[tokio::test]
    async fn reconcile_refuses_a_qemu_guest_without_its_provider_row() {
        let effects = ScriptedEffects::new();
        let manager = RecordingManager::new();
        let mut ctx = context(
            guest_row("work-vm", qemu_guest_spec()),
            Arc::clone(&manager),
            RecordingRequeue::new(),
        );
        let mut driver = driver(Arc::clone(&effects));

        let failure = driver
            .reconcile(&mut ctx)
            .await
            .expect_err("unfenced provider row");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(format!("{failure}"), "guest-provider-unavailable");
        assert!(effects.call_order().is_empty());
        assert!(manager.ensure_order().is_empty());
    }

    /// Issue #511 at the migrated provider-row read
    /// ([`GuestDriver::provider_spec`], classified): an absent row and
    /// an unanswerable manager both defer (retryable - the actor requeues),
    /// while a present row that cannot be decoded names its terminal evidence
    /// (the closed `guest-spec-invalid` refusal) instead of deferring
    /// forever.
    #[tokio::test]
    async fn classified_provider_row_read_defers_absence_and_requires_terminal_evidence() {
        // Absent: the manager answers that it holds no provider row.
        let effects = ScriptedEffects::new();
        let manager = RecordingManager::new();
        let mut ctx = context(
            guest_row("work-vm", qemu_guest_spec()),
            Arc::clone(&manager),
            RecordingRequeue::new(),
        );
        let mut driver = driver(Arc::clone(&effects));
        let failure = driver
            .reconcile(&mut ctx)
            .await
            .expect_err("absent provider row");
        assert_eq!(failure.class(), FailureClass::Retryable, "absence defers");
        assert_eq!(format!("{failure}"), "guest-provider-unavailable");
        assert!(effects.call_order().is_empty(), "absence never reaches the effect");

        // Unavailable: the manager cannot answer - the same defer, and never
        // reported as absence.
        manager.set_fail_reads(true);
        let failure = driver
            .reconcile(&mut ctx)
            .await
            .expect_err("unanswerable manager");
        assert_eq!(failure.class(), FailureClass::Retryable, "an unanswered plane defers");
        assert_eq!(format!("{failure}"), "guest-provider-unavailable");
        manager.set_fail_reads(false);

        // Present but undecodable: terminal on named evidence, not a deferral.
        let mut row = provider_row("runtime-qemu-media", qemu_provider_spec());
        row.spec = b"{ not json".to_vec();
        manager.add(row, ResourceStatus::Ready);
        let failure = driver
            .reconcile(&mut ctx)
            .await
            .expect_err("undecodable provider row");
        assert_eq!(
            failure.class(),
            FailureClass::Terminal,
            "a committed row that cannot decode is the named terminal evidence"
        );
        assert_eq!(format!("{failure}"), "guest-spec-invalid");
    }

    /// Dependency edges of the spec are registered exactly once (R12).
    #[tokio::test]
    async fn reconcile_watches_declared_dependencies_once() {
        let (mut ctx, _effects, manager) = qemu_fixture();
        manager.set_children_ready(true);
        let mut driver = driver(ScriptedEffects::new());

        driver.reconcile(&mut ctx).await.expect("first pass");
        driver.reconcile(&mut ctx).await.expect("second pass");
        let watches = manager
            .call_order()
            .into_iter()
            .filter(|call| call.starts_with("watch:"))
            .collect::<Vec<_>>();
        assert_eq!(
            watches,
            vec![
                "watch:Device/host-kvm".to_owned(),
                "watch:Provider/runtime-qemu-media".to_owned(),
                "watch:Volume/work-vm-runtime".to_owned(),
                "watch:Process/work-vm-qemu".to_owned(),
            ],
            "each declared dependency and desired child is watched exactly once",
        );
    }

    /// The Cloud Hypervisor kind commits nothing through the driver: its
    /// child rows are the controller session's (the manager route).
    #[tokio::test]
    async fn cloud_hypervisor_guests_commit_no_children_through_the_driver() {
        let effects = ScriptedEffects::new();
        effects.set_projection(Some(serde_json::json!({ "phase": "Ready" })));
        let manager = RecordingManager::new();
        let mut ctx = context(
            guest_row(
                "acceptance-guest",
                serde_json::json!({ "providerRef": "Provider/runtime-cloud-hypervisor" }),
            ),
            Arc::clone(&manager),
            RecordingRequeue::new(),
        );
        let mut driver = driver(Arc::clone(&effects));

        driver.reconcile(&mut ctx).await.expect("reconcile");
        assert_eq!(
            effects.observations().pop().expect("effect call").kind,
            GuestKind::CloudHypervisor,
        );
        assert!(manager.ensure_order().is_empty());
        assert!(manager.call_order().iter().all(|call| !call.starts_with("delete:")));
        assert_eq!(guest_status(&ctx).phase, GuestEffectPhase::Ready);
    }

    // -- recover -------------------------------------------------------------

    /// Adoption for the Cloud Hypervisor kind is the committed VMM child row
    /// (the controller session's evidence), not a driver-owned child set.
    #[tokio::test]
    async fn recover_adopts_a_cloud_hypervisor_guest_with_a_live_vmm_child() {
        let manager = RecordingManager::new();
        let mut ctx = context(
            guest_row(
                "acceptance-guest",
                serde_json::json!({ "providerRef": "Provider/runtime-cloud-hypervisor" }),
            ),
            Arc::clone(&manager),
            RecordingRequeue::new(),
        );
        let mut driver = driver(ScriptedEffects::new());
        assert_eq!(
            driver.recover(&mut ctx).await.expect("recover"),
            RecoveryOutcome::Missing,
        );

        manager.add(
            owned_row(
                "Process",
                "acceptance-guest-vmm",
                serde_json::json!({ "processClass": "worker" }),
            ),
            ResourceStatus::Ready,
        );
        assert_eq!(
            driver.recover(&mut ctx).await.expect("recover"),
            RecoveryOutcome::Adopted,
        );
    }

    #[tokio::test]
    async fn recover_adopts_a_qemu_guest_with_its_complete_child_set() {
        let (mut ctx, _effects, manager) = qemu_fixture();
        let mut driver = driver(ScriptedEffects::new());
        assert_eq!(
            driver.recover(&mut ctx).await.expect("recover"),
            RecoveryOutcome::Missing,
        );
        manager.add(
            owned_row("Volume", "work-vm-runtime", serde_json::json!({})),
            ResourceStatus::Ready,
        );
        manager.add(
            owned_row("Process", "work-vm-qemu", serde_json::json!({})),
            ResourceStatus::Ready,
        );
        assert_eq!(
            driver.recover(&mut ctx).await.expect("recover"),
            RecoveryOutcome::Adopted,
        );
    }

    // -- finalize / delete ---------------------------------------------------

    #[tokio::test]
    async fn finalize_blocks_while_an_owned_child_is_live() {
        let (mut ctx, _effects, manager) = qemu_fixture();
        manager.add(
            owned_row("Volume", "work-vm-runtime", serde_json::json!({})),
            ResourceStatus::Ready,
        );
        let mut driver = driver(ScriptedEffects::new());

        let failure = driver.finalize(&mut ctx).await.expect_err("child live");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.op(), DriverOp::Delete);
        assert_eq!(format!("{failure}"), "guest-finalize-pending");

        manager.drop_row(&ResourceKey::new("work", "Volume", "work-vm-runtime"));
        driver.finalize(&mut ctx).await.expect("children retired");
    }

    /// Delete runs the Provider's teardown stage first, then retires the
    /// children the desired set no longer derives (R10 order).
    #[tokio::test]
    async fn delete_runs_the_provider_stage_before_retiring_the_children() {
        let manager = RecordingManager::new();
        let effects = ScriptedEffects::with_shared_log(manager.log_handle());
        manager.add(
            provider_row("runtime-qemu-media", qemu_provider_spec()),
            ResourceStatus::Ready,
        );
        manager.add(
            owned_row("Volume", "work-vm-stale", serde_json::json!({})),
            ResourceStatus::Ready,
        );
        let mut ctx = context(
            guest_row("work-vm", qemu_guest_spec()),
            Arc::clone(&manager),
            RecordingRequeue::new(),
        );
        let mut driver = driver(Arc::clone(&effects));

        driver.delete(&mut ctx).await.expect("teardown");
        let order = manager.call_order();
        let finalize_at = order
            .iter()
            .position(|call| call == "finalize:runtime-qemu-media-guest")
            .expect("provider stage ran");
        let delete_at = order
            .iter()
            .position(|call| call == "delete:Volume/work-vm-stale")
            .expect("stale child retired");
        assert!(
            finalize_at < delete_at,
            "the provider stage precedes child retirement: {order:?}",
        );
    }

    #[tokio::test]
    async fn delete_refuses_to_retire_children_while_the_provider_stage_pends() {
        let (mut ctx, effects, manager) = qemu_fixture();
        manager.add(
            owned_row("Volume", "work-vm-stale", serde_json::json!({})),
            ResourceStatus::Ready,
        );
        effects.set_finalize(GuestFinalizeStage::Pending);
        let mut driver = driver(Arc::clone(&effects));

        let failure = driver.delete(&mut ctx).await.expect_err("stage pending");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(format!("{failure}"), "guest-finalize-pending");
        assert!(manager
            .call_order()
            .iter()
            .all(|call| !call.starts_with("delete:")));
    }

    // -- status projection ---------------------------------------------------

    /// The provider projection is published (and taken) on every pass that
    /// carries one, and the last one stays sticky when a later pass does not.
    #[tokio::test]
    async fn status_projection_is_sticky_across_passes() {
        let (mut ctx, effects, manager) = qemu_fixture();
        manager.set_children_ready(true);
        let projection = serde_json::json!({ "phase": "Ready", "runtimeReady": true });
        effects.set_projection(Some(projection.clone()));
        let mut driver = driver(Arc::clone(&effects));
        driver.reconcile(&mut ctx).await.expect("first pass");
        assert_eq!(ctx.take_status_projection(), Some(projection.clone()));

        effects.set_projection(None);
        driver.reconcile(&mut ctx).await.expect("second pass");
        assert_eq!(ctx.take_status_projection(), Some(projection.clone()));
        let status = guest_status(&ctx);
        assert_eq!(status.phase, GuestEffectPhase::Ready);
        assert_eq!(status.resource, Some(projection));
    }

    // -- canonical phase (issue #515) ----------------------------------------

    /// One manager view carrying the given status classification, published
    /// generation, and durable deleting mark.
    fn phase_view(
        status: Option<ResourceStatus>,
        status_generation: Option<u64>,
        deleting: bool,
    ) -> ResourceView {
        ResourceView {
            key: ResourceKey::new("work", "Guest", "phase-view"),
            uid: [0x42; 16],
            generation: 2,
            deleting,
            provenance: ResourceProvenance::Api,
            spec: b"{}".to_vec(),
            metadata: b"{}".to_vec(),
            owner_key: None,
            status,
            status_generation,
            status_projection: None,
        }
    }

    /// Issue #515: `view_phase` delegates to the canonical wire producer.
    /// Across the closed status vocabulary - and across the runtime flags
    /// (the durable deleting mark, an ungenerationed status, a stale
    /// generation) - the gate's phase equals the phase
    /// `ResourceView::wire_status` serves, so a future divergence fails here.
    #[test]
    fn view_phase_delegates_to_the_canonical_wire_phase() {
        let statuses = [
            None,
            Some(ResourceStatus::Pending),
            Some(ResourceStatus::Recovering),
            Some(ResourceStatus::Reconciling),
            Some(ResourceStatus::Ready),
            Some(ResourceStatus::Failed(DriverFailure::retryable(DriverOp::Reconcile))),
            Some(ResourceStatus::Failed(DriverFailure::terminal(DriverOp::Delete))),
            Some(ResourceStatus::Deleting),
        ];
        for deleting in [false, true] {
            for status in &statuses {
                for status_generation in [None, Some(1), Some(2), Some(3)] {
                    let view = phase_view(status.clone(), status_generation, deleting);
                    let canonical = view.wire_status()["phase"]
                        .as_str()
                        .expect("the canonical status always carries a phase")
                        .to_owned();
                    assert_eq!(
                        view_phase(&view),
                        canonical,
                        "phase divergence: status={status:?} status_generation={status_generation:?} deleting={deleting}",
                    );
                }
            }
        }
    }
}
