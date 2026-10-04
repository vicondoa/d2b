//! The Process family's driver: the reconciliation of `Process` and
//! `EphemeralProcess` rows.
//!
//! One factory serves both family types: the durable `Process` and the
//! one-shot `EphemeralProcess`. The durable arm probes and classifies
//! adopt/missing/quarantine per the preserved `ProviderAdoption`
//! classification, launches through the signed provider-ticket path (which
//! stays behind the effect port), observes a live row through the liveness
//! probe so an exit leaves `Ready` and the restart policy runs on the
//! preserved 5s resync, refuses an identity the probe can no longer verify
//! terminally (the row never reads `Ready` over a process that cannot be
//! identified, and an ambiguous candidate is never adopted or signalled), and
//! deletes through the exact term-then-kill escalation with pidfd retry.
//!
//! The ephemeral arm preserves the one-shot lifecycle: the launch goes
//! through the Process Provider's ephemeral ticket (never a direct spawn), a
//! refused launch is terminal (the type carries no restart policy), the
//! bounded `runtimeDeadline` stops an over-running process and reports
//! `Failed`, an observed exit reports `Succeeded`, and the row then waits out
//! `successfulTtl`/`failedTtl` in runtime memory before asking the manager to
//! retire it. `incidentHold` keeps a failed row until an explicit release.
//!
//! Ticket inputs come from the factory's zone-authority wiring - the bundle
//! resolver and `ZoneAuthorityIdentity` path - never from the spec store. The
//! restart budget is runtime-only: no restart annotation is ever written to
//! the durable envelope.
//!
//! The effects the driver needs run through the typed seam declared in
//! [`crate::effects`], implemented by the family itself
//! ([`crate::effects_service`]) over the daemon-supplied facets
//! ([`crate::facets`]); the composition supplies the facet objects, so this
//! module holds no host state (U1).
#![allow(dead_code)]
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

use crate::effects::{
    BindingAuthorityLease, BindingDeliveryEvidence, BindingGateError, ExpectedBindingRow,
    ObservedBinding, ProcessBindingPreparation, ProcessDriverEffects, ProviderAdoption,
    ProviderLiveness, resolve_process_binding_preparation,
};
use crate::effects_service::{PROCESS_EFFECTS_SERVICE, ProcessEffectsService};
use crate::execution::{ExecutionMode, execution_target_allowed};
use crate::facets::ProcessEffectFacets;
use crate::identity::{ProcessFamilySpec, ProcessResourceIdentity};
use crate::launch_identity::{LaunchRow, resolve_launch_identity};
use crate::operations::process_family_operations;
use crate::worker_launch::{
    GuestBindingDelivery, GuestProcessRealization, ServingWorkerLaunch, ServingWorkerRoot,
};
use d2b_contracts_resource::v3::{
    AdoptionPolicy, ControllerGeneration, ENDPOINT_BINDING_RESOURCE_TYPE, EndpointBindingSpec,
    ResourceGeneration, ResourceName, ResourceRef, ResourceSpec,
    ResourceTypeName as ContractResourceTypeName, ResourceUid, ZoneId,
    process::{DesiredLifecycle, EphemeralProcessSpec, ProcessSpec, RestartClass},
};
use d2b_process_conformance::{
    AdoptionCandidate, BindingPreparation, GuestExecutionBinding, ProcessStatusReport,
};
use d2b_resource_runtime::guest_target::{GuestAdoption, TargetInstanceState};
use d2b_resource_runtime::context::{
    EffectCompleted, EffectResult, ResourceContext, RowLookup, SpecDecoder, WatchCondition,
    typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName, StoredDesiredResource};
use d2b_resource_runtime::manager::ResourceView;
use d2b_resource_runtime::resource::ResourceStatus;
use d2b_resource_runtime::target::{TargetBinding, TargetError, TargetObservation};
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, OperationDef, ServiceDecl,
    WellKnownType,
};

/// The durable Process resource type this factory serves (KTD4 Phase A).
pub(crate) const PROCESS_TYPE_NAME: &str = "Process";

/// The one-shot Process resource type this factory serves (KTD4 Phase A).
pub(crate) const EPHEMERAL_PROCESS_TYPE_NAME: &str = "EphemeralProcess";

/// Preserved launch budget for durable Process resources (old
/// `launch_timeout`).
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Preserved kill budget after the drain timeout elapses (old stop
/// escalation).
const KILL_TIMEOUT: Duration = Duration::from_secs(30);

/// Preserved one-shot stop escalation (old `ProcessResourceRuntime::
/// stop_record` for `DesiredProcess::Ephemeral`: a fixed 30s term, then
/// `KILL_TIMEOUT`).
const EPHEMERAL_TERM_TIMEOUT: Duration = Duration::from_secs(30);

/// Preserved observation cadence: the old descriptor's 5s resync
/// (`process_controller_descriptor`, both Process types), now this driver's
/// self-requeue while a Process row is live - the durable row's liveness
/// observation (so an exit is noticed and the restart policy runs) and the
/// one-shot's bounded-runtime and exit observation. Without it an exit would
/// only be noticed when something else woke the actor.
const PROCESS_RESYNC: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessDriverErrorKind {
    /// The durable spec did not decode as the closed Process contract.
    SpecInvalid,
    /// The row's launch identity is incomplete or invalid; the named
    /// construction error is the only diagnosis.
    IdentityIncomplete,
    /// The spec selected a Provider this driver does not own.
    ProviderUnsupported,
    /// The spec's execution target is not drivable in this execution domain.
    ExecutionUnsupported,
    /// The trusted bundle did not contain the requested template binding.
    TemplateUnavailable,
    /// The trusted bundle refused to resolve the launch ticket before any
    /// launch or observation (no matching intent, wrong target/scope/
    /// descriptor posture). Distinct from an ambiguous identity (R15 is about
    /// observed identity) and from a missing template binding.
    ResolutionRefused,
    /// The row is a Guest-owned one-shot outside the guest VMM chain: no
    /// host-minted ticket can ever describe it.
    GuestProcessNotVmm,
    /// A process identity was ambiguous; quarantine per policy (R15).
    IdentityAmbiguous,
    /// A Provider effect failed transiently.
    ProviderEffect,
    /// The in-memory restart budget is exhausted (spec section 32).
    StartExhausted,
    /// Owned children are still retiring; the delete pass requeues.
    DrainPending,
}

impl ProcessDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::ProviderEffect | Self::DrainPending => FailureClass::Retryable,
            Self::SpecInvalid
            | Self::IdentityIncomplete
            | Self::ProviderUnsupported
            | Self::ExecutionUnsupported
            | Self::TemplateUnavailable
            | Self::ResolutionRefused
            | Self::GuestProcessNotVmm
            | Self::IdentityAmbiguous
            | Self::StartExhausted => FailureClass::Terminal,
        }
    }

    /// Whether this kind names a launch refusal no retry can reverse: the
    /// trusted bundle holds no template binding for the row, refuses to
    /// resolve its launch ticket, or owns the row outside the guest VMM
    /// chain. These are the closed provider spellings `template-not-found`,
    /// `resolution-failed` and `guest-process-not-vmm`; the in-memory restart
    /// budget cannot make any of them launchable, so a durable launch reports
    /// them terminally instead of retrying forever. Every other kind stays
    /// with the budget: a provider effect may succeed on the next attempt, and
    /// an identity the ticket path could not bind (for example
    /// `provider-controller-provider-identity-missing`) is evidence the next
    /// pass can re-observe.
    const fn is_unresolvable_launch(self) -> bool {
        matches!(
            self,
            Self::TemplateUnavailable | Self::ResolutionRefused | Self::GuestProcessNotVmm
        )
    }

    /// The registered failure kind this classification reports (issue #508).
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SpecInvalid => FailureKinds::PROCESS_SPEC_INVALID,
            Self::IdentityIncomplete => FailureKinds::PROCESS_IDENTITY_INCOMPLETE,
            Self::ProviderUnsupported => FailureKinds::PROCESS_PROVIDER_UNSUPPORTED,
            Self::ExecutionUnsupported => FailureKinds::PROCESS_EXECUTION_UNSUPPORTED,
            Self::TemplateUnavailable => FailureKinds::PROCESS_TEMPLATE_UNAVAILABLE,
            Self::ResolutionRefused => FailureKinds::PROCESS_RESOLUTION_REFUSED,
            Self::GuestProcessNotVmm => FailureKinds::PROCESS_GUEST_PROCESS_NOT_VMM,
            Self::IdentityAmbiguous => FailureKinds::PROCESS_IDENTITY_AMBIGUOUS,
            Self::ProviderEffect => FailureKinds::PROCESS_PROVIDER_EFFECT_FAILED,
            Self::StartExhausted => FailureKinds::PROCESS_START_BUDGET_EXHAUSTED,
            Self::DrainPending => FailureKinds::PROCESS_DRAIN_PENDING,
        }
    }
}

/// Typed driver failure; mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`] (R13, issue
/// #508).
#[derive(Debug, Clone)]
pub(crate) struct ProcessDriverError {
    kind: ProcessDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl ProcessDriverError {
    fn new(kind: ProcessDriverErrorKind, op: DriverOp) -> Self {
        Self {
            kind,
            op,
            detail: FailureDetail::new(),
        }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for ProcessDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            ProcessDriverErrorKind::SpecInvalid => "process-spec-invalid",
            ProcessDriverErrorKind::IdentityIncomplete => "process-identity-incomplete",
            ProcessDriverErrorKind::ProviderUnsupported => "process-provider-unsupported",
            ProcessDriverErrorKind::ExecutionUnsupported => "process-execution-unsupported",
            ProcessDriverErrorKind::TemplateUnavailable => "process-template-unavailable",
            ProcessDriverErrorKind::ResolutionRefused => "process-resolution-refused",
            ProcessDriverErrorKind::GuestProcessNotVmm => "process-guest-process-not-vmm",
            ProcessDriverErrorKind::IdentityAmbiguous => "process-identity-ambiguous",
            ProcessDriverErrorKind::ProviderEffect => "process-provider-effect-failed",
            ProcessDriverErrorKind::StartExhausted => "process-start-budget-exhausted",
            ProcessDriverErrorKind::DrainPending => "process-drain-pending",
        })
    }
}

impl std::error::Error for ProcessDriverError {}

/// Typed in-memory status projection (R11: `UpdateStatus` -> `set_status`;
/// never persisted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessDriverStatus {
    /// A launch effect is in flight.
    Launching,
    /// The row's desired lifecycle is `stopped` and this pass scheduled its
    /// own re-check: the process is not yet observed gone, so the projection
    /// stays non-Ready until the stop is established.
    Stopping,
    /// The exact process is live; `adopted` records whether the Provider
    /// adopted it instead of launching it.
    Ready { adopted: bool },
    /// A restart backoff timer is scheduled (runtime-only requeue).
    AwaitingRestart { restart_count: u32 },
    /// The process reached a state that satisfies its desired lifecycle.
    Succeeded { code: &'static str },
    /// A process reached a terminal failure that never restarts: the one-shot
    /// `runtime-deadline`, or a durable identity the liveness probe can no
    /// longer verify.
    Failed { code: &'static str },
    /// The realization target carried a drifted/ambiguous identity (R15).
    Quarantined { code: &'static str },
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one Process row (KTD2): the `ResourceSpec`
/// layer (`providerRef` + typed base fields) exactly as persisted. `raw` keeps
/// the exact stored bytes so audits can assert the driver never mutates the
/// durable envelope (no restart annotation is ever written; spec section 32).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessSpecEnvelope {
    /// The exact stored spec bytes; never rewritten by this driver.
    pub(crate) raw: Vec<u8>,
    provider_ref: Option<ResourceRef>,
    base: d2b_contracts_resource::v3::CanonicalJsonObject,
}

/// Typed decode failure; stays inside the provider and surfaces only through
/// the runtime's closed [`d2b_resource_runtime::error::ResourceError::SpecDecode`].
#[derive(Debug)]
pub(crate) struct SpecDecodeFailure {
    pub(crate) reason: String,
}

impl core::fmt::Display for SpecDecodeFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl core::error::Error for SpecDecodeFailure {}

/// The manager-wired decode hook for Process-family rows (`Process` and
/// `EphemeralProcess`). The envelope is type-agnostic (`ResourceSpec` layer
/// plus the exact stored base bytes); the driver decodes the typed family
/// spec from the row's own type name.
pub fn process_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes)
            .map(|spec| ProcessSpecEnvelope {
                raw: bytes.to_vec(),
                provider_ref: spec.provider_ref().cloned(),
                base: spec.base().clone(),
            })
            .map_err(|error| SpecDecodeFailure {
                reason: error.to_string(),
            })
    })
}

/// The four declared Device-owned worker template families (`U17`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceWorkerFamily {
    /// The long-lived `swtpm socket` worker.
    Swtpm,
    /// The one-shot `swtpm_ioctl -i` flush worker.
    SwtpmFlush,
    /// The `crosvm device gpu` sidecar.
    Gpu,
    /// The `crosvm device video-decoder` sidecar.
    Video,
}

/// The VM scope of one declared Device-owned worker row: the owning Device's
/// declared Guest owner (`Device.metadata.ownerRef == Guest/<vm>`).
///
/// A Device-owned worker row runs on the Host (`executionRef
/// Host/host-system`) and names no Guest target, so the row's own launch
/// identity carries no VM. The owning Device's Guest owner is the one
/// coherent VM identity - the same derivation `tpm_device_targets_vm` fences
/// the TPM admission on and the TPM shared-provider effects mint their
/// `VmId` from - and it is what the worker's state dir and sockets are scoped
/// to. A Device with no Guest owner is the genuinely unresolvable case and
/// refuses by name instead of launching against a path no trusted row names.
pub async fn device_worker_vm(
    ctx: &mut ResourceContext,
    device_key: &ResourceKey,
) -> Result<String, &'static str> {
    let Some(row) = ctx
        .get(device_key)
        .await
        .map_err(|_| "device-worker-device-row-unreadable")?
    else {
        return Err("device-worker-device-row-missing");
    };
    serde_json::from_slice::<serde_json::Value>(&row.metadata)
        .ok()
        .and_then(|metadata| {
            metadata
                .get("ownerRef")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .and_then(|owner| ResourceRef::parse(&owner).ok())
        .filter(|owner| owner.resource_type().as_str() == "Guest")
        .map(|owner| owner.name().as_str().to_owned())
        .ok_or("device-worker-vm-unresolved")
}

/// The Device-worker family a template name declares, if any.
///
/// The name alone is not the authority - [`d2b_core::bundle_resolver::device_worker_posture`]
/// still fences the owning Device Provider - but the daemon composes only
/// these argv shapes, so no other template ever receives arguments. The video
/// sidecar's two closed postures (plain vaapi, NVIDIA decode) share the same
/// argv shape; the template the declared row carries is the posture, bound
/// by the broker's own posture table.
pub fn device_worker_family(template: &str) -> Option<DeviceWorkerFamily> {
    match template {
        "swtpm-socket" => Some(DeviceWorkerFamily::Swtpm),
        "swtpm-init-flush" => Some(DeviceWorkerFamily::SwtpmFlush),
        "gpu-worker" | "gpu-render-node" => Some(DeviceWorkerFamily::Gpu),
        "video-worker" | "video-worker-nvidia" => Some(DeviceWorkerFamily::Video),
        _ => None,
    }
}

/// Map the new store's 16-byte deterministic uid onto the contracts crate's
/// UUIDv4-shaped `ResourceUid` (version nibble 4, RFC 9562 variant).
pub fn resource_uid_from_bytes(bytes: &[u8; 16]) -> Option<ResourceUid> {
    ResourceUid::from_bytes(bytes).ok()
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// KTD7 Guest-owner identity source: the owning `Guest` row's durable uid for
/// one canonical Guest reference. `Guest` has not been converted, so a
/// converted Process row owned by a Guest cannot carry the durable owner
/// linkage the pre-v3 store computed (`record.owner_uid` = the resolved owner
/// row's uid) - the manager row carries only the authored
/// `metadata.ownerRef`. The old descriptor composer read the linkage first
/// and the owner identity cache second; a reference this source cannot
/// resolve stays unbound, so the Cloud Hypervisor launch stays refused closed
/// (the broker requires the owner uid to bind `d2b.guest_uid=`).
///
/// Trait, not a concrete type, because `d2b-provider-process` consumes it
/// (`resolve_guest_owner_uid`) and cannot depend on `d2bd`, where the
/// production impl `PlaneGuestOwnerIdentities` (`d2bd/src/resource_plane_v3.rs:1620`)
/// lives.
#[async_trait::async_trait]
pub trait GuestOwnerIdentitySource: Send + Sync + 'static {
    /// The owning row's durable uid for one `Guest` reference, when the
    /// plane that owns `Guest` retains the row.
    async fn guest_owner_uid(&self, zone: &ZoneId, guest_ref: &ResourceRef) -> Option<ResourceUid>;
}

/// The owning Guest's durable uid for a row whose own linkage is absent.
/// A row that already carries a linked owner uid keeps it (the old
/// composer's precedence), and only a Guest owner is ever resolved; without
/// a source the slot stays unbound and the launch refuses closed.
pub async fn resolve_guest_owner_uid(
    source: Option<&dyn GuestOwnerIdentitySource>,
    identity: &ProcessResourceIdentity,
) -> Option<ResourceUid> {
    if identity.launch.owner_uid().is_some() {
        return None;
    }
    let guest = identity
        .launch
        .owner_ref()
        .filter(|owner| owner.resource_type().as_str() == "Guest")?;
    source?.guest_owner_uid(&identity.zone, guest).await
}

// ---------------------------------------------------------------------------
// In-memory restart budget (spec section 32)
// ---------------------------------------------------------------------------

/// Runtime-only restart budget. The count lives in the driver (in-memory,
/// shared with the spawned launch effect); nothing is persisted and no
/// restart annotation is ever written to the durable envelope.
#[derive(Default)]
struct RestartBudget {
    count: AtomicU32,
    restart_scheduled: AtomicBool,
    /// Set when a failure was refused a restart; no further launches.
    exhausted: AtomicBool,
}

impl RestartBudget {
    fn count(&self) -> u32 {
        self.count.load(Ordering::Relaxed)
    }

    fn allows(&self, spec: &ProcessSpec) -> bool {
        let policy = spec.restart_policy();
        policy.class() != RestartClass::Never
            && policy.max_restarts().is_none_or(|max| self.count() < max)
    }

    /// Consume one restart from the budget without arming the next pass's
    /// policy backoff: the exit-driven path schedules its own backoff in the
    /// pass that observed the exit.
    fn consume_restart(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one consumed restart; the next reconcile pass schedules the
    /// policy backoff exactly once.
    fn record_restart(&self) {
        self.consume_restart();
        self.restart_scheduled.store(true, Ordering::Relaxed);
    }

    fn take_restart_scheduled(&self) -> bool {
        self.restart_scheduled.swap(false, Ordering::Relaxed)
    }

    fn mark_exhausted(&self) {
        self.exhausted.store(true, Ordering::Relaxed);
    }

    fn is_exhausted(&self) -> bool {
        self.exhausted.load(Ordering::Relaxed)
    }
}

/// Preserved restart backoff: `base * multiplier^(count-1)`, capped at
/// `backoff_max` (old `restart_delay`).
fn restart_delay(spec: &ProcessSpec, restart_count: u32) -> Duration {
    let policy = spec.restart_policy();
    let base = policy.backoff_base().as_millis();
    let max = policy.backoff_max().as_millis();
    let multiplier = u64::from(policy.backoff_multiplier_milli());
    let mut delay = base;
    for _ in 1..restart_count {
        delay = delay
            .saturating_mul(multiplier)
            .saturating_div(1_000)
            .min(max);
    }
    Duration::from_millis(delay.min(max))
}

// ---------------------------------------------------------------------------
// Factory (U9 wiring shape)
// ---------------------------------------------------------------------------

/// Everything the composition must construct to instantiate the Process
/// driver factory for one zone: the declared facet set the effects run over
/// plus the zone-authority inputs every derived identity folds in (U1).
pub struct ProcessDriverArgs {
    /// Zone the plane serves; the rows this driver reconciles live in it.
    pub zone: ZoneId,
    /// The daemon-supplied facet set the family's effects implementation is
    /// built from. The composition supplies the objects; the driver never
    /// holds a daemon state type (R2).
    pub facets: ProcessEffectFacets,
    /// Zone authority uid (bundle resolver / ZoneAuthorityIdentity path).
    pub zone_uid: Option<ResourceUid>,
    /// Zone policy revision from the authority path.
    pub policy_revision: Option<u64>,
    /// Provider assignment generation (guest execution sessions).
    pub provider_assignment_generation: Option<ResourceGeneration>,
    /// Controller generation the launch ticket binds.
    pub controller_generation: ControllerGeneration,
    /// Binding-declared Guest execution inputs, when the plane serves a
    /// Guest-executing row set.
    pub guest_execution: Option<GuestExecutionBinding>,
    /// The execution domain this plane reconciles under: it gates which
    /// execution targets the rows may name.
    pub mode: ExecutionMode,
}

/// [`ResourceDriverFactory`] for the Process-family resource types
/// (`Process` and `EphemeralProcess`). Construction is infallible by
/// contract: resource-specific failures surface through the driver's
/// validate/recover where the actor owns retry policy (R3).
pub(crate) struct ProcessDriverFactory {
    types: [ResourceTypeName; 2],
    args: ProcessDriverArgs,
}

impl ProcessDriverFactory {
    pub(crate) fn new(args: ProcessDriverArgs) -> Self {
        Self {
            types: [
                ResourceTypeName::new(PROCESS_TYPE_NAME),
                ResourceTypeName::new(EPHEMERAL_PROCESS_TYPE_NAME),
            ],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for ProcessDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        let ProcessDriverArgs {
            zone,
            facets,
            zone_uid,
            policy_revision,
            provider_assignment_generation,
            controller_generation,
            guest_execution,
            mode,
        } = &self.args;
        Box::new(ProcessDriver::new(ProcessDriverArgs {
            zone: zone.clone(),
            facets: facets.clone(),
            zone_uid: zone_uid.clone(),
            policy_revision: *policy_revision,
            provider_assignment_generation: *provider_assignment_generation,
            controller_generation: *controller_generation,
            guest_execution: guest_execution.clone(),
            mode: *mode,
        }))
    }
}

// ---------------------------------------------------------------------------
// Registration: the family's driver declarations
// ---------------------------------------------------------------------------

/// The execution domains both Process-family types can be reconciled in.
///
/// Derived from the execution contract: a Process row's `executionRef` must
/// name a `Host` or a `Guest` (`require_execution_ref` in
/// `d2b-contracts-resource`), and the trusted bundle's runner intents bind
/// the same pair, so both member types can execute in either domain. The
/// labels are lowercase, matching the execution-domain vocabulary the
/// contracts serialize.
pub(crate) const PROCESS_FAMILY_EXECUTION_DOMAINS: &[&str] = &["host", "guest"];

/// The resource types the Process-family driver reads while reconciling.
///
/// Derived from the driver's row reads: a serving worker resolves its owning
/// `VolumeBinding` and that binding's `Volume`; a Device-owned worker reads
/// its `Device` (GPU settings, TPM state volume); a guest-owned row resolves
/// its owning `Guest`; a controller row binds its committed `Provider`
/// identity (KTD7); and the launch binding gate reads the `Endpoint` rows this
/// row's owner owns, because their own publication intent is what names the
/// relationships this launch requires (R18).
pub(crate) const PROCESS_FAMILY_READS: &[WellKnownType] = &[
    WellKnownType::VOLUME_BINDING,
    WellKnownType::VOLUME,
    WellKnownType::DEVICE,
    WellKnownType::GUEST,
    WellKnownType::PROVIDER,
    WellKnownType::ENDPOINT,
];

/// The family's driver declarations: one descriptor per member type, both
/// over the family's shared decoder and factory.
///
/// `Process` and `EphemeralProcess` are `BUILTIN | STARTUP` (no RUNTIME bit):
/// the plane cannot admit workloads without a process launcher, so both must
/// be registered before the plane opens. Neither member type is exportable:
/// `ResourceExport` admits only qualified `*.d2bus.org.*Service` types, so a
/// process can never be an export subject.
///
/// The family's declared operations and services ride on the `Process`
/// descriptor alone: the registry gives one operation reference exactly one
/// owning type (a second declaring driver is refused as foreign), the
/// declaration inspection is a family operation, not a per-member one, and
/// the family's effects service (`process.d2bus.org/effects`, U1) is a
/// family surface its member types share. The family creates no children
/// through this declaration today.
pub fn process_family_descriptors(args: ProcessDriverArgs) -> [DriverDescriptor; 2] {
    let factory: Arc<dyn ResourceDriverFactory> = Arc::new(ProcessDriverFactory::new(args));
    let decoder = process_spec_decoder();
    let descriptor = |resource_type: WellKnownType,
                      operations: &'static [OperationDef],
                      services: &'static [ServiceDecl]|
     -> DriverDescriptor {
        DriverDescriptor {
            resource_type,
            allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
            verbs: CONVERTED_TYPE_VERBS,
            execution: PROCESS_FAMILY_EXECUTION_DOMAINS,
            exportable: false,
            reads: PROCESS_FAMILY_READS,
            operations,
            creations: &[],
            startup: &[],
            services,
            decoder: Arc::clone(&decoder),
            factory: Arc::clone(&factory),
        }
    };
    [
        descriptor(
            WellKnownType::PROCESS,
            process_family_operations(),
            &[PROCESS_EFFECTS_SERVICE],
        ),
        descriptor(WellKnownType::EPHEMERAL_PROCESS, &[], &[]),
    ]
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One Process resource's driver. Effects run through the family's
/// [`ProcessDriverEffects`] implementation, built from the composition-
/// supplied facet set; the actor owns scheduling, retries, and status
/// publication.
#[derive(Clone)]
pub(crate) struct ProcessDriver {
    zone: ZoneId,
    authority: ProcessZoneAuthority,
    effects: Arc<dyn ProcessDriverEffects>,
    budget: Arc<RestartBudget>,
    ephemeral: Arc<EphemeralRuntime>,
    durable: Arc<DurableRuntime>,
    /// The dependency rows this actor has already registered an evidence
    /// watch on. Watches are ephemeral (R12) and this actor re-registers them
    /// from the rows it read each pass, so the set is a per-target guard
    /// against re-registering the same edge every pass rather than state the
    /// launch decision depends on.
    watched: Vec<ResourceKey>,
    /// The authenticated Guest target transport of this row, re-bound to the
    /// live session generation (R19, R29). `None` for a Host-targeted row and
    /// before this row first reaches a live Guest session. Runtime-only, like
    /// the rest of this struct's memory: after a restart it starts empty and
    /// recovery re-adopts from the target instead of inheriting a claim about
    /// a process this process no longer observes.
    guest: Option<GuestArm>,
    /// The live Guest incarnation became unverifiable - a lost session, or a
    /// launch lease that stopped revalidating - so it is quarantined: no
    /// adoption, no signal, and no replacement launch until a fresh
    /// target-local discovery answers (R21).
    guest_quarantined: bool,
}

/// One pass's effect transport for a `Process` row committed to a Guest
/// target (R19, R29).
///
/// One structure, two transports. Everything below the classification - what a
/// live identity does, what a missing one does, what an unattributable one is
/// refused - is written once against [`AdoptionOutcome`] and
/// [`LivenessOutcome`] and holds for a Host-targeted row driving its local
/// process exactly as it does for a Guest-targeted row driving its
/// target-local one. Only the effect that produces the classification differs.
#[derive(Clone)]
struct GuestArm {
    /// The binding re-bound to the live session generation. Every frame it
    /// carries is fenced on that generation by the directory, so a session
    /// that has gone cannot act through a value kept from before it.
    target: TargetBinding,
    /// The session generation the current incarnation was established under.
    session_generation: u64,
    /// The adoption this pass performed while re-binding to a new session
    /// (F5). Consumed once, so the discovery the reconnect required is the
    /// discovery the adoption classification reads.
    adoption: Option<GuestAdoption>,
}

/// What one adoption classification found, over either transport (R15, R16).
///
/// The local Provider vocabulary is richer than a target-local one - it can
/// hand back a stale candidate for exact replacement and can name a missing
/// controller bootstrap - so the local arm maps down to this closed shape and
/// the Guest arm answers in it directly.
#[derive(Debug, Clone)]
enum AdoptionOutcome {
    /// The exact live identity is serving and is adopted.
    Adopted,
    /// Nothing is realized for this row: the pass launches.
    Missing,
    /// A uniquely identified stale identity is available for exact
    /// replacement: stop that identity exactly, then launch.
    Stale(AdoptionCandidate),
    /// A controller exists without its exact bootstrap endpoint: stop and
    /// finalize through the retained authority, then launch.
    StopAndRestart,
    /// The exact realization is present but has not converged: present, and
    /// emphatically not a reason to launch a second one.
    Converging,
    /// Evidence exists and cannot be attributed to this row's exact identity:
    /// never adopted, never signalled. The report is the local Provider's own
    /// evidence for that answer; a target-local classification carries none,
    /// because a target-local realization is either this row's or nothing.
    Quarantined(Option<Box<ProcessStatusReport>>),
}

/// What one liveness observation found, over either transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LivenessOutcome {
    /// The exact identity is present and serving.
    Alive,
    /// The exact identity is present and converging: not ready, and not an
    /// exit.
    Converging,
    /// The exact identity is gone.
    Exited,
    /// The identity could not be established safely.
    Unknown,
    /// The target could not answer at all. This is not an exit (R21): the
    /// incarnation is unreachable, so it is quarantined rather than replaced.
    Unavailable,
}

/// Runtime-only one-shot lifecycle memory (R11: nothing here is persisted;
/// the old durable `startedAt`/`completedAt`/`cleanupEligibleAt` fields are
/// not ported).
#[derive(Default)]
struct EphemeralRuntime {
    /// Set once this actor launched or adopted the one-shot process; a later
    /// `Absent` classification is then the terminal exit, never a first
    /// launch. Across a daemon restart this memory starts empty again, so the
    /// manager-served row is re-adopted/reconciled exactly like a durable row
    /// (no durable status exists to read).
    started: AtomicBool,
    /// When the process was launched or adopted: the clock for the bounded
    /// runtime deadline.
    started_at: tokio::sync::Mutex<Option<tokio::time::Instant>>,
    /// The terminal outcome, once observed: the clock for the retention TTL.
    completed: tokio::sync::Mutex<Option<EphemeralCompletion>>,
}

/// One one-shot terminal outcome: the TTL class and when it was reached.
#[derive(Clone, Copy)]
struct EphemeralCompletion {
    failed: bool,
    code: &'static str,
    at: tokio::time::Instant,
}

impl EphemeralRuntime {
    fn started(&self) -> bool {
        self.started.load(Ordering::Relaxed)
    }

    async fn mark_started(&self) {
        let mut started_at = self.started_at.lock().await;
        if started_at.is_none() {
            *started_at = Some(tokio::time::Instant::now());
        }
        self.started.store(true, Ordering::Relaxed);
    }

    async fn started_at(&self) -> Option<tokio::time::Instant> {
        *self.started_at.lock().await
    }

    /// Record the one-shot terminal state once; a second observation keeps
    /// the first (the TTL clock must not restart).
    async fn finish(&self, failed: bool, code: &'static str) -> EphemeralCompletion {
        let mut completed = self.completed.lock().await;
        *completed.get_or_insert(EphemeralCompletion {
            failed,
            code,
            at: tokio::time::Instant::now(),
        })
    }

    async fn completed(&self) -> Option<EphemeralCompletion> {
        *self.completed.lock().await
    }

    /// Test-only: backdate the runtime-deadline clock.
    #[cfg(test)]
    async fn backdate_started(&self, elapsed: Duration) {
        let mut started_at = self.started_at.lock().await;
        if let Some(at) = started_at.as_mut() {
            *at -= elapsed;
        }
        self.started.store(true, Ordering::Relaxed);
    }

    /// Test-only: backdate the retention TTL clock.
    #[cfg(test)]
    async fn backdate_completed(&self, elapsed: Duration) {
        if let Some(completion) = self.completed.lock().await.as_mut() {
            completion.at -= elapsed;
        }
    }
}

/// Runtime-only durable lifecycle memory (R11: nothing here is persisted).
/// The steady-state pass observes the process this actor adopted or launched
/// through the liveness probe (old `observe_liveness`) instead of re-running
/// the adoption classification, so an exit moves the row out of `Ready` and
/// the restart policy decides what happens next. Across a daemon restart the
/// memory starts empty again and the manager-served row re-enters the
/// adoption path exactly like a first sight.
#[derive(Default)]
struct DurableRuntime {
    /// Set once this actor observed a live durable identity (an `Adopted`
    /// classification at recovery or reconcile, an `Alive` probe, or - for a
    /// `never-adopt` row - the launch this actor completed itself).
    watching: AtomicBool,
}

impl DurableRuntime {
    fn watching(&self) -> bool {
        self.watching.load(Ordering::Relaxed)
    }

    fn mark_watching(&self) {
        self.watching.store(true, Ordering::Relaxed);
    }

    /// The observed process is gone and the pass that saw the exit hands the
    /// relaunch back to the adoption path, so the next pass launches instead
    /// of probing an identity the provider has already released.
    fn mark_exited(&self) {
        self.watching.store(false, Ordering::Relaxed);
    }
}

/// Zone-authority inputs folded into every derived identity (KTD7).
#[derive(Clone)]
struct ProcessZoneAuthority {
    zone_uid: Option<ResourceUid>,
    policy_revision: Option<u64>,
    provider_assignment_generation: Option<ResourceGeneration>,
    controller_generation: ControllerGeneration,
    guest_execution: Option<GuestExecutionBinding>,
    mode: ExecutionMode,
}

// ---------------------------------------------------------------------------
// The launch binding gate read (U4, KTD6, R18, R21)
// ---------------------------------------------------------------------------

/// The `Endpoint` row type whose own publication intent names the
/// relationships a Process launch requires (R18).
const ENDPOINT_ROW_TYPE: &str = "Endpoint";

/// What one read of the manager proved about the bindings this Process
/// requires.
///
/// `expected` is derived from each `Endpoint` row's OWN `/endpoint/bindings`
/// publication layer - the source's current publication intent, never a
/// consumer-local slot table and never the rows that happen to exist.
/// `observed` is read from the manager for exactly those rows: a
/// relationship's consumer and canonical slot come from its OWN committed
/// spec, so comparing them against the source's publication is a real check,
/// and its delivery state comes from the `/binding` projection its own actor
/// publishes. The two authority digests exist only in the endpoint's
/// publication layer, so they are read with it - which is exactly what lets
/// the sealed lease notice an authorization-only change that moved no
/// endpoint generation (R18, R21).
#[derive(Default)]
struct BindingObservation {
    expected: Vec<ExpectedBindingRow>,
    observed: Vec<ObservedBinding>,
    /// The rows this read took evidence from: the endpoints it derived
    /// expectations from and the relationships it observed.
    dependencies: Vec<ResourceKey>,
}

/// Why one binding read could not produce an observation.
enum BindingObservationFault {
    /// The manager cannot answer right now, or a source published a
    /// relationship whose realization or committed row it has not proven yet:
    /// the launch defers and issues no effect.
    Unproven,
    /// The published evidence is malformed or foreign: terminal, because
    /// retrying the same evidence cannot change the answer.
    Refused(BindingGateError),
}

/// Derive what one `Endpoint` row publishes for `process_ref`, and observe the
/// relationships it names (R18).
///
/// Nothing here consults a consumer-local slot table or the set of rows that
/// happen to exist: the endpoint's own `/endpoint/bindings` layer is the
/// publication intent, and only the entries naming this exact consumer are
/// required. An endpoint that PUBLISHED that layer and named no entry for
/// this consumer mints no expectation, which is the answer for every Process
/// that requires no `EndpointBinding` at all.
///
/// An endpoint that has published nothing is a different answer, and the
/// difference is the whole point: it has stated nothing, and a statement that
/// was never made is not read as the one that would have been made.
async fn observe_endpoint_bindings(
    ctx: &mut ResourceContext,
    view: &ResourceView,
    process_ref: &ResourceRef,
    observation: &mut BindingObservation,
) -> Result<(), BindingObservationFault> {
    let Some(projection) = view.observed_status_projection() else {
        // The manager holds this endpoint's row and view, and the view
        // carries no projection published for its CURRENT generation: its
        // actor has not run a pass yet (spawn in flight, actor restart), or
        // the row has moved past the generation its last publication was for.
        // None of those is this source saying it publishes no relationship
        // for this consumer, so the launch defers until it has published -
        // silence is not a publication (R18, R21).
        return Err(BindingObservationFault::Unproven);
    };
    let published = projection.pointer("/endpoint/bindings");
    let Some(entries) = published.and_then(serde_json::Value::as_array) else {
        return match published {
            // The layer is published and does not carry the publication set:
            // the source stated something this reader cannot interpret.
            Some(_) => Err(BindingObservationFault::Refused(BindingGateError::Malformed)),
            // The endpoint family publishes this layer on every pass - an
            // endpoint that grants nothing publishes an EMPTY array - so a
            // projection with no layer under `/endpoint` is not this source
            // saying it grants nothing. It is a publication this read could
            // not interpret as the publication set, which is the same
            // not-proven the missing projection is, and defers on the same
            // terms: a later pass over a layer this source does publish.
            None => Err(BindingObservationFault::Unproven),
        };
    };
    let consumer = process_ref.to_canonical_string();
    // The source's own readiness evidence travels with every expectation: a
    // relationship is delivered over one exact REALIZATION, and the token the
    // endpoint published is the realization this launch is gated on (R18).
    let ready = view.observed_status() == Some(ResourceStatus::Ready);
    let published_incarnation = projection
        .pointer("/endpoint/incarnation")
        .and_then(serde_json::Value::as_str);
    for entry in entries {
        if entry.pointer("/consumer").and_then(serde_json::Value::as_str) != Some(consumer.as_str()) {
            continue;
        }
        let (Some(name), Some(endpoint), Some(slot), Some(authorization), Some(dependency)) = (
            published_field(entry, "/name"),
            published_field(entry, "/endpoint"),
            published_field(entry, "/slot"),
            published_field(entry, "/authorizationDigest"),
            published_field(entry, "/dependencyRevision"),
        ) else {
            return Err(BindingObservationFault::Refused(BindingGateError::Malformed));
        };
        let Some(incarnation) = published_incarnation.map(str::to_owned) else {
            // An endpoint that published a relationship without naming a
            // realization has proven nothing about what it would grant access
            // to, so the launch waits for the token instead of gating on one it
            // cannot name.
            return Err(BindingObservationFault::Unproven);
        };
        let (Ok(endpoint_ref), Ok(binding_ref)) = (
            ResourceRef::parse(&endpoint),
            ResourceRef::parse(&format!("{ENDPOINT_BINDING_RESOURCE_TYPE}/{name}")),
        ) else {
            return Err(BindingObservationFault::Refused(BindingGateError::Malformed));
        };
        let key = binding_key(ctx, binding_ref.name().as_str());
        observation.dependencies.push(key.clone());
        let relation = match ctx.lookup_view(&key).await {
            RowLookup::Present { row, .. } => row,
            // The publication intent names this relationship and the manager
            // has not shown it. That is the ordinary not-yet: the gate defers
            // on a required row it cannot see rather than dropping the
            // expectation and launching as if it were never required.
            RowLookup::Absent { .. } | RowLookup::Unavailable { .. } => {
                return Err(BindingObservationFault::Unproven);
            }
            RowLookup::Error { .. } => {
                return Err(BindingObservationFault::Refused(
                    BindingGateError::EvidenceUnreadable,
                ));
            }
        };
        let committed: EndpointBindingSpec = serde_json::from_slice(&relation.spec).map_err(|_| {
            BindingObservationFault::Refused(BindingGateError::EvidenceUnreadable)
        })?;
        // The relationship's OWN committed endpoint and consumer, compared
        // against what the source published for it. This is what tells a
        // withdrawn authorization - the source no longer admits this consumer,
        // or moved it - from a delivery that simply has not arrived, with no
        // endpoint generation bump involved.
        if committed.endpoint_ref() != &endpoint_ref || committed.execution_ref() != process_ref {
            return Err(BindingObservationFault::Refused(BindingGateError::Foreign));
        }
        let expectation = ExpectedBindingRow::new(
            binding_ref.clone(),
            endpoint_ref,
            view.generation,
            // The relationship's row generation is store-assigned, so no
            // publication can name it before the row is read and inventing one
            // would make every expectation a guess. What fences a re-issued row
            // is the lease, which seals this row's identity and generation and
            // compares them again immediately before the effect.
            relation.generation,
            process_ref.clone(),
            slot,
            authorization.clone(),
            dependency.clone(),
            incarnation.clone(),
            if ready {
                BindingPreparation::Prepared
            } else {
                BindingPreparation::Incomplete
            },
        )
        .map_err(BindingObservationFault::Refused)?;
        let binding_uid = resource_uid_from_bytes(&relation.uid)
            .map(|uid| uid.as_str().to_owned())
            .ok_or(BindingObservationFault::Refused(BindingGateError::Foreign))?;
        observation.expected.push(expectation);
        observation.observed.push(ObservedBinding::new(
            binding_ref,
            binding_uid,
            relation.generation,
            committed.execution_ref().clone(),
            committed.slot().as_str().to_owned(),
            authorization,
            dependency,
            // The relationship travels with the ENDPOINT row generation its
            // owner published it from, so the sealed lease compares it too.
            view.generation,
            ready,
            Some(incarnation.clone()),
            BindingDeliveryEvidence::from_projection(relation.observed_status_projection()),
        ));
    }
    Ok(())
}

/// The manager key of the canonical relationship row one published
/// relationship names.
fn binding_key(ctx: &ResourceContext, name: &str) -> ResourceKey {
    ResourceKey::new(ctx.key().zone.as_str(), ENDPOINT_BINDING_RESOURCE_TYPE, name)
}

/// One committed row that still holds a `Process` row's retirement.
///
/// Every variant names a condition, never a row: the detail carries the
/// closed slug and no key, no spec, and no host material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetirementBlocker {
    /// An `EndpointBinding` row naming this exact Process as its consumer.
    ConsumedBinding,
    /// An `Endpoint` row naming this exact Process as its producer.
    ProducedEndpoint,
    /// A row of one of those two types whose committed bytes could not be
    /// read at all.
    Unreadable,
}

impl RetirementBlocker {
    /// The closed slug the retention failure names.
    const fn code(self) -> &'static str {
        match self {
            Self::ConsumedBinding => "binding.consumer-retiring",
            Self::ProducedEndpoint => "endpoint.producer-retiring",
            Self::Unreadable => "dependency.unreadable",
        }
    }
}

/// Whether one owner-scoped sibling is an `Endpoint` row this exact Process
/// produces, and so still holds its retirement (R22).
///
/// The blocker is read from the committed bytes rather than from the row name,
/// because the producer is not derivable from a key. Bytes that cannot be
/// read at all are a blocker too: a retirement barrier fails closed, because a
/// row this pass cannot read is a row whose release this pass cannot prove.
fn produced_endpoint_blocker(
    row: &StoredDesiredResource,
    process: &ResourceRef,
) -> Option<RetirementBlocker> {
    match row.key.type_name.as_str() {
        ENDPOINT_ROW_TYPE => match endpoint_producer_ref(&row.spec) {
            Some(producer) if producer == *process => Some(RetirementBlocker::ProducedEndpoint),
            Some(_) => None,
            None => Some(RetirementBlocker::Unreadable),
        },
        _ => None,
    }
}

/// The producer one `Endpoint` row's committed bytes name.
///
/// The `Endpoint` spec belongs to the Endpoint family and this driver never
/// holds another family's type (R2), so the one committed fact the barrier
/// needs is read out of the stored document instead of through a typed
/// decode that would pin this crate to that family's contract. Both
/// persisted shapes answer: the spec-store envelope and the bare typed
/// document.
fn endpoint_producer_ref(spec: &[u8]) -> Option<ResourceRef> {
    let document = serde_json::from_slice::<serde_json::Value>(spec).ok()?;
    let reference = document
        .pointer("/producerRef")
        .or_else(|| document.pointer("/spec/producerRef"))?;
    ResourceRef::parse(reference.as_str()?).ok()
}

/// One non-empty string field of a published entry, or the fact that it is
/// not there.
fn published_field(entry: &serde_json::Value, pointer: &str) -> Option<String> {
    entry
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

impl ProcessDriver {
    pub(crate) fn new(args: ProcessDriverArgs) -> Self {
        let ProcessDriverArgs {
            zone,
            facets,
            zone_uid,
            policy_revision,
            provider_assignment_generation,
            controller_generation,
            guest_execution,
            mode,
        } = args;
        Self {
            zone,
            authority: ProcessZoneAuthority {
                zone_uid,
                policy_revision,
                provider_assignment_generation,
                controller_generation,
                guest_execution,
                mode,
            },
            // The family's own implementation, built from the facet set the
            // composition supplied (U1): the same value the hosted service
            // factory builds.
            effects: Arc::new(ProcessEffectsService::new(facets)),
            budget: Arc::new(RestartBudget::default()),
            ephemeral: Arc::new(EphemeralRuntime::default()),
            durable: Arc::new(DurableRuntime::default()),
            watched: Vec::new(),
            guest: None,
            guest_quarantined: false,
        }
    }

    /// In-memory restart count (spec section 32): observable for audits, never
    /// persisted.
    pub(crate) fn restart_count(&self) -> u32 {
        self.budget.count()
    }

    fn error(&self, kind: ProcessDriverErrorKind, op: DriverOp) -> ProcessDriverError {
        ProcessDriverError::new(kind, op)
    }

    /// The terminal quarantine failure for one ambiguous adoption report
    /// (issue #508): the observed adoption condition and phase name what was
    /// ambiguous instead of a bare code.
    fn identity_ambiguous(&self, op: DriverOp, report: &ProcessStatusReport) -> ProcessDriverError {
        self.error(ProcessDriverErrorKind::IdentityAmbiguous, op)
            .with_detail(
                FailureDetail::at("adopt/identity").comparison(FailureComparison::new(
                    "observed.adoption",
                    "exactly one matching identity",
                    format!("{:?} ({:?})", report.adoption, report.phase),
                )),
            )
    }

    /// The terminal failure for one row identity field that does not parse
    /// (issue #508): the field and the parse error are the diagnosis.
    fn identity_field_invalid(
        &self,
        op: DriverOp,
        field: &'static str,
        detail: String,
    ) -> ProcessDriverError {
        self.error(ProcessDriverErrorKind::SpecInvalid, op)
            .with_detail(
                FailureDetail::at("identity/field")
                    .comparison(FailureComparison::new(
                        field,
                        "a valid contract value",
                        "invalid",
                    ))
                    .with_note(detail),
            )
    }

    /// The terminal failure for a declared Device-worker launch whose typed
    /// parameters the trusted inputs cannot resolve (`U17` gap closure): the
    /// row launches with its derived parameters or it refuses, never bare.
    fn resolution_refused(&self, op: DriverOp, code: &'static str) -> ProcessDriverError {
        self.error(ProcessDriverErrorKind::ResolutionRefused, op)
            .with_detail(FailureDetail::at("identity/device-worker-parameters").with_note(code))
    }

    /// Decode the stored envelope and the typed family spec in one step. The
    /// row's own type name selects the contract (`Process` or
    /// `EphemeralProcess`); both are served by this factory.
    fn decoded_spec(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<(ProcessSpecEnvelope, ProcessFamilySpec), ProcessDriverError> {
        let envelope = ctx.spec::<ProcessSpecEnvelope>().map_err(|error| {
            self.error(ProcessDriverErrorKind::SpecInvalid, op)
                .with_detail(FailureDetail::at("spec/decode").with_note(error.to_string()))
        })?;
        let base = envelope.base.to_canonical_bytes();
        let spec = match ctx.key().type_name.as_str() {
            EPHEMERAL_PROCESS_TYPE_NAME => serde_json::from_slice::<EphemeralProcessSpec>(&base)
                .map(ProcessFamilySpec::Ephemeral)
                .map_err(|error| {
                    self.error(ProcessDriverErrorKind::SpecInvalid, op)
                        .with_detail(
                            FailureDetail::at("spec/decode")
                                .comparison(FailureComparison::new(
                                    "spec.contract",
                                    EPHEMERAL_PROCESS_TYPE_NAME,
                                    "decode failed",
                                ))
                                .with_note(error.to_string()),
                        )
                })?,
            other => serde_json::from_slice::<ProcessSpec>(&base)
                .map(ProcessFamilySpec::Process)
                .map_err(|error| {
                    self.error(ProcessDriverErrorKind::SpecInvalid, op)
                        .with_detail(
                            FailureDetail::at("spec/decode")
                                .comparison(FailureComparison::new(
                                    "spec.contract",
                                    other,
                                    "decode failed",
                                ))
                                .with_note(error.to_string()),
                        )
                })?,
        };
        Ok((envelope.clone(), spec))
    }

    /// Provider reference checks (old `desired()`): a Process spec must select
    /// one of the daemon-owned fixed Providers.
    fn check_provider(
        &self,
        envelope: &ProcessSpecEnvelope,
        op: DriverOp,
    ) -> Result<(), ProcessDriverError> {
        let expected = format!(
            "{} or {}",
            d2b_provider_process_minijail::PROVIDER_REF,
            d2b_provider_process_systemd::PROVIDER_REF
        );
        let Some(provider_ref) = envelope.provider_ref.as_ref() else {
            return Err(self
                .error(ProcessDriverErrorKind::SpecInvalid, op)
                .with_detail(FailureDetail::at("spec/provider").comparison(
                    FailureComparison::new("spec.providerRef", expected, "absent"),
                )));
        };
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(self
                .error(ProcessDriverErrorKind::SpecInvalid, op)
                .with_detail(FailureDetail::at("spec/provider").comparison(
                    FailureComparison::new(
                        "spec.providerRef",
                        expected,
                        provider_ref.to_canonical_string(),
                    ),
                )));
        }
        if !matches!(
            provider_ref.name().as_str(),
            d2b_provider_process_minijail::PROVIDER_NAME
                | d2b_provider_process_systemd::PROVIDER_NAME
        ) {
            return Err(self
                .error(ProcessDriverErrorKind::ProviderUnsupported, op)
                .with_detail(FailureDetail::at("spec/provider").comparison(
                    FailureComparison::new(
                        "spec.providerRef",
                        expected,
                        provider_ref.to_canonical_string(),
                    ),
                )));
        }
        Ok(())
    }

    /// Resolve the adoption/launch identity (KTD7) from the durable row plus
    /// the zone-authority inputs.
    ///
    /// Owner ref/UID, execution target, target selector, VM scope, and legacy
    /// role come from one resolver ([`resolve_launch_identity`]) so this
    /// driver, the legacy runtime, and the ticket builder cannot disagree
    /// about any field. Target-local evidence (pidfd, /proc starttime, socket
    /// path) stays behind the provider effect port.
    async fn identity(
        &self,
        ctx: &mut ResourceContext,
        provider_ref: &ResourceRef,
        op: DriverOp,
    ) -> Result<ProcessResourceIdentity, ProcessDriverError> {
        let key = ctx.key();
        let resource_label = format!("{}/{}", key.type_name, key.name);
        let resource_type = ContractResourceTypeName::parse(&key.type_name).map_err(|error| {
            self.identity_field_invalid(op, "resource.typeName", error.to_string())
        })?;
        let name = ResourceName::parse(&key.name)
            .map_err(|error| self.identity_field_invalid(op, "resource.name", error.to_string()))?;
        let zone = ZoneId::parse(&key.zone)
            .map_err(|error| self.identity_field_invalid(op, "resource.zone", error.to_string()))?;
        let resource_uid = resource_uid_from_bytes(ctx.uid()).ok_or_else(|| {
            self.identity_field_invalid(op, "resource.uid", "not a uid".to_owned())
        })?;
        let resource_generation = ResourceGeneration::new(ctx.generation()).map_err(|_| {
            self.identity_field_invalid(op, "resource.generation", ctx.generation().to_string())
        })?;
        // The row's process class rides the identity: the production effects
        // bind the committed controller-provider identity for controller rows
        // and have no spec to consult when they finalize (KTD7).
        let (_, spec) = self.decoded_spec(ctx, op)?;
        let process_class = spec.execution().process_class();
        // The manager resolves an owner key only for owners that are its own
        // rows; an owned child of an unconverted owner falls back to the
        // authored reference the row was ingested with (a Guest owner never
        // appears in the plane).
        let owner_ref = ctx
            .owner_key()
            .and_then(|owner| {
                ResourceRef::parse(&format!("{}/{}", owner.type_name, owner.name)).ok()
            })
            .or_else(|| crate::identity::decode_metadata_owner_ref(ctx.metadata()));
        // A binding-owned virtiofsd worker executes on the host (the signed
        // `virtiofsd-worker` template binds the Host execution reference) and
        // is targeted at the attachment's Guest through the ticket's target
        // ref (KTD7). The authoritative target input is the owning
        // VolumeBinding row's declared execution ref.
        let mut worker_launch = None;
        let declared_target = match ctx.owner_key().cloned() {
            Some(owner) if owner.type_name == "VolumeBinding" => match ctx.get(&owner).await {
                Ok(Some(row)) => {
                    let binding = match serde_json::from_slice::<ResourceSpec>(&row.spec) {
                        Ok(envelope) => serde_json::from_slice::<
                            d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec,
                        >(&envelope.base().to_canonical_bytes())
                        .map_err(|error| {
                            self.error(ProcessDriverErrorKind::SpecInvalid, op).with_detail(
                                FailureDetail::at("spec/decode").with_note(error.to_string()),
                            )
                        })?,
                        Err(error) => {
                            return Err(self
                                .error(ProcessDriverErrorKind::SpecInvalid, op)
                                .with_detail(
                                    FailureDetail::at("spec/decode").with_note(error.to_string()),
                                ))
                        }
                    };
                    worker_launch = self.serving_worker_launch(ctx, &binding, op).await?;
                    Some(binding.execution_ref().clone())
                }
                _ => None,
            },
            _ => None,
        };
        let launch = resolve_launch_identity(&LaunchRow {
            owner_ref: owner_ref.as_ref(),
            owner_uid: ctx.owner().and_then(resource_uid_from_bytes),
            execution_ref: spec.execution().execution_ref(),
            process_name: name.as_str(),
            template: spec.execution().template().as_str(),
            // The binding row is the owner that declares this target, so the
            // declared-target rule applies to it.
            declared_target: declared_target.as_ref().zip(owner_ref.as_ref()),
        })
        .map_err(|error| {
            tracing::warn!(
                resource = %resource_label,
                identity_error = error.code(),
                "process launch identity incomplete"
            );
            self.error(ProcessDriverErrorKind::IdentityIncomplete, op)
                .with_detail(
                    FailureDetail::at("identity/resolve")
                        .comparison(FailureComparison::new(
                            "launch.identity",
                            "complete",
                            "incomplete",
                        ))
                        .with_note(error.code()),
                )
        })?;
        let resource_ref = ResourceRef::new(resource_type, name);
        let mut identity = ProcessResourceIdentity {
            zone,
            resource_ref,
            resource_uid,
            resource_generation,
            process_class,
            provider_ref: provider_ref.clone(),
            launch,
            zone_uid: self.authority.zone_uid.clone(),
            policy_revision: self.authority.policy_revision,
            provider_assignment_generation: self.authority.provider_assignment_generation,
            controller_generation: self.authority.controller_generation,
            controller_provider_uid: None,
            controller_provider_generation: None,
            guest_execution: self.authority.guest_execution.clone(),
            worker_launch,
            device_worker_launch: None,
        };
        // The declared Device-owned worker rows carry their typed launch
        // parameters into the launch: the derivation needs the trusted bundle
        // and the daemon runtime paths, so it stays behind the provider seam
        // and this pass attaches what it derived.
        //
        // Only the launch consumes them, and only a launch can refuse for
        // their inputs (an unbound Wayland socket, an unresolvable state
        // dir). The delete op stops and finalizes an identity and must never
        // depend on launch-only inputs: an identity error there converges the
        // delete while the launched process keeps running unowned.
        if op != DriverOp::Delete {
            identity.device_worker_launch = self
                .effects
                .device_worker_launch(ctx, &identity, &spec)
                .await
                .map_err(|code| self.resolution_refused(op, code))?;
        }
        Ok(identity)
    }

    /// Derive the binding-declared serving-worker launch inputs.
    ///
    /// Only the signed `virtiofsd-worker` template on a VolumeBinding-owned
    /// Process reaches this path; anything else keeps an unbound launch. The
    /// declaration itself stays in the binding and Volume rows: the worker
    /// serves the view the binding names, with the attachment tuning the
    /// Volume declared.
    async fn serving_worker_launch(
        &self,
        ctx: &mut ResourceContext,
        binding: &d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec,
        op: DriverOp,
    ) -> Result<Option<ServingWorkerLaunch>, ProcessDriverError> {
        let (_, spec) = self.decoded_spec(ctx, op)?;
        if spec.execution().template().as_str() != d2b_provider_volume_virtiofs::WORKER_TEMPLATE {
            return Ok(None);
        }
        let volume_key = ResourceKey::new(
            self.zone.as_str(),
            "Volume",
            binding.volume_ref().name().as_str(),
        );
        let Some(row) = ctx.get(&volume_key).await.ok().flatten() else {
            return Ok(None);
        };
        let volume = match serde_json::from_slice::<ResourceSpec>(&row.spec) {
            Ok(envelope) => serde_json::from_slice::<
                d2b_contracts_resource::v3::volume::VolumeSpec,
            >(&envelope.base().to_canonical_bytes())
            .map_err(|error| {
                self.error(ProcessDriverErrorKind::SpecInvalid, op).with_detail(
                    FailureDetail::at("spec/decode").with_note(error.to_string()),
                )
            })?,
            Err(error) => {
                return Err(self
                    .error(ProcessDriverErrorKind::SpecInvalid, op)
                    .with_detail(
                        FailureDetail::at("spec/decode").with_note(error.to_string()),
                    ))
            }
        };
        let Some(view) = volume.views().get(binding.view().as_str()) else {
            return Ok(None);
        };
        let Some(attachment) = volume
            .attachments()
            .iter()
            .find(|attachment| attachment.execution_ref() == binding.execution_ref())
        else {
            return Ok(None);
        };
        let settings = attachment.settings();
        let source = volume.source();
        let root = match source.settings().kind() {
            d2b_contracts_resource::v3::volume::SourceKind::LocalPath => {
                let Some(policy) = source.settings().source_policy_id() else {
                    return Ok(None);
                };
                let policy = policy.as_str().to_owned();
                Some(ServingWorkerRoot::StoragePath(
                    if policy == "state-root" || policy == "default-state" {
                        "path:state-root".to_owned()
                    } else {
                        format!("path:{policy}")
                    },
                ))
            }
            // A `nix-closure` Volume's bytes are the broker-managed
            // per-Guest store-view farm the bundle names through the
            // Guest's store-view intent; the ticket composes the served
            // view root from it (the `ro-store` share's preserved
            // `store-view/live` redirect).
            d2b_contracts_resource::v3::volume::SourceKind::NixClosure => {
                Some(ServingWorkerRoot::StoreViewFarm)
            }
            _ => None,
        };
        Ok(Some(ServingWorkerLaunch {
            volume_ref: binding.volume_ref().clone(),
            view: binding.view().clone(),
            guest_ref: binding.execution_ref().clone(),
            root,
            view_path: view.path().to_owned(),
            access: binding.access(),
            thread_pool_size: settings.thread_pool_size().unwrap_or(1),
            posix_acl: settings.posix_acl(),
            xattr: settings.xattr(),
            cache: settings.cache(),
            socket_group: settings
                .socket_group()
                .map(|group| group.as_str().to_owned()),
        }))
    }

    /// The effect transport for one pass (R19, R29).
    ///
    /// A Host-targeted row - and a row whose context was assembled without a
    /// target layer at all, which is every caller that never needed one -
    /// drives locally, exactly as before this transport existed. A
    /// Guest-targeted row gets the binding the manager committed for it,
    /// re-bound to the live session generation.
    ///
    /// A reconnect is the one event that re-binds the transport, and adoption
    /// is the only operation allowed to cross a session generation change
    /// (F5): a binding left over from a lost session re-discovers its
    /// target-local realization before anything acts on it. That answer also
    /// clears a quarantine - the target either confirms this exact
    /// realization is there or confirms nothing is, and either way this Host
    /// stops acting over an incarnation nobody verified (R21).
    async fn guest_arm(
        &mut self,
        target: Option<TargetBinding>,
        op: DriverOp,
    ) -> Result<Option<GuestArm>, ProcessDriverError> {
        // The committed target travels by value, not as a borrow of the
        // actor's context: nothing here keeps the context alive across a
        // session round trip.
        let Some(binding) = target else {
            return Ok(None);
        };
        if !binding.is_guest() {
            return Ok(None);
        }
        let Some(live) = binding.live_generation() else {
            // The live incarnation is unreachable from here on, so it is
            // quarantined: nothing replaces it until the reconnect produced
            // the discovery that clears this flag (R21). Desired state stays,
            // the assignment stays, and nothing is issued - the target's own
            // reconnect drives the retry.
            self.guest_quarantined = true;
            return Err(self.target_unavailable(op, &binding));
        };
        if let Some(arm) = &self.guest
            && arm.session_generation == live
        {
            return Ok(Some(arm.clone()));
        }
        let (rebound, outcome) = binding.adopt().await.map_err(|error| self.target_failed(op, error))?;
        let arm = GuestArm {
            target: rebound,
            session_generation: live,
            adoption: outcome.adopted().first().cloned(),
        };
        self.guest_quarantined = false;
        self.guest = Some(arm.clone());
        Ok(Some(arm))
    }

    /// No live session for the target this row is committed to (R21).
    ///
    /// The row keeps its desired spec and its assignment; nothing is
    /// realized, adopted, or signalled, and the target's reconnect - not a
    /// retry loop - is what brings this actor back.
    fn target_unavailable(&self, op: DriverOp, binding: &TargetBinding) -> ProcessDriverError {
        let guest = binding
            .guest_reference()
            .map(|reference| reference.to_canonical_string())
            .unwrap_or_default();
        tracing::warn!(
            operation = ?op,
            guest = %guest,
            "process target reports no live guest session"
        );
        ProcessDriverError::new(ProcessDriverErrorKind::ProviderEffect, op).with_detail(
            FailureDetail::at("target/session")
                .comparison(FailureComparison::new("target.session", "live", "unavailable"))
                .with_note("the desired row stays committed; the reconnect drives the retry"),
        )
    }

    /// One closed target-layer refusal (R19).
    ///
    /// Every variant is a condition, never a material: the detail names the
    /// closed code the directory reported, so no socket name, host path, or
    /// device identity can reach a status or a log line through it.
    fn target_failed(&self, op: DriverOp, error: TargetError) -> ProcessDriverError {
        let code = error.to_string();
        tracing::warn!(operation = ?op, code = %code, "process target effect failed");
        ProcessDriverError::new(ProcessDriverErrorKind::ProviderEffect, op).with_detail(
            FailureDetail::at("target/effect")
                .comparison(FailureComparison::new("target.effect", "accepted", code.as_str()))
                .with_note("the authenticated target-control session refused this effect"),
        )
    }

    /// Whether this Zone still retains an identity for the row.
    async fn has_active_process(
        &self,
        guest: Option<&GuestArm>,
        identity: &ProcessResourceIdentity,
        op: DriverOp,
    ) -> Result<bool, ProcessDriverError> {
        let Some(arm) = guest else {
            return Ok(self.effects.has_active(
                &identity.zone,
                identity.zone_uid.as_ref(),
                &identity.resource_ref,
            ));
        };
        // A target-local realization is present exactly when the target says
        // so. An unavailable answer is not presence and not absence either
        // (R21); it reads as "nothing this Host may signal", which is the
        // only answer a destructive step may act on.
        let observation = arm.target.observe().await.map_err(|error| self.target_failed(op, error))?;
        Ok(observation.is_present())
    }

    /// Classify what is live for this row (R15, R16).
    async fn adopt_process(
        &mut self,
        guest: Option<&mut GuestArm>,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        op: DriverOp,
    ) -> Result<AdoptionOutcome, ProcessDriverError> {
        let Some(arm) = guest else {
            return Ok(match self.effects.adopt(identity, spec).await {
                Ok(classification) => match classification {
                    ProviderAdoption::Adopted(_) => AdoptionOutcome::Adopted,
                    ProviderAdoption::Absent => AdoptionOutcome::Missing,
                    ProviderAdoption::Stale { candidate } => AdoptionOutcome::Stale(candidate),
                    ProviderAdoption::ControllerBootstrapMissing => AdoptionOutcome::StopAndRestart,
                    ProviderAdoption::Quarantined(report) => {
                        AdoptionOutcome::Quarantined(Some(Box::new(report)))
                    }
                },
                Err(error) => return Err(map_provider_error(error, op)),
            });
        };
        if arm.adoption.is_none() {
            let (rebound, outcome) =
                arm.target.adopt().await.map_err(|error| self.target_failed(op, error))?;
            arm.target = rebound;
            arm.adoption = outcome.adopted().first().cloned();
        }
        let outcome = match arm.adoption.take() {
            Some(GuestAdoption::Adopted(instance))
                if instance.state() == TargetInstanceState::Ready =>
            {
                arm.session_generation = instance.session_generation();
                self.guest_quarantined = false;
                AdoptionOutcome::Adopted
            }
            // Present and converging: not an adoption, and never a reason to
            // realize a second incarnation over it.
            Some(GuestAdoption::Adopted(_)) => AdoptionOutcome::Converging,
            // The target confirmed that nothing is realized for this exact
            // source, uid, and generation: there is nothing to inherit.
            Some(GuestAdoption::Missing) | None => {
                self.guest_quarantined = false;
                AdoptionOutcome::Missing
            }
        };
        self.guest = Some(arm.clone());
        Ok(outcome)
    }

    /// Observe the identity this row owns (R21).
    async fn probe_process(
        &mut self,
        guest: Option<&mut GuestArm>,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        op: DriverOp,
    ) -> Result<LivenessOutcome, ProcessDriverError> {
        let Some(arm) = guest else {
            return Ok(match self.effects.probe(identity, spec).await {
                Ok(ProviderLiveness::Alive) => LivenessOutcome::Alive,
                Ok(ProviderLiveness::Exited) => LivenessOutcome::Exited,
                Ok(ProviderLiveness::Unknown) => LivenessOutcome::Unknown,
                Err(error) => return Err(map_provider_error(error, op)),
            });
        };
        let observation = arm.target.observe().await.map_err(|error| self.target_failed(op, error))?;
        let outcome = match observation {
            TargetObservation::Ready { .. } => LivenessOutcome::Alive,
            TargetObservation::Realizing { .. } => LivenessOutcome::Converging,
            TargetObservation::Absent => LivenessOutcome::Exited,
            // The target could not answer. This is not an exit (R21): the
            // incarnation is unreachable, so it is quarantined rather than
            // replaced, and no relaunch is allowed over it until a fresh
            // discovery answers.
            TargetObservation::Unavailable => {
                self.guest_quarantined = true;
                LivenessOutcome::Unavailable
            }
        };
        self.guest = Some(arm.clone());
        Ok(outcome)
    }

    /// Stop this row's exact identity over whichever transport it is
    /// committed to, and release whatever authority holds it.
    async fn stop_process(
        &mut self,
        guest: Option<&mut GuestArm>,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        op: DriverOp,
    ) -> Result<(), ProcessDriverError> {
        let Some(arm) = guest else {
            return self.stop_and_finalize(identity, spec, op).await;
        };
        tracing::warn!(
            resource = %identity.resource_ref.to_canonical_string(),
            operation = ?op,
            "stopping the target-local process for its driver operation"
        );
        self.delete_guest(arm, op).await
    }

    /// Remove this row's exact target-local realization (F3, R20).
    ///
    /// Idempotent under retry and across a reconnect: a realization that is
    /// already gone answers the same way, and only this source's instance is
    /// touched - another row realized on the same Guest is not disturbed.
    async fn delete_guest(
        &mut self,
        arm: &mut GuestArm,
        op: DriverOp,
    ) -> Result<(), ProcessDriverError> {
        match arm.target.delete().await {
            Ok(_) => {
                self.guest = None;
                self.guest_quarantined = false;
                self.durable.mark_exited();
                Ok(())
            }
            Err(error) => Err(self.target_failed(op, error)),
        }
    }

    async fn stop_and_finalize(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        op: DriverOp,
    ) -> Result<(), ProcessDriverError> {
        tracing::warn!(
            resource = %identity.resource_ref.to_canonical_string(),
            operation = ?op,
            "stopping a managed process for its driver operation"
        );
        self.effects
            .stop(
                identity,
                spec,
                Duration::from_millis(spec.drain_timeout().as_millis()),
                KILL_TIMEOUT,
            )
            .await
            .map_err(|error| map_provider_error(error, op))?;
        self.effects
            .finalize(identity)
            .await
            .map_err(|error| map_provider_error(error, op))?;
        Ok(())
    }

    /// Spawn the signed-ticket launch as a long effect (R5, KTD12): the
    /// mailbox never blocks on the launch; completion arrives as
    /// [`EffectCompleted`] with the operation id. Retryable failures count
    /// against the in-memory restart budget (spec section 32) and classify
    /// retryable; the actor schedules the requeue from the closed class (R13)
    /// and the next pass applies the policy backoff. A launch ticket the
    /// trusted bundle can never mint, and an exhausted budget, are terminal
    /// instead - no requeue ever follows them.
    ///
    /// The sealed binding lease is revalidated HERE, immediately before the
    /// effect and under this pass's own serialized manager boundary, not
    /// earlier: a lease that was correct when preparation concluded says
    /// nothing about the moment the process starts (KTD6, R18). A lease that
    /// no longer holds issues no effect at all - the pass reports the revoked
    /// authority and schedules the retry the rest of this arm already uses.
    async fn spawn_launch(
        &mut self,
        ctx: &mut ResourceContext,
        guest: Option<GuestArm>,
        identity: ProcessResourceIdentity,
        spec: &ProcessSpec,
        lease: Option<&BindingAuthorityLease>,
    ) -> Result<ReconcileOutcome, ProcessDriverError> {
        if self.budget.is_exhausted() {
            return Err(self.error(ProcessDriverErrorKind::StartExhausted, DriverOp::Reconcile));
        }
        if let Some(lease) = lease
            && self
                .revalidate_lease(ctx, &identity, lease, DriverOp::Reconcile)
                .await
                .is_err()
        {
            tracing::warn!(
                resource = %identity.resource_ref.to_canonical_string(),
                "the launch authority moved before the launch effect; issuing none"
            );
            let _ = ctx.requeue_after(PROCESS_RESYNC);
            return Ok(ReconcileOutcome::RetryScheduled);
        }
        // A quarantined incarnation is never replaced from under the Host
        // (R21): nothing is realized until the target's own reconnect produced
        // the discovery that cleared the quarantine.
        if self.guest_quarantined && guest.is_some() {
            ctx.set_status(ProcessDriverStatus::Quarantined { code: "target-unavailable" });
            let _ = ctx.requeue_after(PROCESS_RESYNC);
            return Ok(ReconcileOutcome::RetryScheduled);
        }
        // The target-local realization is assembled before the operation
        // starts, so an incomplete one is a refusal this pass reports rather
        // than a long effect that fails after the frame was due.
        let guest_launch = match &guest {
            Some(arm) => Some((
                arm.target.clone(),
                guest_realization(&identity, spec, lease)?,
                guest_local_handle(arm.target.source()),
            )),
            None => None,
        };
        let operation = ctx.begin_operation();
        let effects = Arc::clone(&self.effects);
        let effect_sender = ctx.effect_sender();
        let budget = Arc::clone(&self.budget);
        let task_spec = spec.clone();
        // A `never-adopt` row never enters the adoption classification, so the
        // launch itself arms the observation flag for the identity it just
        // recorded; the `adopt-on-restart` arm arms the same flag through
        // `adopt` on the pass that follows.
        let arm_observation = task_spec.adoption_policy() == AdoptionPolicy::NeverAdopt;
        let durable = Arc::clone(&self.durable);
        tokio::spawn(async move {
            let effect_result = match guest_launch {
                // A row committed to a Guest target realizes through the
                // authenticated session (R19, R29). The frame carries the
                // exact host-resolved realization - the resolved spec plus
                // the prepared `EndpointBinding` deliveries this sealed lease
                // held - so the delivery reaches the Guest only because that
                // lease revalidated immediately above (KTD6, R18).
                Some((target, realization, handle)) => {
                    let digest = realization.spec_digest();
                    match target.realize(realization.encode(), &digest, &handle).await {
                        Ok(instance) if instance.state() == TargetInstanceState::Ready => {
                            if arm_observation {
                                durable.mark_watching();
                            }
                            EffectResult::Completed
                        }
                        // The target holds the realization but its local effect
                        // has not converged. That is a launch in flight, not a
                        // failed launch: the next pass observes it instead of
                        // realizing a second incarnation over it.
                        Ok(_) => EffectResult::Failed(
                            DriverFailure::error(
                                DriverOp::Reconcile,
                                FailureKinds::PROCESS_PROVIDER_EFFECT_FAILED,
                                FailureClass::Retryable,
                            )
                            .at("reconcile/launch")
                            .with_comparison(FailureComparison::new(
                                "target.realization",
                                "ready",
                                "realizing",
                            ))
                            .with_note("the target-local effect has not converged"),
                        ),
                        Err(error) => {
                            launch_failure(&budget, &identity, &task_spec, error.to_string())
                        }
                    }
                }
                None => match effects.launch(&identity, &task_spec, LAUNCH_TIMEOUT).await {
                    Ok(_) => {
                        if arm_observation {
                            durable.mark_watching();
                        }
                        EffectResult::Completed
                    }
                    Err(error) => launch_failure(&budget, &identity, &task_spec, error),
                },
            };
            let _ = effect_sender.send(EffectCompleted {
                operation,
                result: effect_result,
            });
        });
        Ok(ReconcileOutcome::InProgress { operation })
    }

    /// One-shot recovery (old `start_record` ephemeral arm + the start
    /// classification): an exact live identity is adopted and remembered, an
    /// absent one waits for the first reconcile launch, and drifted or
    /// ambiguous evidence quarantines. `ControllerBootstrapMissing` cannot
    /// describe a one-shot ticket and stays terminal, exactly as the old
    /// `start_record_plan` refused it (`TemplateUnavailable`).
    ///
    /// The adoption is fenced by the same launch binding gate the reconcile
    /// arm answers (KTD6, R18): a one-shot survivor is re-admitted by
    /// evidence read now, never by the fact that its process still exists,
    /// because a delivery withdrawn while this daemon was down is invisible
    /// to every effect it has already issued.
    async fn recover_ephemeral(
        &mut self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<RecoveryOutcome, ProcessDriverError> {
        // `Pending` reports `Missing` and touches nothing: the survivor is
        // not an identity this pass verified, so unproven evidence is no
        // reason to stop one. `Refused` is the same terminal refusal the
        // reconcile arms report, and `Ready` carries the lease the adoption
        // revalidates immediately before it runs.
        let binding = self.prepare_launch(ctx, identity, DriverOp::Recover).await?;
        let lease = match &binding {
            ProcessBindingPreparation::NotRequired => None,
            ProcessBindingPreparation::Ready(lease) => Some(lease),
            ProcessBindingPreparation::Pending => return Ok(RecoveryOutcome::Missing),
            ProcessBindingPreparation::Refused(error) => {
                return Err(self.binding_gate_refused(identity, *error, DriverOp::Recover));
            }
        };
        if let Some(lease) = lease
            && self
                .revalidate_lease(ctx, identity, lease, DriverOp::Recover)
                .await
                .is_err()
        {
            return Ok(RecoveryOutcome::Missing);
        }
        match self.effects.adopt_ephemeral(identity, spec).await {
            Ok(ProviderAdoption::Adopted(_)) => {
                self.ephemeral.mark_started().await;
                ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                Ok(RecoveryOutcome::Adopted)
            }
            Ok(ProviderAdoption::Absent) => Ok(RecoveryOutcome::Missing),
            Ok(ProviderAdoption::Stale { .. }) | Ok(ProviderAdoption::Quarantined(_)) => {
                ctx.set_status(ProcessDriverStatus::Quarantined {
                    code: "identity-ambiguous",
                });
                Ok(RecoveryOutcome::Quarantined)
            }
            Ok(ProviderAdoption::ControllerBootstrapMissing) => Err(self.error(
                ProcessDriverErrorKind::TemplateUnavailable,
                DriverOp::Recover,
            )),
            Err(error) => Err(map_provider_error(error, DriverOp::Recover)),
        }
    }

    /// The one preparation both Process lifetimes run before they act.
    ///
    /// A long-running `Process` and a run-to-completion `EphemeralProcess`
    /// reach their launch, their adoption, and their restart through this
    /// single call. It derives the committed consumer identity from the row the
    /// manager already built - so no layer copies the row's identity a second
    /// time - asks the effect owner for the plan, and refuses a launch whose
    /// bindings are not prepared (AE20, AE28, R40).
    ///
    /// After the plan it answers the launch binding gate (KTD6, R18) and
    /// returns that closed answer: the expected canonical `EndpointBinding`
    /// set comes from the CURRENT publication intent of the `Endpoint` rows
    /// this row's owner holds, so the caller acts on the gate instead of
    /// dropping it. The plan answer is consumed here - `admits_start` is the
    /// last thing that reads it - and the gate outcome is what the launch and
    /// adoption paths act on.
    ///
    /// A `None` plan is the pre-plan path: the effect owner has not resolved a
    /// plan for this row, and the row's own posture still drives the launch.
    /// U34 removes that branch together with the ticket authority.
    async fn prepare_launch(
        &mut self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        op: DriverOp,
    ) -> Result<ProcessBindingPreparation, ProcessDriverError> {
        let subject = identity
            .subject()
            .map_err(|refusal| {
                tracing::warn!(
                    resource = %identity.resource_ref.to_canonical_string(),
                    refusal = %refusal,
                    "process preparation refused"
                );
                self.error(ProcessDriverErrorKind::SpecInvalid, op)
            })?;
        let plan = self.effects.prepare(identity, &subject).await.map_err(|error| {
            tracing::warn!(
                resource = %identity.resource_ref.to_canonical_string(),
                error = %error,
                "process plan resolution refused"
            );
            self.error(ProcessDriverErrorKind::ResolutionRefused, op)
        })?;
        if let Some(plan) = plan.as_ref()
            && !plan.admits_start()
        {
            // The plan resolved but its source side is not complete. A
            // consumer that started now would be waiting on access that does
            // not exist, which is exactly the startup cycle R40 removes.
            return Err(self
                .error(ProcessDriverErrorKind::ResolutionRefused, op)
                .with_detail(
                    FailureDetail::at("prepare/bindings")
                        .with_note("a required binding is not prepared"),
                ));
        }
        let binding = match self.observe_bindings(ctx, &identity.resource_ref).await {
            Ok(observation) => {
                self.watch_evidence(ctx, &observation.dependencies).await;
                resolve_process_binding_preparation(
                    identity.resource_uid.clone(),
                    &observation.expected,
                    &observation.observed,
                )
            }
            Err(BindingObservationFault::Unproven) => ProcessBindingPreparation::Pending,
            Err(BindingObservationFault::Refused(error)) => {
                ProcessBindingPreparation::Refused(error)
            }
        };
        Ok(binding)
    }

    /// Revalidate one sealed lease against freshly read evidence, inside the
    /// same serialized manager boundary the preparation ran under (KTD6).
    ///
    /// This is the call that belongs immediately before the effect, never the
    /// preparation: authority that was standing when the lease was sealed says
    /// nothing about the moment the process starts. Anything that moved in
    /// between - a re-derived row, a re-issued grant, a withdrawn
    /// authorization, a re-realized endpoint - fails closed and issues no
    /// effect.
    async fn revalidate_lease(
        &mut self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        lease: &BindingAuthorityLease,
        op: DriverOp,
    ) -> Result<(), ProcessDriverError> {
        let observed = match self.observe_bindings(ctx, &identity.resource_ref).await {
            Ok(observation) => observation.observed,
            // A plane that cannot answer is not a world that stayed still.
            Err(_) => {
                return Err(self.binding_gate_refused(
                    identity,
                    BindingGateError::LeaseRevoked,
                    op,
                ));
            }
        };
        lease
            .revalidate(&observed)
            .map_err(|error| self.binding_gate_refused(identity, error, op))
    }

    /// Register this actor's evidence watch on one dependency row, at most
    /// once per target (R12, R21).
    ///
    /// The condition is [`WatchCondition::ProjectionChanged`], never `Ready`:
    /// a delivery downgrade - a relationship whose endpoint was replaced, or
    /// whose authorization was withdrawn - keeps its target on `Ready` while
    /// its evidence layer changes underneath it, so a readiness phase can
    /// never be the wake-up this row needs.
    async fn watch_evidence(&mut self, ctx: &mut ResourceContext, targets: &[ResourceKey]) {
        for target in targets {
            if self.watched.contains(target) {
                continue;
            }
            if ctx
                .watch(target.clone(), WatchCondition::ProjectionChanged)
                .await
                .is_ok()
            {
                self.watched.push(target.clone());
            }
        }
    }

    /// The terminal refusal one binding-gate answer carries (R18).
    ///
    /// The gate's closed slug rides the failure detail: the failure-kind
    /// vocabulary is the contracts crate's, and the slug is what says WHICH
    /// gate condition stopped this launch - malformed expectation, foreign
    /// evidence, unreadable projection, or a revoked lease.
    fn binding_gate_refused(
        &self,
        identity: &ProcessResourceIdentity,
        error: BindingGateError,
        op: DriverOp,
    ) -> ProcessDriverError {
        tracing::warn!(
            resource = %identity.resource_ref.to_canonical_string(),
            slug = error.code(),
            "process launch binding gate refused the pass"
        );
        self.error(ProcessDriverErrorKind::ResolutionRefused, op)
            .with_detail(FailureDetail::at("prepare/binding-gate").comparison(
                FailureComparison::new(
                    "binding.evidence",
                    "delivered at the expected realization",
                    error.code(),
                ),
            ))
    }

    /// Read what the manager currently proves about the canonical
    /// `EndpointBinding` rows this exact Process requires (R18).
    ///
    /// The endpoints a Process may consume are the endpoints its OWNER owns: a
    /// session owns the `Process` rows and the `Endpoint` rows together, so
    /// this is the existing owner-scoped sibling listing and no new manager
    /// surface is added for it. A root Process has no siblings, which is the
    /// honest answer - and the one every existing non-display Process gets.
    async fn observe_bindings(
        &self,
        ctx: &mut ResourceContext,
        process_ref: &ResourceRef,
    ) -> Result<BindingObservation, BindingObservationFault> {
        let siblings = ctx
            .owner_siblings()
            .await
            .map_err(|_| BindingObservationFault::Unproven)?;
        let mut observation = BindingObservation::default();
        for row in siblings
            .iter()
            .filter(|row| row.key.type_name == ENDPOINT_ROW_TYPE)
        {
            observation.dependencies.push(row.key.clone());
            let view = match ctx.lookup_view(&row.key).await {
                RowLookup::Present { row, .. } => row,
                // The owner-scoped listing named this endpoint, and the
                // manager then answered that it holds no view for it. Both
                // answers come from the one committed-row set, so this is one
                // pass learning that the row it must reason about is not there
                // to be reasoned about - deleted between the two reads, or a
                // listing this pass could not catch up with. Skipping it
                // would drop an expectation this launch may owe and start the
                // row carrying no delivery at all. Deferring is not a stall:
                // the next pass re-derives the sibling set from the manager,
                // so an endpoint that really is gone is simply absent from
                // that listing.
                RowLookup::Absent { .. }
                | RowLookup::Unavailable { .. }
                | RowLookup::Error { .. } => {
                    return Err(BindingObservationFault::Unproven);
                }
            };
            observe_endpoint_bindings(ctx, &view, process_ref, &mut observation).await?;
        }
        Ok(observation)
    }

    /// Refuse to retire this row while a relationship it consumes, or an
    /// `Endpoint` it produces, is still committed (R22).
    ///
    /// The broker derives the principal a revoke names from this row's own
    /// committed consumer reference, so a `Process` row that retired first
    /// would leave its own release unprovable - and the ACL entry it
    /// installed standing. The row therefore outlives them: the effect has
    /// already stopped by the time this runs, and this stage is what keeps
    /// the consumer identity available until every relationship naming it and
    /// every `Endpoint` it produces is gone.
    ///
    /// The two blockers are found the way each is actually written down, out
    /// of one owner-scoped sibling listing: a session owns its `Process` rows
    /// and its `Endpoint` rows together.
    ///
    /// A CONSUMED relationship is named by the publication intent of the
    /// endpoints in that listing - the same `/endpoint/bindings` layer the
    /// launch gate reads, and the only place a relationship is named that
    /// outlives the relationship row itself. It is read here rather than
    /// through the launch observation because the two ask different
    /// questions: at launch an unseen relationship is an ordinary not-yet,
    /// while at retirement an UNSEEN relationship row is a release that has
    /// already been proved and retired, and must not hold this row forever.
    /// A PRODUCED `Endpoint` is named by its own committed bytes instead,
    /// because a producer is a fact the endpoint carries, not something a key
    /// carries.
    ///
    /// Idempotent under retry. A manager that cannot answer, a view it cannot
    /// read, and a sibling whose committed bytes cannot be decoded all retain
    /// the row rather than guessing at it: a barrier that failed open would
    /// retire the very identity the release needs.
    async fn retain_until_dependencies_retired(
        &self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
    ) -> Result<(), ProcessDriverError> {
        let op = DriverOp::Delete;
        let retained = |slug: &'static str, observed: &str| {
            self.error(ProcessDriverErrorKind::DrainPending, op).with_detail(
                FailureDetail::at("delete/relationships")
                    .comparison(FailureComparison::new("delete.dependencies", "retired", observed))
                    .with_note(slug),
            )
        };
        let siblings = ctx
            .owner_siblings()
            .await
            .map_err(|_| retained(RetirementBlocker::Unreadable.code(), "unavailable"))?;
        let consumer_ref = identity.resource_ref.to_canonical_string();
        for endpoint in siblings
            .iter()
            .filter(|row| row.key.type_name == ENDPOINT_ROW_TYPE)
        {
            let view = match ctx.lookup_view(&endpoint.key).await {
                RowLookup::Present { row, .. } => row,
                // A view the manager cannot answer is not evidence that
                // nothing is published for this consumer.
                RowLookup::Unavailable { .. } | RowLookup::Error { .. } => {
                    return Err(retained(RetirementBlocker::Unreadable.code(), "unavailable"));
                }
                // No view at all: the endpoint's own actor published no
                // publication intent, which names no relationship here.
                RowLookup::Absent { .. } => continue,
            };
            // The entries are read in place: the publication layer is the
            // view's own value and nothing here mutates it, so neither the
            // layer nor the array is copied out of it.
            let entries = view
                .observed_status_projection()
                .and_then(|layer| layer.pointer("/endpoint/bindings"))
                .and_then(serde_json::Value::as_array);
            for entry in entries.into_iter().flatten() {
                let publishes_this_consumer =
                    published_field(entry, "/consumer").as_deref() == Some(consumer_ref.as_str());
                if !publishes_this_consumer {
                    continue;
                }
                let Some(name) = published_field(entry, "/name") else {
                    continue;
                };
                let key = binding_key(ctx, &name);
                if matches!(ctx.lookup(&key).await, RowLookup::Present { .. }) {
                    return Err(retained(RetirementBlocker::ConsumedBinding.code(), "committed"));
                }
            }
        }
        if let Some(blocker) = siblings
            .iter()
            .find_map(|row| produced_endpoint_blocker(row, &identity.resource_ref))
        {
            return Err(retained(blocker.code(), "committed"));
        }
        Ok(())
    }

    /// Stop the verified live incarnation whose delivery authority was
    /// withdrawn, and never report readiness behind it (R21, R22).
    ///
    /// A required relationship that is no longer `Delivered` at the
    /// realization this row launched over is an authority withdrawal, not a
    /// not-yet. Two facts are established here and neither is optional:
    ///
    /// 1. The row stops being ready. `Ready` is what a dependent reads as
    ///    "this consumer may use its endpoint" and what a phase gate mints
    ///    identities from, so this pass publishes a non-ready classification
    ///    before it returns whatever the stop decided.
    /// 2. The verified live incarnation stops. A process that keeps running
    ///    over access its source no longer grants is exactly the effect the
    ///    barrier exists to prevent, so this pass stops the exact identity it
    ///    can verify through the same adoption evidence every other stop uses.
    ///
    /// Nothing here launches. The next pass relaunches only after the gate
    /// answers `Ready` with a freshly sealed lease, so a withdrawn delivery
    /// defers a relaunch rather than forbidding the row forever. An identity
    /// this Host cannot attribute is left alone and never signalled: it is
    /// reported as not-ready and replaced by nothing.
    ///
    async fn stop_withdrawn_effect(
        &mut self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<(), ProcessDriverError> {
        let op = DriverOp::Reconcile;
        let mut guest = self.guest_arm(ctx.target().cloned(), op).await?;
        if self.has_active_process(guest.as_ref(), identity, op).await? {
            tracing::warn!(
                resource = %identity.resource_ref.to_canonical_string(),
                "a required binding is no longer delivered; stopping the live incarnation"
            );
            self.stop_process(guest.as_mut(), identity, spec, op).await?;
            ctx.set_status(ProcessDriverStatus::Succeeded {
                code: "binding-delivery-withdrawn",
            });
            return Ok(());
        }
        Ok(())
    }

    /// Stop the verified live one-shot whose delivery authority was
    /// withdrawn (R21, R22).
    ///
    /// The same two facts as the durable arm over the one-shot's own
    /// stop-and-finalize effect: the row stops reading ready and the
    /// verified incarnation stops, and no relaunch is issued here. A one-shot
    /// that never started has nothing live to stop, so this converges on the
    /// gate's own answer.
    async fn stop_withdrawn_one_shot(
        &mut self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<(), ProcessDriverError> {
        let op = DriverOp::Reconcile;
        if !self.effects.has_active(
            &identity.zone,
            identity.zone_uid.as_ref(),
            &identity.resource_ref,
        ) {
            return Ok(());
        }
        tracing::warn!(
            resource = %identity.resource_ref.to_canonical_string(),
            "a required binding is no longer delivered; stopping the live one-shot"
        );
        self.stop_and_finalize_ephemeral(identity, spec, op).await?;
        ctx.set_status(ProcessDriverStatus::Succeeded {
            code: "binding-delivery-withdrawn",
        });
        Ok(())
    }

    async fn reconcile_process(
        &mut self,
        ctx: &mut ResourceContext,
        identity: ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ReconcileOutcome, ProcessDriverError> {
        // A row whose desired lifecycle is `Stopped` is realized only once the
        // process is observed gone, so the pass establishes the stop instead
        // of asserting it: `Satisfied` publishes wire `Ready` and fires every
        // `WatchCondition::Ready` watcher on the row, and a live process
        // behind that reading is a realized state the row never reached.
        // Both lifetimes prepare through this one call before they act, so
        // the long-running and run-to-completion arms reach their plan the
        // same way (AE20, AE28). The launch binding gate is answered inside
        // it: the expected canonical relationship set comes from what the
        // endpoints this row's owner publishes for this exact consumer (R18).
        let binding = self.prepare_launch(ctx, &identity, DriverOp::Reconcile).await?;

        // A row committed to a Guest target runs the same policy below over
        // its target-local realization; only the effect that produces each
        // classification differs (R19, R29). The transport is resolved once,
        // here, so every branch reaches the authenticated session the same way.
        let mut guest = self.guest_arm(ctx.target().cloned(), DriverOp::Reconcile).await?;
        if spec.desired_lifecycle() == DesiredLifecycle::Stopped {
            // A live identity is stopped through the same exact escalation
            // every other path uses; with no verified identity there is
            // nothing this daemon may signal.
            if self.has_active_process(guest.as_ref(), &identity, DriverOp::Reconcile).await? {
                self.stop_process(guest.as_mut(), &identity, spec, DriverOp::Reconcile)
                    .await?;
            }
            return match self
                .probe_process(guest.as_mut(), &identity, spec, DriverOp::Reconcile)
                .await?
            {
                // The observed stop: the desired state is realized, so the row
                // may read its terminal status. The identity is handed back to
                // the adoption path (a later `running` spec adopts/launches
                // afresh instead of probing an identity the provider already
                // released).
                LivenessOutcome::Exited => {
                    self.durable.mark_exited();
                    ctx.set_status(ProcessDriverStatus::Succeeded {
                        code: "process-stopped",
                    });
                    Ok(ReconcileOutcome::Satisfied)
                }
                // Still live (a process that came up behind the stop, one the
                // stop left running, or one that has not converged yet), or an
                // identity nothing confirms: the stop is not established, so
                // the pass schedules its own re-check and reports the retry -
                // never `Satisfied` with a live process behind it.
                LivenessOutcome::Alive | LivenessOutcome::Converging | LivenessOutcome::Unknown => {
                    ctx.set_status(ProcessDriverStatus::Stopping);
                    let _ = ctx.requeue_after(PROCESS_RESYNC);
                    Ok(ReconcileOutcome::RetryScheduled)
                }
                // The target cannot answer at all (R21): the stop is not
                // established either and the row is not ready, and the
                // reconnect - not a claim of success - brings this actor back.
                LivenessOutcome::Unavailable => {
                    ctx.set_status(ProcessDriverStatus::Quarantined {
                        code: "target-unavailable",
                    });
                    let _ = ctx.requeue_after(PROCESS_RESYNC);
                    Ok(ReconcileOutcome::RetryScheduled)
                }
            };
        }

        // The launch binding gate (KTD6, R18). `NotRequired` is a real answer
        // - this row requires no `EndpointBinding` - and every path below then
        // runs exactly as it did before the gate existed. `Ready` carries the
        // sealed authority the effect revalidates.
        //
        // The two closed answers that are NOT `Ready` are where a live
        // process would otherwise keep running over access its source no
        // longer grants (R21). `Pending` is the ordinary not-yet for a row
        // that never launched, and an authority withdrawal for one that did:
        // either way the verified incarnation stops, the row stops reading
        // ready, and exactly one retryable requeue runs on the cadence this
        // arm already uses - a relaunch waits for a freshly sealed lease, it
        // is never issued here. `Refused` is terminal for the launch, because
        // retrying the same malformed or foreign evidence cannot change the
        // answer, and it stops the live effect on exactly the same terms.
        let lease = match &binding {
            ProcessBindingPreparation::NotRequired => None,
            ProcessBindingPreparation::Ready(lease) => Some(lease),
            ProcessBindingPreparation::Pending => {
                self.stop_withdrawn_effect(ctx, &identity, spec).await?;
                let _ = ctx.requeue_after(PROCESS_RESYNC);
                return Ok(ReconcileOutcome::RetryScheduled);
            }
            ProcessBindingPreparation::Refused(error) => {
                self.stop_withdrawn_effect(ctx, &identity, spec).await?;
                return Err(self.binding_gate_refused(&identity, *error, DriverOp::Reconcile));
            }
        };

        // A retryable launch failure from the previous pass: schedule exactly
        // one runtime-only requeue with the policy restart delay (R13; spec
        // section 32 - nothing is persisted). The row is NOT ready (no
        // process exists): the pass reports the scheduled retry, never a
        // readiness claim.
        if self.budget.take_restart_scheduled() {
            let restart_count = self.budget.count();
            let delay = restart_delay(spec, restart_count);
            let _ = ctx.requeue_after(delay);
            ctx.set_status(ProcessDriverStatus::AwaitingRestart { restart_count });
            return Ok(ReconcileOutcome::RetryScheduled);
        }

        if spec.adoption_policy() == AdoptionPolicy::NeverAdopt && !self.durable.watching() {
            // The unexpected live identity (if any) is stopped and the row is
            // launched fresh (preserved behavior). Once this actor has
            // launched - the line below - the identity is its own, the flag
            // is armed by the launch effect, and the observation branch above
            // takes over; without that gate the next pass would read its own
            // process as unexpected and stop it on every completion.
            if self.has_active_process(guest.as_ref(), &identity, DriverOp::Reconcile).await? {
                self.stop_process(guest.as_mut(), &identity, spec, DriverOp::Reconcile)
                    .await?;
            }
            ctx.set_status(ProcessDriverStatus::Launching);
            return self.spawn_launch(ctx, guest, identity, spec, lease).await;
        }

        // Steady state: a row this actor saw live is observed through the
        // liveness probe (old `observe_liveness`), so an exit leaves `Ready`
        // and the restart policy decides what happens next. The self-requeue
        // is the preserved 5s descriptor resync: without it nothing would ever
        // re-enter the pass and the row would report `Ready` over a process
        // that is gone.
        if self.durable.watching() {
            return match self
                .probe_process(guest.as_mut(), &identity, spec, DriverOp::Reconcile)
                .await?
            {
                LivenessOutcome::Alive => {
                    ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                    let _ = ctx.requeue_after(PROCESS_RESYNC);
                    Ok(ReconcileOutcome::Satisfied)
                }
                // Present and not serving yet: not ready, and emphatically not
                // an exit. A realization that is converging is never replaced.
                LivenessOutcome::Converging => {
                    ctx.set_status(ProcessDriverStatus::Launching);
                    let _ = ctx.requeue_after(PROCESS_RESYNC);
                    Ok(ReconcileOutcome::RetryScheduled)
                }
                LivenessOutcome::Exited => self.durable_exit(ctx, &identity, spec),
                // An identity no longer verifies (old `observe_liveness`
                // Unknown): the same terminal `process-identity-ambiguous`
                // refusal the adoption classification reports, so the row
                // publishes `Failed` - a `Satisfied` pass here would publish
                // wire `Ready` over a process this daemon cannot identify.
                // Nothing re-enters the pass, no relaunch happens, and no
                // signal ever reaches the unverifiable candidate.
                LivenessOutcome::Unknown => {
                    ctx.set_status(ProcessDriverStatus::Failed {
                        code: "identity-ambiguous",
                    });
                    Err(self
                        .error(
                            ProcessDriverErrorKind::IdentityAmbiguous,
                            DriverOp::Reconcile,
                        )
                        .with_detail(
                            FailureDetail::at("observe/liveness")
                                .comparison(FailureComparison::new(
                                    "observed.liveness",
                                    "exactly one verifiable identity",
                                    "Unknown",
                                ))
                                .with_note("provider identity could not be verified safely"),
                        ))
                }
                // The target could not answer (R21): the incarnation is
                // unreachable, not gone. It is quarantined, the row is not
                // ready, and nothing - a replacement launch included - acts on
                // it until the reconnect produced a fresh discovery.
                LivenessOutcome::Unavailable => {
                    ctx.set_status(ProcessDriverStatus::Quarantined {
                        code: "target-unavailable",
                    });
                    let _ = ctx.requeue_after(PROCESS_RESYNC);
                    Ok(ReconcileOutcome::RetryScheduled)
                }
            };
        }

        // The adoption classification is itself an effect this row's authority
        // is held to, so the sealed lease is revalidated before it runs and
        // not only before the launch that may follow it (KTD6, R18).
        if let Some(lease) = lease
            && self
                .revalidate_lease(ctx, &identity, lease, DriverOp::Reconcile)
                .await
                .is_err()
        {
            let _ = ctx.requeue_after(PROCESS_RESYNC);
            return Ok(ReconcileOutcome::RetryScheduled);
        }
        match self
            .adopt_process(guest.as_mut(), &identity, spec, DriverOp::Reconcile)
            .await?
        {
            AdoptionOutcome::Adopted => {
                // The live identity is observed from here on: this pass arms
                // the observation cadence and every later pass probes liveness
                // instead of re-adopting.
                self.durable.mark_watching();
                ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                let _ = ctx.requeue_after(PROCESS_RESYNC);
                Ok(ReconcileOutcome::Satisfied)
            }
            AdoptionOutcome::Missing => {
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_launch(ctx, guest, identity, spec, lease).await
            }
            AdoptionOutcome::StopAndRestart => {
                // The Provider owns the exact stop and finalization before the
                // replacement launch (preserved controller-bootstrap effect
                // ordering).
                self.stop_and_finalize(&identity, spec, DriverOp::Reconcile)
                    .await?;
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_launch(ctx, guest, identity, spec, lease).await
            }
            AdoptionOutcome::Stale(candidate) => {
                self.effects
                    .stop_stale(&identity.provider_ref, &candidate)
                    .await
                    .map_err(|error| map_provider_error(error, DriverOp::Reconcile))?;
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_launch(ctx, guest, identity, spec, lease).await
            }
            // The exact realization is there and still converging: present, so
            // no second incarnation is realized over it, and not ready either.
            AdoptionOutcome::Converging => {
                ctx.set_status(ProcessDriverStatus::Launching);
                let _ = ctx.requeue_after(PROCESS_RESYNC);
                Ok(ReconcileOutcome::RetryScheduled)
            }
            AdoptionOutcome::Quarantined(Some(report)) => {
                Err(self.identity_ambiguous(DriverOp::Reconcile, &report))
            }
            // A target-local realization is either this exact row's or nothing
            // - there is no third identity for it to be ambiguous with - so this
            // answer is the target reporting that it could not attribute what
            // it holds. It is quarantined, never adopted, never signalled.
            AdoptionOutcome::Quarantined(None) => {
                ctx.set_status(ProcessDriverStatus::Quarantined {
                    code: "identity-ambiguous",
                });
                Err(self
                    .error(ProcessDriverErrorKind::IdentityAmbiguous, DriverOp::Reconcile)
                    .with_detail(
                        FailureDetail::at("adopt/identity").comparison(FailureComparison::new(
                            "adopt.identity",
                            "exactly one attributable realization",
                            "unattributable",
                        )),
                    ))
            }
        }
    }

    /// The observed durable exit (old `observe_liveness` `Exited` plus
    /// `process_restart_allowed`): the restart policy decides between one
    /// budgeted restart, requeued at the policy backoff so the next pass
    /// re-enters the adoption path and relaunches, and the terminal
    /// `process-exited` refusal the row reports from then on.
    fn durable_exit(
        &mut self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ReconcileOutcome, ProcessDriverError> {
        if !self.budget.allows(spec) {
            // No restart is available (policy class `never`, or the ceiling
            // is reached): the exit is the row's terminal reading, and the
            // pass refuses instead of reporting a satisfied row. A satisfied
            // pass publishes wire `Ready`, and a phase gate mints identities
            // from that phase (`DaemonGpuLifecyclePort::declared_worker`), so
            // a row whose process is gone and whose restart budget cannot
            // authorize another launch must never read Ready. The refusal is
            // the spent restart budget, terminal; the raw exit spelling rides
            // the failure's note. The row stays under observation, so a later
            // trigger re-reports the exit instead of reading the row as a
            // first sight and launching a process the policy forbade.
            ctx.set_status(ProcessDriverStatus::Failed {
                code: "process-exited",
            });
            return Err(self
                .error(ProcessDriverErrorKind::StartExhausted, DriverOp::Reconcile)
                .with_detail(
                    FailureDetail::at("observe/liveness")
                        .comparison(FailureComparison::new(
                            "restart.budget",
                            "restarts available",
                            "exhausted",
                        ))
                        .with_note("process-exited"),
                ));
        }
        self.budget.consume_restart();
        let restart_count = self.budget.count();
        tracing::warn!(
            resource = %identity.resource_ref.to_canonical_string(),
            restart_count,
            "managed process exited; restarting under its restart policy"
        );
        self.durable.mark_exited();
        ctx.set_status(ProcessDriverStatus::AwaitingRestart { restart_count });
        let _ = ctx.requeue_after(restart_delay(spec, restart_count));
        // The row is not ready while the restart waits out its backoff: the
        // process this row claims is gone until the next pass relaunches it.
        Ok(ReconcileOutcome::RetryScheduled)
    }

    /// The one-shot arm (old `DesiredProcess::Ephemeral` per-record block).
    ///
    /// Preserved ordering: a terminal row only waits out its retention TTL; a
    /// live row that outlived its bounded runtime stops exactly and turns
    /// terminal `Failed`; a row this actor already started is observed
    /// through the liveness probe (`Alive` requeues, `Exited` reports
    /// `Succeeded`, `Unknown` reports `identity-ambiguous`); and a first-sight
    /// row runs the adoption classification (adopt, launch, exact stale
    /// replacement, or fail closed). A one-shot never restarts.
    async fn reconcile_ephemeral(
        &mut self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<ReconcileOutcome, ProcessDriverError> {
        if let Some(completion) = self.ephemeral.completed().await {
            return self.ephemeral_retention(ctx, spec, completion).await;
        }

        // The same single preparation the long-running arm runs. Nothing here
        // branches on the lifetime: the row's own reference decides it inside
        // the plan, and both arms then follow the same path (AE20, AE28).
        let binding = self.prepare_launch(ctx, identity, DriverOp::Reconcile).await?;

        // Runtime deadline (old `ephemeral runtime-deadline` arm): the process
        // this actor started outlived its bounded run, so it stops exactly and
        // the terminal outcome is `Failed`.
        if self.ephemeral.started()
            && let Some(started_at) = self.ephemeral.started_at().await
            && started_at.elapsed() >= Duration::from_millis(spec.runtime_deadline().as_millis())
        {
            if self.effects.has_active(
                &identity.zone,
                identity.zone_uid.as_ref(),
                &identity.resource_ref,
            ) {
                self.stop_and_finalize_ephemeral(identity, spec, DriverOp::Reconcile)
                    .await?;
            } else {
                // Nothing live to stop; the provider's exact finalization
                // still runs (old `stop_record` no-op + `finalize_resource`).
                self.effects
                    .finalize(identity)
                    .await
                    .map_err(|error| map_provider_error(error, DriverOp::Reconcile))?;
            }
            let completion = self.ephemeral.finish(true, "runtime-deadline").await;
            Self::publish_ephemeral_outcome(ctx, completion);
            ctx.set_status(ProcessDriverStatus::Failed {
                code: "runtime-deadline",
            });
            return self.ephemeral_retention(ctx, spec, completion).await;
        }

        // The launch binding gate, answered exactly as the long-running arm
        // answers it and AFTER the bounded-runtime stop above: a one-shot whose
        // runtime deadline elapsed still stops exactly, because the stop is
        // the fence, not a use of the authority (R18, R22). The two answers
        // that are not `Ready` stop a verified live one-shot on the same
        // terms the durable arm does (R21): the row stops reading ready, the
        // incarnation stops, and neither path launches.
        let lease = match &binding {
            ProcessBindingPreparation::NotRequired => None,
            ProcessBindingPreparation::Ready(lease) => Some(lease),
            ProcessBindingPreparation::Pending => {
                self.stop_withdrawn_one_shot(ctx, identity, spec).await?;
                let _ = ctx.requeue_after(PROCESS_RESYNC);
                return Ok(ReconcileOutcome::RetryScheduled);
            }
            ProcessBindingPreparation::Refused(error) => {
                self.stop_withdrawn_one_shot(ctx, identity, spec).await?;
                return Err(self.binding_gate_refused(identity, *error, DriverOp::Reconcile));
            }
        };

        // Steady state: the process this actor started is observed through
        // the preserved liveness probe (old `probe_record`), and `Exited` is
        // the one-shot's terminal exit - never a relaunch. `Unknown` (an
        // identity that no longer verifies) is the preserved terminal
        // `identity-ambiguous` failure.
        if self.ephemeral.started() {
            return match self.effects.probe_ephemeral(identity, spec).await {
                Ok(ProviderLiveness::Alive) => {
                    ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                    // Observe the bounded runtime and the exit while the row
                    // is live: the old descriptor resynced both Process types
                    // at 5s.
                    let _ = ctx.requeue_after(PROCESS_RESYNC);
                    Ok(ReconcileOutcome::Satisfied)
                }
                Ok(ProviderLiveness::Exited) => {
                    let completion = self.ephemeral.finish(false, "process-exited").await;
                    Self::publish_ephemeral_outcome(ctx, completion);
                    ctx.set_status(ProcessDriverStatus::Succeeded {
                        code: "process-exited",
                    });
                    self.ephemeral_retention(ctx, spec, completion).await
                }
                Ok(ProviderLiveness::Unknown) => {
                    let completion = self.ephemeral.finish(true, "identity-ambiguous").await;
                    Self::publish_ephemeral_outcome(ctx, completion);
                    ctx.set_status(ProcessDriverStatus::Failed {
                        code: "identity-ambiguous",
                    });
                    self.ephemeral_retention(ctx, spec, completion).await
                }
                Err(error) => Err(map_provider_error(error, DriverOp::Reconcile)),
            };
        }

        // The adoption classification is itself an effect this row's authority
        // is held to, so the sealed lease is revalidated before it runs and
        // not only before the launch that may follow it (KTD6, R18).
        if let Some(lease) = lease
            && self
                .revalidate_lease(ctx, identity, lease, DriverOp::Reconcile)
                .await
                .is_err()
        {
            let _ = ctx.requeue_after(PROCESS_RESYNC);
            return Ok(ReconcileOutcome::RetryScheduled);
        }

        // First sight of the row (first pass, or the first after a daemon
        // restart): the preserved adoption classification decides adopt (an
        // already-live identity), launch (absent), exact stale replacement,
        // or a fail-closed refusal.
        match self.effects.adopt_ephemeral(identity, spec).await {
            Ok(ProviderAdoption::Adopted(_)) => {
                self.ephemeral.mark_started().await;
                ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                let _ = ctx.requeue_after(PROCESS_RESYNC);
                Ok(ReconcileOutcome::Satisfied)
            }
            Ok(ProviderAdoption::Absent) => {
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_ephemeral_launch(ctx, identity, spec, lease).await
            }
            Ok(ProviderAdoption::Stale { candidate }) => {
                self.effects
                    .stop_stale(&identity.provider_ref, &candidate)
                    .await
                    .map_err(|error| map_provider_error(error, DriverOp::Reconcile))?;
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_ephemeral_launch(ctx, identity, spec, lease).await
            }
            Ok(ProviderAdoption::Quarantined(report)) => {
                Err(self.identity_ambiguous(DriverOp::Reconcile, &report))
            }
            // A one-shot ticket carries no controller bootstrap endpoint, so
            // this classification is the old `start_record_plan` refusal
            // (terminal template unavailability), never a restart.
            Ok(ProviderAdoption::ControllerBootstrapMissing) => Err(self.error(
                ProcessDriverErrorKind::TemplateUnavailable,
                DriverOp::Reconcile,
            )),
            Err(error) => Err(map_provider_error(error, DriverOp::Reconcile)),
        }
    }

    /// The one-shot retention wait (old `ephemeral_ttl_elapsed` +
    /// `request_delete`). R11: the TTL clock is the driver's runtime memory,
    /// never a persisted status field; the manager owns row retirement, so an
    /// elapsed TTL asks it to delete this row and the delete pass runs the
    /// preserved stop/finalize cleanup. `incidentHold` blocks a failed row's
    /// cleanup until an explicit release, exactly as the old TTL gate did.
    async fn ephemeral_retention(
        &mut self,
        ctx: &mut ResourceContext,
        spec: &EphemeralProcessSpec,
        completion: EphemeralCompletion,
    ) -> Result<ReconcileOutcome, ProcessDriverError> {
        // The terminal outcome rides the projection for the whole retention
        // window: every pass in the window republishes it, so a Device port
        // gating on the row reads the same outcome the terminal pass
        // published.
        Self::publish_ephemeral_outcome(ctx, completion);
        ctx.set_status(if completion.failed {
            ProcessDriverStatus::Failed {
                code: completion.code,
            }
        } else {
            ProcessDriverStatus::Succeeded {
                code: completion.code,
            }
        });
        if completion.failed && spec.incident_hold() {
            return Ok(ReconcileOutcome::Satisfied);
        }
        let ttl = Duration::from_millis(if completion.failed {
            spec.failed_ttl().as_millis()
        } else {
            spec.successful_ttl().as_millis()
        });
        let elapsed = completion.at.elapsed();
        if elapsed < ttl {
            let _ = ctx.requeue_after(ttl - elapsed);
            return Ok(ReconcileOutcome::Satisfied);
        }
        let key = ctx.key().clone();
        ctx.delete(&key)
            .await
            .map_err(|_| self.error(ProcessDriverErrorKind::ProviderEffect, DriverOp::Reconcile))?;
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Publish one one-shot terminal outcome as the row's wire-visible status
    /// projection (R11: in-memory, dropped with the next status).
    ///
    /// A driver pass that concluded publishes the runtime's ready
    /// classification, and the closed wire vocabulary has no "succeeded"
    /// phase, so the terminal outcome a Device port must gate on is only
    /// observable as this projection layer:
    /// `{"ephemeral": {"state": "succeeded" | "failed", "code": "<closed>"}}`.
    /// A reader that needs a completed one-shot reads that layer, never the
    /// phase alone.
    fn publish_ephemeral_outcome(ctx: &mut ResourceContext, completion: EphemeralCompletion) {
        ctx.set_status_projection(serde_json::json!({
            "ephemeral": {
                "state": if completion.failed { "failed" } else { "succeeded" },
                "code": completion.code,
            },
        }));
    }

    /// Preserved one-shot stop: the fixed 30s term, the 30s kill budget, then
    /// the provider's exact finalization (old `stop_record` ephemeral arm).
    async fn stop_and_finalize_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
        op: DriverOp,
    ) -> Result<(), ProcessDriverError> {
        tracing::warn!(
            resource = %identity.resource_ref.to_canonical_string(),
            operation = ?op,
            "stopping a managed one-shot process for its driver operation"
        );
        self.effects
            .stop_ephemeral(identity, spec, EPHEMERAL_TERM_TIMEOUT, KILL_TIMEOUT)
            .await
            .map_err(|error| map_provider_error(error, op))?;
        self.effects
            .finalize(identity)
            .await
            .map_err(|error| map_provider_error(error, op))?;
        Ok(())
    }

    /// Spawn the one-shot signed-ticket launch as a long effect (R5, KTD12);
    /// completion arrives as [`EffectCompleted`] with the operation id. A
    /// one-shot row has no restart policy, so a refused launch is terminal
    /// (old `handle_start_failure` with no ephemeral restart arm) and the
    /// start is remembered only on success.
    ///
    /// The sealed binding lease is revalidated HERE, immediately before the
    /// effect under this pass's own serialized manager boundary, for the same
    /// reason the durable arm does it there (KTD6, R18).
    async fn spawn_ephemeral_launch(
        &mut self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
        lease: Option<&BindingAuthorityLease>,
    ) -> Result<ReconcileOutcome, ProcessDriverError> {
        if let Some(lease) = lease
            && self
                .revalidate_lease(ctx, identity, lease, DriverOp::Reconcile)
                .await
                .is_err()
        {
            tracing::warn!(
                resource = %identity.resource_ref.to_canonical_string(),
                "the launch authority moved before the one-shot launch effect; issuing none"
            );
            let _ = ctx.requeue_after(PROCESS_RESYNC);
            return Ok(ReconcileOutcome::RetryScheduled);
        }
        let operation = ctx.begin_operation();
        let effects = Arc::clone(&self.effects);
        let effect_sender = ctx.effect_sender();
        let ephemeral = Arc::clone(&self.ephemeral);
        // The bounded start deadline is the launch budget (old
        // `launch_timeout` for the ephemeral arm).
        let timeout = Duration::from_millis(spec.start_deadline().as_millis());
        let task_spec = spec.clone();
        let identity = identity.clone();
        tokio::spawn(async move {
            let effect_result = match effects
                .launch_ephemeral(&identity, &task_spec, timeout)
                .await
            {
                Ok(_) => {
                    ephemeral.mark_started().await;
                    EffectResult::Completed
                }
                Err(error) => {
                    // The provider's reason is the only diagnostic: status is
                    // memory-only (R11) and the closed classification is all
                    // that reaches the actor. `ResourceRef`'s `Display` is the
                    // redaction stub, so both refs render canonically.
                    tracing::warn!(
                        resource = %identity.resource_ref.to_canonical_string(),
                        provider = %identity.provider_ref.to_canonical_string(),
                        error = %error,
                        "ephemeral process launch failed"
                    );
                    EffectResult::Failed(
                        DriverFailure::refused(
                            DriverOp::Reconcile,
                            provider_error_kind(&error).failure_kind(),
                        )
                        .at("reconcile/launch")
                        .with_comparison(FailureComparison::new(
                            "launch.attempt",
                            "accepted",
                            "failed",
                        ))
                        .with_note(error),
                    )
                }
            };
            let _ = effect_sender.send(EffectCompleted {
                operation,
                result: effect_result,
            });
        });
        Ok(ReconcileOutcome::InProgress { operation })
    }

    /// One-shot teardown: deletion adopts first (old `deletion_adoption`),
    /// then stops the exact live identity or the uniquely identified stale
    /// candidate, and finalizes the provider's local authority. An ambiguous
    /// identity refuses destructive action.
    async fn delete_ephemeral(
        &mut self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<(), ProcessDriverError> {
        match self.effects.adopt_ephemeral(identity, spec).await {
            Ok(ProviderAdoption::Adopted(_)) => {
                self.stop_and_finalize_ephemeral(identity, spec, DriverOp::Delete)
                    .await
            }
            Ok(ProviderAdoption::Stale { candidate }) => self
                .effects
                .stop_stale(&identity.provider_ref, &candidate)
                .await
                .map_err(|error| map_provider_error(error, DriverOp::Delete)),
            Ok(ProviderAdoption::Absent) | Ok(ProviderAdoption::ControllerBootstrapMissing) => {
                Ok(())
            }
            Ok(ProviderAdoption::Quarantined(report)) => {
                Err(self.identity_ambiguous(DriverOp::Delete, &report))
            }
            // A row no host-minted ticket can describe (a Guest-owned one-shot
            // outside the guest VMM chain, e.g. the projected
            // `store-preflight-<guest>` intent) has no identity this daemon
            // could ever have realized, so its deletion converges with no
            // provider effect. The guard's real intent is preserved: nothing
            // is stopped, so no process the row does not claim is ever
            // touched.
            Err(error) if error.contains("guest-process-not-vmm") => {
                tracing::warn!(
                    resource = %identity.resource_ref.to_canonical_string(),
                    error = %error,
                    "one-shot delete converged without provider effects: no host ticket can describe this row"
                );
                Ok(())
            }
            Err(error) => Err(map_provider_error(error, DriverOp::Delete)),
        }
    }
}

/// The target-local handle one Guest realization is recorded under.
///
/// A logical name inside the target, derived from the Host-zone source the
/// realization belongs to - never a Host path, and never a second identity
/// for the resource: the Guest keys the record on the source key the
/// assignment carries, and this names that same row from the target's side.
fn guest_local_handle(source: &ResourceKey) -> String {
    format!("d2b/process/{}/{}", source.type_name, source.name)
}

/// Assemble the host-resolved target-local realization one launch carries
/// (R18, R29).
///
/// The resolved Process spec travels verbatim and the prepared
/// `EndpointBinding` deliveries travel exactly as the sealed lease recorded
/// them. The Host composes this only after that lease revalidated in this
/// pass, so a stale or revoked lease reaches the Guest as no realization at
/// all rather than as a process started over bindings that moved (KTD6).
fn guest_realization(
    identity: &ProcessResourceIdentity,
    spec: &ProcessSpec,
    lease: Option<&BindingAuthorityLease>,
) -> Result<GuestProcessRealization, ProcessDriverError> {
    let resolved = serde_json::to_vec(spec)
        .map_err(|_| guest_realization_refused("the resolved spec does not serialize"))?;
    if resolved.is_empty() {
        return Err(guest_realization_refused("the resolved spec is empty"));
    }
    let mut deliveries = Vec::new();
    if let Some(lease) = lease {
        for row in lease.rows() {
            let expectation = row.expectation();
            deliveries.push(
                GuestBindingDelivery::new(
                    expectation.binding_ref().to_canonical_string(),
                    expectation.endpoint_ref().to_canonical_string(),
                    expectation.slot(),
                    expectation.incarnation(),
                )
                .map_err(|error| guest_realization_refused(error.code()))?,
            );
        }
    }
    Ok(GuestProcessRealization::new(
        identity.resource_ref.to_canonical_string(),
        resolved,
        deliveries,
    ))
}

/// One refusal to compose a target-local realization.
fn guest_realization_refused(code: &'static str) -> ProcessDriverError {
    ProcessDriverError::new(ProcessDriverErrorKind::SpecInvalid, DriverOp::Reconcile)
        .with_detail(FailureDetail::at("guest/realization").with_note(code))
}

/// Classify one failed launch against the in-memory restart budget (spec
/// section 32).
///
/// The closed classification is what reaches status, and status is
/// memory-only (R11), so the journal is the only place the provider's reason
/// for refusing the launch is observable. `ResourceRef`'s `Display` is the
/// redaction stub, so both refs render canonically: the redacting form would
/// make the only diagnostic for a refused launch unreadable.
fn launch_failure(
    budget: &RestartBudget,
    identity: &ProcessResourceIdentity,
    spec: &ProcessSpec,
    error: String,
) -> EffectResult {
    tracing::warn!(
        resource = %identity.resource_ref.to_canonical_string(),
        provider = %identity.provider_ref.to_canonical_string(),
        error = %error,
        "process launch failed"
    );
    let kind = provider_error_kind(&error);
    if kind.is_unresolvable_launch() {
        // The closed spellings no retry can reverse (`template-not-found`,
        // `resolution-failed`, `guest-process-not-vmm`): the in-memory budget
        // cannot mint the missing ticket, so the row fails instead of
        // relaunching (and warning) forever. The ephemeral arm classifies its
        // launch the same way.
        EffectResult::Failed(
            DriverFailure::refused(DriverOp::Reconcile, kind.failure_kind())
                .at("reconcile/launch")
                .with_comparison(FailureComparison::new(
                    "launch.attempt",
                    "accepted",
                    "failed",
                ))
                .with_note(error),
        )
    } else if budget.allows(spec) {
        budget.record_restart();
        EffectResult::Failed(
            DriverFailure::error(
                DriverOp::Reconcile,
                FailureKinds::PROCESS_PROVIDER_EFFECT_FAILED,
                FailureClass::Retryable,
            )
            .at("reconcile/launch")
            .with_comparison(FailureComparison::new(
                "launch.attempt",
                "accepted",
                "failed",
            ))
            .with_note(error),
        )
    } else {
        budget.mark_exhausted();
        EffectResult::Failed(
            DriverFailure::refused(
                DriverOp::Reconcile,
                FailureKinds::PROCESS_START_BUDGET_EXHAUSTED,
            )
            .at("reconcile/launch")
            .with_comparison(FailureComparison::new(
                "restart.budget",
                "restarts available",
                "exhausted",
            ))
            .with_note(error),
        )
    }
}

/// Map preserved provider error spellings onto the closed driver kinds (the
/// same classification as the old `map_provider_error`, split for issue #508
/// so each distinct cause reports its own kind).
///
/// `resolution-failed` is the supervisor's code for a launch ticket the
/// trusted bundle refuses to resolve (no matching intent, wrong execution
/// target/scope/descriptor posture). Nothing was launched and nothing was
/// observed, so the probe answer is the ticket's resolution failing - the same
/// terminal class as `template-not-found`, never an ambiguous identity to
/// quarantine (R15 is about observed identity). It stays a kind of its own.
fn map_provider_error(error: String, op: DriverOp) -> ProcessDriverError {
    tracing::warn!(operation = ?op, error = %error, "process provider effect failed");
    let kind = provider_error_kind(&error);
    ProcessDriverError::new(kind, op).with_detail(
        FailureDetail::at("provider/effect")
            .comparison(FailureComparison::new(
                "provider.effect",
                "accepted",
                kind.failure_kind().code(),
            ))
            .with_note(error),
    )
}

/// The closed kind one provider error spelling names, shared by the effect
/// classification and the effect completions that set their own retry class
/// (issue #508: the kind names what failed, the class stays the driver's).
fn provider_error_kind(error: &str) -> ProcessDriverErrorKind {
    if error.contains("template-not-found") {
        ProcessDriverErrorKind::TemplateUnavailable
    } else if error.contains("resolution-failed") {
        ProcessDriverErrorKind::ResolutionRefused
    } else if error.contains("guest-process-not-vmm") {
        // The trusted bundle holds no host-minted intent for this row at all
        // (a Guest-owned one-shot outside the guest VMM chain, e.g. a
        // projected preflight intent): no retry can ever mint a ticket, so
        // the refusal is terminal - never an ambiguous identity to quarantine
        // (R15 is about observed identity).
        ProcessDriverErrorKind::GuestProcessNotVmm
    } else if error.contains("quarantined")
        || error.contains("identity")
        || error.contains("ambiguous")
    {
        ProcessDriverErrorKind::IdentityAmbiguous
    } else {
        ProcessDriverErrorKind::ProviderEffect
    }
}

#[async_trait::async_trait]
impl ResourceDriver for ProcessDriver {
    type Error = ProcessDriverError;

    fn classify_error(&self, error: &ProcessDriverError) -> DriverFailure {
        let failure = match error.kind {
            ProcessDriverErrorKind::ProviderEffect => {
                DriverFailure::error(error.op, error.kind.failure_kind(), FailureClass::Retryable)
            }
            ProcessDriverErrorKind::DrainPending => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            ProcessDriverErrorKind::SpecInvalid
            | ProcessDriverErrorKind::IdentityIncomplete
            | ProcessDriverErrorKind::ProviderUnsupported
            | ProcessDriverErrorKind::ExecutionUnsupported
            | ProcessDriverErrorKind::TemplateUnavailable
            | ProcessDriverErrorKind::ResolutionRefused
            | ProcessDriverErrorKind::GuestProcessNotVmm
            | ProcessDriverErrorKind::IdentityAmbiguous
            | ProcessDriverErrorKind::StartExhausted => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Spec decode plus provider reference and execution-target checks.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let (envelope, spec) = self.decoded_spec(ctx, DriverOp::Validate)?;
        self.check_provider(&envelope, DriverOp::Validate)?;
        // The gate is a statement about WHERE this row's effects run, and a row
        // the manager committed to a Guest target answers that itself: its
        // whole lifecycle rides the authenticated target session its own
        // `TargetBinding` carries, so a Host-mode plane drives it over exactly
        // the transport a Guest-mode plane would (KTD11, R29). The gate
        // therefore fences what is LEFT - rows that would run locally and have
        // no target session to run over - which keeps the Host/Guest pairing
        // symmetric in both modes and leaves every row carrying no Guest
        // binding refused exactly as before.
        let on_guest_target = ctx.target().is_some_and(TargetBinding::is_guest);
        if !on_guest_target
            && !execution_target_allowed(self.authority.mode, spec.execution().execution_ref())
        {
            return Err(self
                .error(
                    ProcessDriverErrorKind::ExecutionUnsupported,
                    DriverOp::Validate,
                )
                .with_detail(FailureDetail::at("spec/execution").comparison(
                    FailureComparison::new(
                        "spec.executionRef",
                        "a target this daemon mode drives",
                        spec.execution().execution_ref().to_canonical_string(),
                    ),
                )));
        }
        // A row committed to a Guest target runs its whole lifecycle over the
        // authenticated target session, and that transport covers the durable
        // `Process` arm only. A one-shot has no target-local lifecycle yet, so
        // it is refused here rather than launched through the local Provider
        // effects of a target it does not run on (R19).
        if matches!(spec, ProcessFamilySpec::Ephemeral(_))
            && ctx.target().is_some_and(|target| target.is_guest())
        {
            return Err(self
                .error(
                    ProcessDriverErrorKind::ExecutionUnsupported,
                    DriverOp::Validate,
                )
                .with_detail(FailureDetail::at("spec/execution").comparison(
                    FailureComparison::new(
                        "spec.resourceType",
                        "a type with a target-local lifecycle",
                        EPHEMERAL_PROCESS_TYPE_NAME,
                    ),
                )));
        }
        Ok(())
    }

    /// Probe and classify on the realization target (R15, R16): exact match
    /// adopts, missing waits for reconcile, drifted/ambiguous quarantines.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let (envelope, spec) = self.decoded_spec(ctx, DriverOp::Recover)?;
        self.check_provider(&envelope, DriverOp::Recover)?;
        let identity = self
            .identity(
                ctx,
                envelope.provider_ref.as_ref().expect("checked"),
                DriverOp::Recover,
            )
            .await?;

        match &spec {
            ProcessFamilySpec::Ephemeral(ephemeral) => {
                self.recover_ephemeral(ctx, &identity, ephemeral).await
            }
            ProcessFamilySpec::Process(process) => {
                if process.desired_lifecycle() == DesiredLifecycle::Stopped {
                    ctx.set_status(ProcessDriverStatus::Succeeded {
                        code: "process-stopped",
                    });
                    return Ok(RecoveryOutcome::Missing);
                }
                // The launch binding gate, answered before this pass acts at
                // all (KTD6, R18). Adoption is an effect this row's authority
                // is held to exactly as the launch is, and a restart is the
                // pass that most needs the fence: a delivery withdrawn while
                // this daemon was down is invisible to every effect it has
                // already issued. `Pending` therefore reports `Missing` and
                // touches nothing - a survivor is not a verified identity, so
                // unproven evidence is no reason to stop one either - and
                // `Refused` is the same terminal refusal the reconcile arms
                // report. `Ready` carries the sealed lease, which the
                // adoption revalidates immediately before it runs.
                let binding = self.prepare_launch(ctx, &identity, DriverOp::Recover).await?;
                let lease = match &binding {
                    ProcessBindingPreparation::NotRequired => None,
                    ProcessBindingPreparation::Ready(lease) => Some(lease),
                    ProcessBindingPreparation::Pending => return Ok(RecoveryOutcome::Missing),
                    ProcessBindingPreparation::Refused(error) => {
                        return Err(self.binding_gate_refused(&identity, *error, DriverOp::Recover));
                    }
                };
                // Discovery runs on the realization target: for a
                // Guest-targeted row that is the authenticated session, and a
                // restart survivor is adopted only because the live target
                // generation said so (F5, R21, R29).
                let mut guest =
                    self.guest_arm(ctx.target().cloned(), DriverOp::Recover).await?;
                if process.adoption_policy() == AdoptionPolicy::NeverAdopt {
                    // NeverAdopt never adopts; an unexpected live identity is stopped
                    // exactly (preserved behavior) and the next launch starts fresh.
                    if self.has_active_process(guest.as_ref(), &identity, DriverOp::Recover).await? {
                        self.stop_process(guest.as_mut(), &identity, process, DriverOp::Recover)
                            .await?;
                    }
                    return Ok(RecoveryOutcome::Missing);
                }

                // The adoption effect revalidates the sealed lease against
                // freshly read evidence under this pass's own boundary,
                // exactly as the reconcile arms do before the same
                // classification: a lease that moved since it was sealed
                // adopts nothing and signals nothing.
                if let Some(lease) = lease
                    && self
                        .revalidate_lease(ctx, &identity, lease, DriverOp::Recover)
                        .await
                        .is_err()
                {
                    return Ok(RecoveryOutcome::Missing);
                }
                match self
                    .adopt_process(guest.as_mut(), &identity, process, DriverOp::Recover)
                    .await?
                {
                    AdoptionOutcome::Adopted => {
                        // The first reconcile pass right after recovery probes
                        // this identity: mark it so that pass observes
                        // liveness and arms the cadence instead of re-adopting.
                        self.durable.mark_watching();
                        ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                        Ok(RecoveryOutcome::Adopted)
                    }
                    // A static controller without its exact bootstrap endpoint
                    // has nothing to adopt; reconcile restarts it. So does a
                    // realization that is present but still converging: the
                    // first reconcile observes it instead of launching over it.
                    AdoptionOutcome::Missing
                    | AdoptionOutcome::StopAndRestart
                    | AdoptionOutcome::Converging => Ok(RecoveryOutcome::Missing),
                    // A restart survivor this daemon cannot attribute exactly,
                    // and one available for exact replacement, both stay out of
                    // the adoption path: quarantine and let the evidence settle
                    // (R15, R18).
                    AdoptionOutcome::Stale(_) | AdoptionOutcome::Quarantined(_) => {
                        ctx.set_status(ProcessDriverStatus::Quarantined {
                            code: "identity-ambiguous",
                        });
                        Ok(RecoveryOutcome::Quarantined)
                    }
                }
            }
        }
    }

    /// One reconcile pass: probe, then adopt/launch/stop-stale per the
    /// preserved classification. Launches spawn as long effects (R5).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let (envelope, spec) = self.decoded_spec(ctx, DriverOp::Reconcile)?;
        self.check_provider(&envelope, DriverOp::Reconcile)?;
        let identity = self
            .identity(
                ctx,
                envelope.provider_ref.as_ref().expect("checked"),
                DriverOp::Reconcile,
            )
            .await?;
        match &spec {
            ProcessFamilySpec::Ephemeral(ephemeral) => {
                self.reconcile_ephemeral(ctx, &identity, ephemeral).await
            }
            ProcessFamilySpec::Process(process) => {
                self.reconcile_process(ctx, identity, process).await
            }
        }
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown. The call nudges each owned child through its
    /// own finalize-before-delete pass and requeues this pass while any child
    /// row is still live. Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources().await.map_err(|_| {
            self.error(ProcessDriverErrorKind::DrainPending, DriverOp::Delete)
                .with_detail(
                    FailureDetail::at("delete/drain").comparison(FailureComparison::new(
                        "owned.children",
                        "retired",
                        "still live",
                    )),
                )
        })?;
        Ok(())
    }

    /// Teardown (old prepare/execute/finalize fold). Idempotent under retry
    /// (R10): the durable deleting mark is already committed. Deletion adopts
    /// first (preserved `deletion_adoption` behavior): the exact live identity
    /// stops through the preserved term-then-kill escalation with pidfd
    /// retry, a stale candidate after a daemon restart stops through its
    /// adoption evidence, and an absent process converges without effects.
    /// An ambiguous identity refuses destructive action (old
    /// `stale_candidate_for_deletion`).
    ///
    /// Two stages, and the second one is what makes the first safe (R22). The
    /// EFFECT stops here exactly as it always has, immediately, whatever the
    /// surrounding graph looks like. The ROW retires only after
    /// [`Self::retain_until_dependencies_retired`] finds no committed
    /// relationship this row consumes and no `Endpoint` it produces, because
    /// the release of the first depends on this row's consumer identity still
    /// being there to resolve.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let Ok((envelope, spec)) = self.decoded_spec(ctx, DriverOp::Delete) else {
            // Nothing launchable to clean up (old behavior: absent record
            // converged).
            return Ok(());
        };
        let identity = match self
            .identity(
                ctx,
                envelope.provider_ref.as_ref().expect("checked"),
                DriverOp::Delete,
            )
            .await
        {
            Ok(identity) => identity,
            Err(_) => return Ok(()),
        };

        match &spec {
            ProcessFamilySpec::Ephemeral(ephemeral) => {
                self.delete_ephemeral(&identity, ephemeral).await?;
                return self.retain_until_dependencies_retired(ctx, &identity).await;
            }
            ProcessFamilySpec::Process(process) => {
                // Teardown reaches the realization target too, so a Guest-targeted
                // row removes exactly its own target-local process and nothing
                // else realized on the same Guest (R20, R29).
                let mut guest = self.guest_arm(ctx.target().cloned(), DriverOp::Delete).await?;
                if process.adoption_policy() == AdoptionPolicy::NeverAdopt {
                    // NeverAdopt never adopts; an unexpected live identity stops
                    // exactly through its retained authority.
                    if self.has_active_process(guest.as_ref(), &identity, DriverOp::Delete).await? {
                        self.stop_process(guest.as_mut(), &identity, process, DriverOp::Delete)
                            .await?;
                    }
                    return self.retain_until_dependencies_retired(ctx, &identity).await;
                }

                let stopped = match self
                    .adopt_process(guest.as_mut(), &identity, process, DriverOp::Delete)
                    .await?
                {
                    // Both mean the target holds this row's exact realization,
                    // so the teardown removes exactly it; a repeated delete, or
                    // one after a reconnect, finds it already gone and
                    // converges.
                    AdoptionOutcome::Adopted | AdoptionOutcome::Converging => {
                        self.stop_process(guest.as_mut(), &identity, process, DriverOp::Delete)
                            .await
                    }
                    AdoptionOutcome::Stale(candidate) => self
                        .effects
                        .stop_stale(&identity.provider_ref, &candidate)
                        .await
                        .map_err(|error| map_provider_error(error, DriverOp::Delete)),
                    AdoptionOutcome::Missing | AdoptionOutcome::StopAndRestart => {
                        // Nothing this daemon can stop exactly (old deletion treated
                        // a missing exact identity as converged without effects).
                        Ok(())
                    }
                    AdoptionOutcome::Quarantined(Some(report)) => {
                        Err(self.identity_ambiguous(DriverOp::Delete, &report))
                    }
                    AdoptionOutcome::Quarantined(None) => {
                        ctx.set_status(ProcessDriverStatus::Quarantined {
                            code: "identity-ambiguous",
                        });
                        Err(self
                            .error(ProcessDriverErrorKind::IdentityAmbiguous, DriverOp::Delete)
                            .with_detail(FailureDetail::at("delete/identity").comparison(
                                FailureComparison::new(
                                    "delete.identity",
                                    "exactly one attributable realization",
                                    "unattributable",
                                ),
                            )))
                    }
                };
                stopped?;
                self.retain_until_dependencies_retired(ctx, &identity).await
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a scripted fake effect port (R4: the provider
// effect shapes are exercised exactly as the production port defines them).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::effects::{ProviderAdoption, ProviderLiveness};
    use crate::execution::ExecutionMode;
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::process::ProcessSpec;
    use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceUid, ZoneId};
    use d2b_process_conformance::testing::fixtures;
    use d2b_process_conformance::{
        AdoptionCandidate, AdoptionCondition, IdentityBinding, ObservedIdentity,
        ProcessIdentityDigest, ProcessPhaseClass, ProcessStatusReport, WaitReapOwner,
    };
    use d2b_resource_runtime::guest_target::{
        GuestAdoption, TargetInstanceState, TargetResourceInstance,
    };
    use d2b_provider_toolkit::testing::fakes::RecordingRequeue;
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, ResourceContext, WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{
        DriverFailure, DriverOp, FailureClass, FailureKinds, ResourceError,
    };
    use d2b_resource_runtime::identity::{
        ResourceKey, ResourceProvenance, ResourceTypeName, StoredDesiredResource,
    };
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::{TargetBinding, TargetDirectory, TargetObservation};
    use tokio::sync::mpsc;

    use super::{
        AllowedSources, EPHEMERAL_PROCESS_TYPE_NAME, PROCESS_TYPE_NAME, ProcessDriver,
        ProcessDriverArgs, ProcessDriverErrorKind, ProcessDriverFactory, ProcessDriverStatus,
        process_family_descriptors, process_spec_decoder, restart_delay,
    };

    use crate::test_support::{FakeFacets, FakeFacetsConfig};

    // -- fixtures ------------------------------------------------------------

    const ZONE_UID: &str = "123e4567-e89b-42d3-a456-426614174000";

    fn spec_bytes(restart_policy: Option<&str>) -> Vec<u8> {
        let mut spec = String::from(
            r#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"reaction","drainTimeout":"250ms""#,
        );
        if let Some(policy) = restart_policy {
            spec.push_str(&format!(r#","restartPolicy":{policy}"#));
        }
        spec.push('}');
        spec.into_bytes()
    }

    fn test_row() -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("work", "Process", "worker"),
            uid: [0x42; 16],
            generation: 3,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: spec_bytes(None),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// One durable row that refuses adoption: a live identity this actor did
    /// not launch must be stopped and replaced.
    fn never_adopt_row() -> StoredDesiredResource {
        let mut row = test_row();
        row.spec = br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"reaction","drainTimeout":"250ms","adoptionPolicy":"never-adopt"}"#
            .to_vec();
        row
    }

    /// One durable row whose desired lifecycle is `stopped`: the row is
    /// realized only once the process is observed gone, never by asserting it.
    fn stopped_row() -> StoredDesiredResource {
        let mut row = test_row();
        row.spec = br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"reaction","drainTimeout":"250ms","desiredLifecycle":"stopped"}"#
            .to_vec();
        row
    }

    /// One one-shot row: the activation-runner shape the NixosGeneration
    /// driver mints (KTD13) - the typed `activationInput` on the sanctioned
    /// channel plus an explicit runtime/retention policy.
    fn ephemeral_spec_bytes(
        start_deadline: &str,
        runtime_deadline: &str,
        successful_ttl: &str,
        failed_ttl: &str,
        incident_hold: bool,
    ) -> Vec<u8> {
        format!(
            r#"{{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"activation-nixos-runner","activationInput":{{"systemArtifactId":"system-artifact","targetGeneration":1,"activationMode":"switch"}},"startDeadline":"{start_deadline}","runtimeDeadline":"{runtime_deadline}","successfulTtl":"{successful_ttl}","failedTtl":"{failed_ttl}","incidentHold":{incident_hold}}}"#
        )
        .into_bytes()
    }

    fn ephemeral_row() -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new(
                "work",
                "EphemeralProcess",
                "activation-nixos--runner--gen-1",
            ),
            uid: [0x43; 16],
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: ephemeral_spec_bytes("7s", "5m", "1h", "24h", false),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    fn adopted_report() -> ProcessStatusReport {
        ProcessStatusReport {
            provider: BoundedToken::parse("system-minijail").expect("provider token"),
            identity: ProcessIdentityDigest::from_bytes([0x51; 32]),
            wait_reap_owner: WaitReapOwner::Local,
            execution_ref: d2b_contracts_resource::v3::ResourceRef::parse("Host/host-system")
                .expect("execution ref"),
            domain: d2b_contracts_resource::v3::execution_policy::ExecutionDomain::System,
            user_ref: None,
            digests: fixtures::compiled_digests(),
            phase: ProcessPhaseClass::Ready,
            last_exit: None,
            adoption: AdoptionCondition::Adopted,
        }
    }

    fn quarantined_report() -> ProcessStatusReport {
        let mut report = adopted_report();
        report.phase = ProcessPhaseClass::Unknown;
        report.adoption = AdoptionCondition::Quarantined;
        report
    }

    fn stale_candidate() -> AdoptionCandidate {
        AdoptionCandidate {
            identity: ProcessIdentityDigest::from_bytes([0x42; 32]),
            observed: ObservedIdentity::from_verified([
                IdentityBinding::Pid,
                IdentityBinding::ProcessStartTime,
                IdentityBinding::Cgroup,
                IdentityBinding::Template,
                IdentityBinding::Generation,
            ]),
            wait_reap_owner: WaitReapOwner::Local,
        }
    }

    /// Dead manager: these Process flows make no manager calls.
    struct DeadManager;

    #[async_trait::async_trait]
    impl ManagerEndpoint for DeadManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }

        async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<d2b_resource_runtime::context::WatchId, ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }

        async fn cancel_watch(
            &self,
            _watch: d2b_resource_runtime::context::WatchId,
        ) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerUnavailable("dead".into()))
        }
    }

    /// Owner-scoped manager double for the finalize gate: one scripted owned
    /// row set; `delete` records the retirement nudge and removes the row.
    struct OwnershipManager {
        owned: parking_lot::Mutex<Vec<StoredDesiredResource>>,
        rows: parking_lot::Mutex<Vec<StoredDesiredResource>>,
        deleted: parking_lot::Mutex<Vec<ResourceKey>>,
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl OwnershipManager {
        /// An owner-scoped manager with no owned rows: the retention delete
        /// path's double.
        fn empty() -> Arc<Self> {
            Arc::new(Self {
                owned: parking_lot::Mutex::new(Vec::new()),
                rows: parking_lot::Mutex::new(Vec::new()),
                deleted: parking_lot::Mutex::new(Vec::new()),
            })
        }

        fn with_owned(row: StoredDesiredResource) -> Arc<Self> {
            Arc::new(Self {
                owned: parking_lot::Mutex::new(vec![row]),
                rows: parking_lot::Mutex::new(Vec::new()),
                deleted: parking_lot::Mutex::new(Vec::new()),
            })
        }

        /// Serve one row by key (`get`), for rows the driver reads besides its
        /// own (the owning `VolumeBinding` a serving worker resolves).
        fn with_row(self: &Arc<Self>, row: StoredDesiredResource) -> Arc<Self> {
            self.rows.lock().push(row);
            Arc::clone(self)
        }
    }

    #[async_trait::async_trait]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl ManagerEndpoint for OwnershipManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            Err(ResourceError::ManagerRejected { reason: "unexpected ensure_child".into() })
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self.rows.lock().iter().find(|row| row.key == *key).cloned()) // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            Err(ResourceError::ManagerRejected { reason: "unexpected view".into() })
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.deleted.lock().push(key.clone()); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            self.owned.lock().retain(|row| row.key != *key); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            Ok(())
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self.owned.lock().clone())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<d2b_resource_runtime::context::WatchId, ResourceError> {
            Err(ResourceError::ManagerRejected {
                reason: "unexpected register_watch".into(),
            })
        }

        async fn cancel_watch(
            &self,
            _watch: d2b_resource_runtime::context::WatchId,
        ) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    struct Fixture {
        ctx: ResourceContext,
        effects: mpsc::UnboundedReceiver<d2b_resource_runtime::context::EffectCompleted>,
        requeues: mpsc::UnboundedReceiver<u64>,
        requeue: RecordingRequeue,
        row: StoredDesiredResource,
    }

    fn fixture(row: StoredDesiredResource) -> Fixture {
        fixture_with(row, Arc::new(DeadManager))
    }

    fn fixture_with(row: StoredDesiredResource, manager: Arc<dyn ManagerEndpoint>) -> Fixture {
        fixture_owned_by(row, manager, None)
    }

    /// Fixture whose row resolves the given manager owner key (the shape the
    /// manager attaches for an owned child).
    fn fixture_owned_by(
        row: StoredDesiredResource,
        manager: Arc<dyn ManagerEndpoint>,
        owner_key: Option<ResourceKey>,
    ) -> Fixture {
        let (effects_tx, effects_rx) = mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = mpsc::unbounded_channel();
        let (requeue, requeue_rx) = RecordingRequeue::new();
        let ctx = ResourceContext::new(
            row.clone(),
            process_spec_decoder(),
            manager,
            Arc::new(requeue.clone()),
            effects_tx,
            notify_tx,
        )
        .with_owner_key(owner_key);
        Fixture {
            ctx,
            effects: effects_rx,
            requeues: requeue_rx,
            requeue: requeue.clone(),
            row,
        }
    }

    /// Fixture whose row carries the committed target binding the manager
    /// attaches at the commit-then-spawn boundary (R19, R29).
    fn fixture_targeted(
        row: StoredDesiredResource,
        manager: Arc<dyn ManagerEndpoint>,
        target: TargetBinding,
    ) -> Fixture {
        let (effects_tx, effects_rx) = mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = mpsc::unbounded_channel();
        let (requeue, requeue_rx) = RecordingRequeue::new();
        let ctx = ResourceContext::new(
            row.clone(),
            process_spec_decoder(),
            manager,
            Arc::new(requeue.clone()),
            effects_tx,
            notify_tx,
        )
        .with_target(target);
        Fixture {
            ctx,
            effects: effects_rx,
            requeues: requeue_rx,
            requeue: requeue.clone(),
            row,
        }
    }

    impl Fixture {
        fn requeue_calls(&self) -> Vec<Duration> {
            self.requeue.scheduled()
        }
    }

    fn driver_args(effects: Arc<FakeFacets>) -> ProcessDriverArgs {
        ProcessDriverArgs {
            zone: ZoneId::parse("work").expect("zone"),
            facets: effects.facet_set(),
            zone_uid: Some(ResourceUid::parse(ZONE_UID).expect("zone uid")),
            policy_revision: Some(7),
            provider_assignment_generation: None,
            controller_generation: ControllerGeneration::new(1).expect("controller generation"),
            guest_execution: None,
            mode: ExecutionMode::Host,
        }
    }

    /// The driver under test: the erased boundary the actor holds, plus the
    /// typed handle for in-memory assertions.
    struct DriverUnderTest {
        erased: Box<dyn DynResourceDriver>,
        typed: ProcessDriver,
    }

    impl DriverUnderTest {
        async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
            self.erased.validate(ctx).await
        }

        async fn recover(
            &mut self,
            ctx: &mut ResourceContext,
        ) -> Result<RecoveryOutcome, DriverFailure> {
            self.erased.recover(ctx).await
        }

        async fn reconcile(
            &mut self,
            ctx: &mut ResourceContext,
        ) -> Result<ReconcileOutcome, DriverFailure> {
            self.erased.reconcile(ctx).await
        }

        async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
            self.erased.finalize(ctx).await
        }

        async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
            self.erased.delete(ctx).await
        }

        fn restart_count(&self) -> u32 {
            self.typed.restart_count()
        }

        /// Test-only: backdate the one-shot runtime clock past its deadline.
        async fn backdate_runtime(&self, elapsed: Duration) {
            self.typed.ephemeral.backdate_started(elapsed).await;
        }

        /// Test-only: backdate the one-shot retention clock.
        async fn backdate_completion(&self, elapsed: Duration) {
            self.typed.ephemeral.backdate_completed(elapsed).await;
        }

        async fn ephemeral_completed(&self) -> bool {
            self.typed.ephemeral.completed().await.is_some()
        }
    }

    async fn driver(effects: Arc<FakeFacets>) -> DriverUnderTest {
        let args = driver_args(effects);
        let typed = ProcessDriver::new(args);
        let erased: Box<dyn DynResourceDriver> = Box::new(typed.clone());
        DriverUnderTest { erased, typed }
    }

    fn expect_in_progress(
        outcome: Result<ReconcileOutcome, DriverFailure>,
    ) -> d2b_resource_runtime::context::OperationId {
        match outcome {
            Ok(ReconcileOutcome::InProgress { operation }) => operation,
            other => panic!("expected InProgress, got {other:?}"),
        }
    }

    /// Let spawned effect tasks reach their send.
    async fn yield_until_effects_settled() {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    fn contains_restart_annotation(spec: &[u8]) -> bool {
        let text = String::from_utf8_lossy(spec);
        text.contains("restartCount") || text.contains("restart-generation")
    }

    // -- factory -------------------------------------------------------------

    /// A manager can resolve an owner key only for owners that are its own
    /// rows. An owned child of an unconverted owner therefore carries no
    /// owner key, and the launch ticket must still name its controller owner:
    /// the authored reference the row was ingested with is the fallback.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn identity_falls_back_to_the_authored_owner_reference() {
        let mut row = test_row();
        row.metadata =
            br#"{"annotations":{},"labels":{},"ownerRef":"Provider/network-local"}"#.to_vec();
        let mut f = fixture(row);
        assert!(f.ctx.owner_key().is_none(), "fixture resolves no owner key");
        let d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;
        let identity = d
            .typed
            .identity(
                &mut f.ctx,
                &ResourceRef::parse("Provider/system-minijail").expect("provider ref"),
                DriverOp::Reconcile,
            )
            .await
            .expect("identity");
        assert_eq!(
            identity.launch.owner_ref(),
            Some(ResourceRef::parse("Provider/network-local").expect("owner ref")).as_ref()
        );
    }

    /// A guest-owned VMM Process row: the shape the private guest VMM intent -
    /// and its catalog-bound descriptor digest - exists for.
    fn guest_vmm_row() -> StoredDesiredResource {
        let mut row = test_row();
        row.key = ResourceKey::new("work", "Process", "acceptance-guest-vmm");
        row.metadata =
            br#"{"annotations":{},"labels":{},"ownerRef":"Guest/acceptance-guest"}"#.to_vec();
        row.spec = br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"cloud-hypervisor-runner"}"#.to_vec();
        row
    }

    async fn guest_vmm_identity(f: &mut Fixture) -> super::ProcessResourceIdentity {
        let d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;
        d.typed
            .identity(
                &mut f.ctx,
                &ResourceRef::parse("Provider/system-minijail").expect("provider ref"),
                DriverOp::Reconcile,
            )
            .await
            .expect("identity")
    }

    /// The old `scoped_target_ref` bound a Guest-owned guest-runtime process
    /// to its owning Guest: the launch ticket names the Guest as its target.
    /// The launch identity fence derives the launch VM name from that target
    /// ref, and without it the guest VMM intent is refused
    /// (`identity-rejection: resolved intent vm or legacy role mismatch`), so
    /// a controller-committed `Process/<guest>-vmm` could never launch.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn guest_runtime_row_targets_its_owning_guest() {
        let mut f = fixture(guest_vmm_row());
        let identity = guest_vmm_identity(&mut f).await;
        assert_eq!(
            identity.launch.target_ref(),
            Some(ResourceRef::parse("Guest/acceptance-guest").expect("guest target")).as_ref(),
            "the guest VMM launch ticket must target the owning Guest"
        );

        // A Guest-owned row outside the guest-runtime template families keeps
        // an unbound target: only the families the bundle's guest intents are
        // minted for take the host-exec/guest-target split.
        let mut row = guest_vmm_row();
        row.key = ResourceKey::new("work", "Process", "acceptance-guest-helper");
        row.spec = br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"reaction"}"#.to_vec();
        let mut f = fixture(row);
        let identity = guest_vmm_identity(&mut f).await;
        assert_eq!(identity.launch.target_ref(), None);
    }

    /// A VolumeBinding-owned serving worker: the owning binding declares the
    /// attachment Guest, which becomes the ticket's target ref, while the
    /// launch VM stays the Host the signed serving template binds (KTD7
    /// host-exec/guest-target split). The driver resolves this through the
    /// one launch-identity resolver instead of a call-site rule.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn binding_owned_worker_identity_targets_the_attachment_guest() {
        let binding_key = ResourceKey::new(
            "work",
            "VolumeBinding",
            "vol-binding-000000000000000000000000",
        );
        let binding_row = StoredDesiredResource {
            key: binding_key.clone(),
            uid: [0x42; 16],
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: serde_json::json!({
                "providerRef": "Provider/volume-virtiofs",
                "volumeRef": "Volume/data",
                "executionRef": "Guest/acceptance-guest",
                "view": "root",
                "access": "read-only",
                "presentation": { "presentation": "filesystem", "destination": "/mnt/data" },
                "slot": "data",
                "source": {
                    "admittedRights": ["consume"],
                    "arbitration": "shared",
                    "realizedFacets": ["filesystem-presentation"]
                },
            })
            .to_string()
            .into_bytes(),
            metadata: Vec::new(),
            created_at: 0,
        };
        let mut row = test_row();
        row.key = ResourceKey::new("work", "Process", "vol-vfd-deadbeef");
        row.owner_uid = Some([0x42; 16]);
        row.spec = br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"virtiofsd-worker","drainTimeout":"250ms"}"#.to_vec();
        let manager = OwnershipManager::with_owned(row.clone()).with_row(binding_row);
        let mut f = fixture_owned_by(row, manager, Some(binding_key));
        let d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;
        let identity = d
            .typed
            .identity(
                &mut f.ctx,
                &ResourceRef::parse("Provider/system-minijail").expect("provider ref"),
                DriverOp::Reconcile,
            )
            .await
            .expect("identity");

        assert_eq!(
            identity.launch.owner_ref(),
            Some(
                &ResourceRef::parse("VolumeBinding/vol-binding-000000000000000000000000")
                    .expect("owner ref")
            )
        );
        assert_eq!(
            identity.launch.target_ref(),
            Some(&ResourceRef::parse("Guest/acceptance-guest").expect("guest target"))
        );
        assert!(identity.launch.is_binding_worker());
        assert_eq!(identity.launch.vm(), Some("acceptance-guest"));
        assert_eq!(identity.launch.launch_vm(), "host-system");
    }

    /// A Device-owned worker row names no VM of its own (`executionRef
    /// Host/host-system`, no Guest target), so the Process controller derives
    /// the worker's VM scope from the owning Device's declared Guest owner -
    /// the same `Device.metadata.ownerRef == Guest/<vm>` derivation the TPM
    /// admission fence requires and the TPM shared-provider effects mint
    /// their `VmId` from. A Device with no Guest owner is the genuinely
    /// unresolvable case and refuses by name; absence and an unanswerable
    /// plane keep their own names instead of collapsing into it.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn device_worker_vm_resolves_from_the_owning_devices_guest_owner() {
        use super::device_worker_vm;

        let device_key = ResourceKey::new("work", "Device", "tpm0");
        let device_row = |owner: Option<&str>| StoredDesiredResource {
            key: device_key.clone(),
            uid: [0x43; 16],
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: br#"{"providerRef":"Provider/device-tpm"}"#.to_vec(),
            metadata: owner
                .map(|owner| {
                    serde_json::json!({ "ownerRef": owner })
                        .to_string()
                        .into_bytes()
                })
                .unwrap_or_default(),
            created_at: 0,
        };

        let manager =
            OwnershipManager::empty().with_row(device_row(Some("Guest/acceptance-guest")));
        let mut f = fixture_with(test_row(), manager);
        assert_eq!(
            device_worker_vm(&mut f.ctx, &device_key).await,
            Ok("acceptance-guest".to_owned()),
            "the owning Guest is the worker's VM scope"
        );

        let manager = OwnershipManager::empty().with_row(device_row(Some("Provider/device-tpm")));
        let mut f = fixture_with(test_row(), manager);
        assert_eq!(
            device_worker_vm(&mut f.ctx, &device_key).await,
            Err("device-worker-vm-unresolved"),
            "a non-Guest owner names no VM"
        );

        let manager = OwnershipManager::empty().with_row(device_row(None));
        let mut f = fixture_with(test_row(), manager);
        assert_eq!(
            device_worker_vm(&mut f.ctx, &device_key).await,
            Err("device-worker-vm-unresolved"),
            "a Device with no Guest owner is the named unresolvable case"
        );

        let mut f = fixture_with(test_row(), OwnershipManager::empty());
        assert_eq!(
            device_worker_vm(&mut f.ctx, &device_key).await,
            Err("device-worker-device-row-missing")
        );
        let mut f = fixture(test_row());
        assert_eq!(
            device_worker_vm(&mut f.ctx, &device_key).await,
            Err("device-worker-device-row-unreadable"),
            "an unanswerable plane is never read as absence"
        );
    }

    /// The NVIDIA decode posture is the same worker family and argv shape as
    /// the plain video sidecar: the posture difference is the declared
    /// template the broker's posture table binds, so the daemon must accept
    /// both names for the video family.
    #[test]
    fn device_worker_family_accepts_both_video_postures() {
        for template in ["video-worker", "video-worker-nvidia"] {
            assert_eq!(
                super::device_worker_family(template),
                Some(super::DeviceWorkerFamily::Video),
                "{template} is a video sidecar posture"
            );
        }
        assert_eq!(
            super::device_worker_family("gpu-worker"),
            Some(super::DeviceWorkerFamily::Gpu)
        );
        assert_eq!(
            super::device_worker_family("swtpm-socket"),
            Some(super::DeviceWorkerFamily::Swtpm)
        );
        assert_eq!(super::device_worker_family("reaction"), None);
    }

    /// A binding-owned worker whose owning binding row cannot be read is an
    /// incomplete identity: it fails once, at construction, instead of
    /// reaching the fence without the attachment target the serving intent
    /// resolves under.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn binding_owned_worker_without_its_binding_row_fails_construction() {
        let binding_key = ResourceKey::new(
            "work",
            "VolumeBinding",
            "vol-binding-000000000000000000000000",
        );
        let mut row = test_row();
        row.key = ResourceKey::new("work", "Process", "vol-vfd-deadbeef");
        row.owner_uid = Some([0x42; 16]);
        row.spec = br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"virtiofsd-worker"}"#.to_vec();
        let manager = OwnershipManager::with_owned(row.clone());
        let mut f = fixture_owned_by(row, manager, Some(binding_key));
        let d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;
        let error = d
            .typed
            .identity(
                &mut f.ctx,
                &ResourceRef::parse("Provider/system-minijail").expect("provider ref"),
                DriverOp::Reconcile,
            )
            .await
            .expect_err("an incomplete launch identity is refused at construction");
        assert_eq!(error.to_string(), "process-identity-incomplete");
    }

    /// The committed Provider uid the test source publishes.
    const COMMITTED_PROVIDER_UID: &str = "123e4567-e89b-42d3-a456-426614174010";

    /// A controller-class Process row owned by a Provider.
    fn controller_row() -> StoredDesiredResource {
        let mut row = test_row();
        row.key = ResourceKey::new("work", "Process", "controller");
        row.metadata =
            br#"{"annotations":{},"labels":{},"ownerRef":"Provider/network-local"}"#.to_vec();
        row.spec = br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"controller","template":"reaction","drainTimeout":"250ms"}"#.to_vec();
        row
    }

    async fn controller_identity(f: &mut Fixture) -> super::ProcessResourceIdentity {
        let d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;
        d.typed
            .identity(
                &mut f.ctx,
                &ResourceRef::parse("Provider/system-minijail").expect("provider ref"),
                DriverOp::Reconcile,
            )
            .await
            .expect("identity")
    }

    /// The Guest-owner resolution reads the pre-v3 plane only for a Guest
    /// owner whose row carries no linked uid; a linked uid always wins (the
    /// old composer's precedence: `record.resource.owner_uid` first, the
    /// owner identity cache second), and a non-Guest owner never reaches the
    /// Guest plane. Without a wired source the slot stays unbound, so the
    /// launch still refuses closed.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn guest_owner_uid_resolution_keeps_the_old_composer_precedence() {
        struct FixedGuestOwners {
            uid: ResourceUid,
            consulted: std::sync::atomic::AtomicUsize,
        }

        #[async_trait::async_trait]
        impl super::GuestOwnerIdentitySource for FixedGuestOwners {
            async fn guest_owner_uid(
                &self,
                zone: &ZoneId,
                guest_ref: &ResourceRef,
            ) -> Option<ResourceUid> {
                self.consulted
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                assert_eq!(zone.as_str(), "work");
                assert_eq!(guest_ref.name().as_str(), "acceptance-guest");
                Some(self.uid.clone())
            }
        }

        let guest_uid =
            ResourceUid::parse("323e4567-e89b-42d3-a456-426614174001").expect("guest uid");
        let source = FixedGuestOwners {
            uid: guest_uid.clone(),
            consulted: std::sync::atomic::AtomicUsize::new(0),
        };

        let mut f = fixture(guest_vmm_row());
        let identity = guest_vmm_identity(&mut f).await;
        assert_eq!(
            super::resolve_guest_owner_uid(Some(&source), &identity).await,
            Some(guest_uid.clone())
        );
        assert_eq!(
            source.consulted.load(std::sync::atomic::Ordering::Relaxed),
            1
        );

        let mut linked = guest_vmm_identity(&mut f).await;
        linked.launch = linked
            .launch
            .clone()
            .with_owner_uid(guest_uid)
            .expect("linked owner uid");
        assert_eq!(
            super::resolve_guest_owner_uid(Some(&source), &linked).await,
            None
        );
        assert_eq!(
            source.consulted.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "a linked owner uid never consults the Guest plane"
        );

        let mut f = fixture(controller_row());
        let controller = controller_identity(&mut f).await;
        assert_eq!(
            super::resolve_guest_owner_uid(Some(&source), &controller).await,
            None
        );
        assert_eq!(
            source.consulted.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "a non-Guest owner never consults the Guest plane"
        );
        assert_eq!(super::resolve_guest_owner_uid(None, &identity).await, None);
    }

    /// A supervisor that refuses to resolve the ticket - the identity fence
    /// finding no trusted intent for the role, template, target, scope, or
    /// descriptor posture - reports `resolution-failed`. Nothing was launched
    /// and nothing was observed, so the probe classifies terminally as an
    /// unavailable template; it must never be read as an ambiguous identity
    /// and quarantined (R15).
    #[test]
    fn resolution_refusals_are_never_identity_ambiguity() {
        let refused =
            super::map_provider_error("resolution-failed".to_owned(), DriverOp::Reconcile);
        assert_eq!(refused.to_string(), "process-resolution-refused");
        assert_eq!(refused.kind, ProcessDriverErrorKind::ResolutionRefused);
        let missing =
            super::map_provider_error("template-not-found".to_owned(), DriverOp::Reconcile);
        assert_eq!(missing.to_string(), "process-template-unavailable");
        let outside =
            super::map_provider_error("guest-process-not-vmm".to_owned(), DriverOp::Recover);
        assert_eq!(outside.to_string(), "process-guest-process-not-vmm");
        let observed =
            super::map_provider_error("adoption-ambiguous".to_owned(), DriverOp::Reconcile);
        assert_eq!(observed.to_string(), "process-identity-ambiguous");
        // Every mapping is terminal and names the provider's own code.
        for (error, expected_kind) in [
            (refused, ProcessDriverErrorKind::ResolutionRefused),
            (missing, ProcessDriverErrorKind::TemplateUnavailable),
            (outside, ProcessDriverErrorKind::GuestProcessNotVmm),
            (observed, ProcessDriverErrorKind::IdentityAmbiguous),
        ] {
            assert_eq!(error.kind, expected_kind);
            assert_eq!(error.kind.class(), FailureClass::Terminal);
            assert!(
                !error.detail.is_empty(),
                "a provider mapping carries the provider code"
            );
        }
    }

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn factory_registers_both_process_family_resource_types() {
        let args = driver_args(Arc::new(FakeFacets::new(FakeFacetsConfig::default())));
        let factory = ProcessDriverFactory::new(args);
        assert_eq!(factory.resource_types().len(), 2);
        assert_eq!(factory.resource_types()[0].as_str(), "Process");
        assert_eq!(factory.resource_types()[1].as_str(), "EphemeralProcess");
        let process_key = ResourceKey::new("work", "Process", "worker");
        let _erased = factory.create(&process_key).await;
        let ephemeral_key = ResourceKey::new("work", "EphemeralProcess", "runner");
        let _erased = factory.create(&ephemeral_key).await;
    }

    /// One descriptor per member type: both register through the runtime
    /// registry the plane assembles even though the two descriptors share one
    /// factory, which claims both types.
    #[test]
    fn family_descriptors_register_both_member_types() {
        let args = driver_args(Arc::new(FakeFacets::new(FakeFacetsConfig::default())));
        let descriptors = process_family_descriptors(args);
        let mut registry = d2b_resource_runtime::provider::ProviderDirectory::new();
        for descriptor in &descriptors {
            registry.register_driver(descriptor).expect("register");
            assert!(descriptor.allowed_sources.contains(AllowedSources::BUILTIN));
            assert!(descriptor.allowed_sources.contains(AllowedSources::STARTUP));
            assert!(!descriptor.allowed_sources.contains(AllowedSources::RUNTIME));
            assert!(!descriptor.exportable);
        }
        assert_eq!(
            registry.registered_types(),
            vec![
                ResourceTypeName::new(EPHEMERAL_PROCESS_TYPE_NAME),
                ResourceTypeName::new(PROCESS_TYPE_NAME),
            ]
        );
        assert_eq!(registry.decoders().len(), 2);
    }

    // -- one-shot EphemeralProcess arm (KTD13) -------------------------------

    /// One-shot launch: the typed activation-input spec decodes on the same
    /// factory, the launch runs through the ephemeral provider effect with the
    /// spec's `startDeadline` as its budget, and the retained identity is
    /// adopted on the next pass - exactly the old
    /// `launch_ephemeral_resource`/`adopt_ephemeral_resource` pairing.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn ephemeral_launch_uses_the_one_shot_effect_and_start_deadline() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(ephemeral_row());
        let mut driver = driver(fake.clone()).await;

        driver.validate(&mut f.ctx).await.expect("validate");
        assert_eq!(
            driver.recover(&mut f.ctx).await.expect("recover"),
            RecoveryOutcome::Missing
        );

        let operation = expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        let completed = f.effects.recv().await.expect("typed completion");
        assert_eq!(completed.operation, operation);
        assert!(matches!(
            completed.result,
            d2b_resource_runtime::context::EffectResult::Completed
        ));

        let launch = fake.launch_calls().pop().expect("launch recorded");
        assert_eq!(launch.kind, "EphemeralProcess");
        assert_eq!(
            launch.resource_ref,
            "EphemeralProcess/activation-nixos--runner--gen-1"
        );
        assert_eq!(launch.resource_uid, "43434343-4343-4343-8343-434343434343");
        assert_eq!(launch.provider_ref, "Provider/system-minijail");
        assert_eq!(launch.template, "activation-nixos-runner");
        assert_eq!(launch.execution_ref, "Host/host-system");
        assert_eq!(
            launch.start_deadline_ms,
            Some(7_000),
            "the bounded startDeadline is the launch budget"
        );

        // Post-launch the Provider retains the exact identity: the row goes
        // Ready through the liveness probe and the driver arms its own
        // observation requeue (the preserved 5s resync) so the exit and the
        // bounded runtime are seen without an external wake.
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Ready { adopted: true }
        );
        assert_eq!(f.requeue_calls(), [Duration::from_secs(5)]);
        assert_eq!(fake.launch_calls().len(), 1, "no second launch");
        assert!(
            fake.call_order().contains(&"probe-ephemeral"),
            "the live identity is observed through the liveness probe"
        );
    }

    /// The one-shot exit: the process this actor launched is gone, so the row
    /// reports `Succeeded` and waits out `successfulTtl` in runtime memory
    /// (R11); an elapsed TTL asks the manager to retire the row.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn ephemeral_exit_is_terminal_succeeded_and_the_ttl_retires_the_row() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            ..FakeFacetsConfig::default()
        }));
        let manager = OwnershipManager::empty();
        let mut f = fixture_with(ephemeral_row(), manager.clone());
        let mut driver = driver(fake.clone()).await;

        expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        f.effects.recv().await.expect("launch completion");

        // The process exited: never a relaunch, and the successful TTL starts.
        fake.push_liveness(ProviderLiveness::Exited);
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Succeeded {
                code: "process-exited"
            }
        );
        assert_eq!(
            f.ctx.take_status_projection(),
            Some(serde_json::json!({
                "ephemeral": {"state": "succeeded", "code": "process-exited"},
            })),
            "the terminal outcome is the row's observable one-shot evidence"
        );
        assert_eq!(
            f.requeue_calls(),
            [Duration::from_secs(3600)],
            "successfulTtl is the retention delay"
        );
        assert_eq!(fake.launch_calls().len(), 1, "a one-shot never relaunches");
        assert!(
            manager.deleted.lock().is_empty(), // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            "the retention window has not elapsed"
        );

        // TTL elapsed: the driver asks the manager to retire its own row.
        driver
            .backdate_completion(Duration::from_secs(3600))
            .await;
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(manager.deleted.lock().clone(), vec![f.row.key.clone()]); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
    }

    /// The bounded runtime: a one-shot that outlived `runtimeDeadline` stops
    /// through the preserved fixed escalation, reports `Failed`, and retains
    /// the row for `failedTtl`.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn ephemeral_runtime_deadline_stops_and_reports_a_terminal_failure() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(ephemeral_row());
        let mut driver = driver(fake.clone()).await;

        expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        f.effects.recv().await.expect("launch completion");

        driver.backdate_runtime(Duration::from_secs(300)).await;
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        let stops = fake.stop_calls();
        assert_eq!(stops.len(), 1, "one preserved one-shot stop escalation");
        assert_eq!(stops[0].kind, "EphemeralProcess");
        assert_eq!(
            stops[0].term_timeout,
            Duration::from_secs(30),
            "fixed one-shot term"
        );
        assert_eq!(
            stops[0].kill_timeout,
            Duration::from_secs(30),
            "preserved kill budget"
        );
        assert_eq!(fake.finalize_calls(), 1, "finalize after the exact stop");
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Failed {
                code: "runtime-deadline"
            }
        );
        assert_eq!(
            f.ctx.take_status_projection(),
            Some(serde_json::json!({
                "ephemeral": {"state": "failed", "code": "runtime-deadline"},
            })),
            "a failed one-shot is observable: the row's phase is Ready whatever \
             the outcome was, so only the projection carries the failure"
        );
        assert_eq!(
            f.requeue_calls(),
            [Duration::from_secs(24 * 3600)],
            "failedTtl is the retention delay"
        );
    }

    /// `incidentHold` blocks a failed one-shot's cleanup until an explicit
    /// release: no retention requeue, and no elapsed TTL ever retires it.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn ephemeral_incident_hold_keeps_a_failed_row() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            ..FakeFacetsConfig::default()
        }));
        let manager = OwnershipManager::empty();
        let mut row = ephemeral_row();
        row.spec = ephemeral_spec_bytes("7s", "5m", "1h", "24h", true);
        let mut f = fixture_with(row, manager.clone());
        let mut driver = driver(fake.clone()).await;

        expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        f.effects.recv().await.expect("launch completion");

        driver.backdate_runtime(Duration::from_secs(300)).await;
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Failed {
                code: "runtime-deadline"
            }
        );
        assert!(
            f.requeue_calls().is_empty(),
            "no cleanup timer under incident hold"
        );
        assert!(
            driver.ephemeral_completed().await,
            "the terminal state is recorded"
        );

        driver
            .backdate_completion(Duration::from_secs(365 * 24 * 3600))
            .await;
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert!(
            manager.deleted.lock().is_empty(), // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            "an incident-held failure is never auto-retired"
        );
    }

    /// A refused one-shot launch is terminal: the type carries no restart
    /// policy, so no restart backoff is ever scheduled (old
    /// `handle_start_failure` ephemeral arm).
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn ephemeral_launch_refusal_is_terminal_and_never_restarts() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            launch: Err("provider-effect:launch-failed".to_owned()),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(ephemeral_row());
        let mut driver = driver(fake.clone()).await;

        let operation = expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        let completed = f.effects.recv().await.expect("typed completion");
        assert_eq!(completed.operation, operation);
        match completed.result {
            d2b_resource_runtime::context::EffectResult::Failed(failure) => {
                assert_eq!(failure.class(), FailureClass::Terminal);
                assert_eq!(failure.op(), DriverOp::Reconcile);
            }
            other => panic!("expected a terminal failure, got {other:?}"),
        }
        assert!(
            f.requeue_calls().is_empty(),
            "a one-shot never schedules a restart"
        );
    }

    /// Recovery adopts a live one-shot without launching it, and the next pass
    /// observes the same identity without relaunching.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn ephemeral_recover_adopts_a_live_process_without_launching() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(ephemeral_row());
        let mut driver = driver(fake.clone()).await;

        fake.push_adoption(ProviderAdoption::Adopted(adopted_report()));
        assert_eq!(
            driver.recover(&mut f.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted
        );
        assert!(fake.launch_calls().is_empty(), "adopted without launch");
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Ready { adopted: true }
        );

        // The next pass observes the live identity without relaunching.
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert!(
            fake.launch_calls().is_empty(),
            "no relaunch on a live identity"
        );
        assert_eq!(f.requeue_calls(), [Duration::from_secs(5)]);
    }

    /// Drifted/ambiguous evidence during a one-shot reconcile is terminal and
    /// never launches.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn ephemeral_quarantined_classification_never_launches() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Quarantined(quarantined_report())]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(ephemeral_row());
        let mut driver = driver(fake.clone()).await;

        let failure = driver.reconcile(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Reconcile);
        assert!(
            fake.launch_calls().is_empty(),
            "an ambiguous identity never launches"
        );
    }

    /// One-shot deletion adopts first (old `deletion_adoption`), stops the
    /// exact live identity through the fixed escalation, and finalizes the
    /// provider's local authority.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn ephemeral_delete_stops_the_exact_identity_and_finalizes() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(ephemeral_row());
        let mut driver = driver(fake.clone()).await;

        driver.delete(&mut f.ctx).await.expect("delete");
        assert_eq!(
            fake.call_order(),
            ["adopt-ephemeral", "stop-ephemeral", "finalize"]
        );
        let stops = fake.stop_calls();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].kind, "EphemeralProcess");
        assert_eq!(stops[0].term_timeout, Duration::from_secs(30));
        assert_eq!(stops[0].kill_timeout, Duration::from_secs(30));
    }

    /// Deletion converges without effects when no exact identity remains, and
    /// stops a uniquely identified stale candidate after a restart.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn ephemeral_delete_converges_absent_and_stops_a_stale_candidate() {
        let absent = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(ephemeral_row());
        let mut absent_driver = driver(absent.clone()).await;
        absent_driver.delete(&mut f.ctx).await.expect("delete");
        assert!(absent.stop_calls().is_empty());
        assert_eq!(absent.finalize_calls(), 0);

        let stale = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Stale {
                candidate: stale_candidate(),
            }]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(ephemeral_row());
        let mut stale_driver = driver(stale.clone()).await;
        stale_driver.delete(&mut f.ctx).await.expect("delete");
        assert_eq!(stale.call_order(), ["adopt-ephemeral", "stop-stale"]);
    }

    /// An ambiguous one-shot identity refuses destructive action.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn ephemeral_delete_refuses_an_ambiguous_identity() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Quarantined(quarantined_report())]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(ephemeral_row());
        let mut driver = driver(fake.clone()).await;

        let failure = driver.delete(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Delete);
        assert!(
            fake.stop_calls().is_empty(),
            "no destructive action on ambiguity"
        );
    }

    /// A row no host-minted ticket can describe - a Guest-owned one-shot
    /// outside the guest VMM chain, like the projected
    /// `store-preflight-<guest>` intent - converges on delete without
    /// provider effects: this daemon never realized an identity for it, and
    /// retrying the ticket forever blocked its owner's teardown. Reconcile
    /// classifies the same refusal terminally, never as a retryable identity
    /// fault.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn ephemeral_unmintable_ticket_converges_on_delete_and_is_terminal_on_reconcile() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adopt_error: Some("provider-ticket:guest-process-not-vmm".to_owned()),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(ephemeral_row());
        let mut driver = driver(fake.clone()).await;

        let failure = driver.reconcile(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Reconcile);
        assert!(
            fake.launch_calls().is_empty(),
            "an unmintable ticket never launches"
        );

        driver.delete(&mut f.ctx).await.expect("delete converges");
        assert!(fake.stop_calls().is_empty(), "no provider effect ran");
        assert_eq!(fake.finalize_calls(), 0);
    }

    // -- launch happy path ---------------------------------------------------

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn launch_reaches_ready_with_expected_ticket_inputs() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        driver.validate(&mut f.ctx).await.expect("validate");
        assert_eq!(
            driver.recover(&mut f.ctx).await.expect("recover"),
            RecoveryOutcome::Missing
        );

        // Absent -> spawn the signed-ticket launch as a long effect; the
        // mailbox never blocked on it (R5).
        let operation = expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;

        let launch = fake.launch_calls().pop().expect("launch recorded");
        assert_eq!(launch.resource_ref, "Process/worker");
        assert_eq!(launch.resource_uid, "42424242-4242-4242-8242-424242424242");
        assert_eq!(launch.generation, 3);
        assert_eq!(launch.zone.as_str(), "work");
        assert_eq!(
            launch.zone_uid.as_ref().map(ResourceUid::as_str),
            Some(ZONE_UID)
        );
        assert_eq!(launch.policy_revision, Some(7));
        assert_eq!(launch.provider_ref, "Provider/system-minijail");
        assert_eq!(launch.template, "reaction");
        assert_eq!(launch.execution_ref, "Host/host-system");

        let completed = f.effects.recv().await.expect("typed completion");
        assert_eq!(completed.operation, operation);
        assert!(matches!(
            completed.result,
            d2b_resource_runtime::context::EffectResult::Completed
        ));

        // Post-launch probe adopts the identity the Provider retained.
        fake.push_adoption(ProviderAdoption::Adopted(adopted_report()));
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        let status = f.ctx.status::<ProcessDriverStatus>().expect("status");
        assert_eq!(*status, ProcessDriverStatus::Ready { adopted: true });
        assert_eq!(fake.launch_calls().len(), 1, "no second launch");
    }

    /// A `never-adopt` row must not read the process it launched itself as an
    /// unexpected live identity: the reconcile arm used to stop and relaunch
    /// its own process on every effect completion (a stop/launch loop).
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn never_adopt_row_observes_the_identity_it_launched_instead_of_stopping_it() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            active: false,
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(never_adopt_row());
        let mut driver = driver(fake.clone()).await;

        driver.validate(&mut f.ctx).await.expect("validate");
        let operation = expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        let completed = f.effects.recv().await.expect("typed completion");
        assert_eq!(completed.operation, operation);
        assert!(matches!(
            completed.result,
            d2b_resource_runtime::context::EffectResult::Completed
        ));

        // The Provider retains the identity that launch recorded; the next
        // pass must observe its own process instead of replacing it.
        fake.set_active(true);
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            fake.launch_calls().len(),
            1,
            "the launched identity is not relaunched"
        );
        assert!(
            fake.stop_calls().is_empty(),
            "a live identity this row launched is not an unexpected one"
        );
        let status = f.ctx.status::<ProcessDriverStatus>().expect("status");
        assert_eq!(*status, ProcessDriverStatus::Ready { adopted: true });
    }

    // -- recover: adoption / quarantine / missing ----------------------------

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn recover_adopts_a_live_matching_process_without_launching() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        driver.validate(&mut f.ctx).await.expect("validate");
        assert_eq!(
            driver.recover(&mut f.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted
        );
        assert!(fake.launch_calls().is_empty(), "adopted without launch");
        let status = f.ctx.status::<ProcessDriverStatus>().expect("status");
        assert_eq!(*status, ProcessDriverStatus::Ready { adopted: true });
    }

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn recover_classifies_drifted_and_ambiguous_processes_as_quarantined() {
        for adoption in [
            ProviderAdoption::Stale {
                candidate: stale_candidate(),
            },
            ProviderAdoption::Quarantined(quarantined_report()),
        ] {
            let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
                adoption: VecDeque::from([adoption]),
                ..FakeFacetsConfig::default()
            }));
            let mut f = fixture(test_row());
            let mut driver = driver(fake.clone()).await;

            assert_eq!(
                driver.recover(&mut f.ctx).await.expect("recover"),
                RecoveryOutcome::Quarantined
            );
            assert!(fake.launch_calls().is_empty(), "quarantined: no adopt");
        }
    }

    // -- finalize: owned children retire before the process teardown (F3) ----

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn finalize_finalizes_owned_children_before_the_process_teardown() {
        let manager = OwnershipManager::with_owned(StoredDesiredResource {
            owner_uid: Some([0x42; 16]),
            ..test_row()
        });
        let mut f = fixture_with(test_row(), manager.clone());
        let mut d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;

        // A live owned child: the erased children-first boundary refuses with
        // the shared `children-draining` NotYet before the driver body runs.
        let failure = d
            .finalize(&mut f.ctx)
            .await
            .expect_err("owned child still live");
        assert_eq!(
            failure,
            DriverFailure::not_yet(DriverOp::Delete, FailureKinds::CHILDREN_DRAINING)
        );
        assert_eq!(
            manager.deleted.lock().len(), // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            1,
            "the owned child is nudged first"
        );

        // The manager removed the retired child row: the same pass converges.
        d.finalize(&mut f.ctx)
            .await
            .expect("converged once the child retired");
    }

    // -- delete: term then kill ----------------------------------------------

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn delete_stops_term_then_kill_and_finalizes() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        driver.delete(&mut f.ctx).await.expect("delete");

        let stops = fake.stop_calls();
        assert_eq!(stops.len(), 1, "one preserved stop escalation");
        assert_eq!(
            stops[0].term_timeout,
            Duration::from_millis(250),
            "drain timeout from the spec"
        );
        assert_eq!(
            stops[0].kill_timeout,
            Duration::from_secs(30),
            "preserved kill budget"
        );
        assert_eq!(fake.finalize_calls(), 1, "finalize after the exact stop");
        assert_eq!(fake.call_order(), ["adopt", "stop", "finalize"]);
    }

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn delete_without_a_live_process_is_a_noop() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        driver.delete(&mut f.ctx).await.expect("delete");
        assert!(fake.stop_calls().is_empty());
        assert_eq!(fake.finalize_calls(), 0);
    }

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn delete_stops_an_exact_stale_candidate_after_restart() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Stale {
                candidate: stale_candidate(),
            }]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        driver.delete(&mut f.ctx).await.expect("delete");
        assert_eq!(fake.call_order(), ["adopt", "stop-stale"]);
    }

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn delete_refuses_an_ambiguous_identity() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Quarantined(quarantined_report())]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        let failure = driver.delete(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Delete);
    }

    /// A declared Device-owned worker row deletes through its exact live
    /// identity even when the launch-only parameter derivation refuses (an
    /// unbound Wayland socket, an unresolvable state dir). The derivation
    /// exists for the launch ticket, so a delete that depended on it
    /// converged without stopping anything - retiring the row while the
    /// process it launched kept running unowned.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn delete_stops_a_device_worker_even_when_the_launch_parameters_refuse() {
        let mut row = test_row();
        row.spec = br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"gpu-worker","drainTimeout":"250ms"}"#
            .to_vec();
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture_owned_by(
            row,
            Arc::new(DeadManager),
            Some(ResourceKey::new("work", "Device", "corp-gpu")),
        );
        let mut driver = driver(fake.clone()).await;

        driver.delete(&mut f.ctx).await.expect("delete converges");
        assert_eq!(
            fake.call_order(),
            ["adopt", "stop", "finalize"],
            "the delete never derives launch-only parameters"
        );
        assert_eq!(fake.stop_calls().len(), 1, "the live identity is stopped");
        assert_eq!(fake.finalize_calls(), 1, "the exact authority is finalized");
    }

    // -- retryable reconcile failure -> exactly one requeue with backoff -----

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn retryable_reconcile_failure_requeues_exactly_once_with_backoff() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            launch: Err("provider-effect:launch-failed".to_owned()),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        // First launch attempt fails; the spawned effect reports a retryable
        // failure (the restart policy allows restarts).
        expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        let completed = f.effects.recv().await.expect("completion");
        assert!(matches!(
            completed.result,
            d2b_resource_runtime::context::EffectResult::Failed(_)
        ));
        if let d2b_resource_runtime::context::EffectResult::Failed(failure) = completed.result {
            assert_eq!(failure.class(), FailureClass::Retryable);
            assert_eq!(failure.op(), DriverOp::Reconcile);
        }

        // The next pass schedules exactly one runtime-only requeue with the
        // policy restart delay (in-memory restart budget, spec section 32).
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::RetryScheduled
        );
        let requeue = f.requeue_calls();
        assert_eq!(requeue.len(), 1, "exactly one requeue schedule");
        assert_eq!(requeue[0], Duration::from_secs(1), "restart backoff base");

        // The requeue timer fires after the backoff and the next reconcile
        // relaunches.
        tokio::time::advance(Duration::from_secs(1)).await;
        yield_until_effects_settled().await;
        f.requeues.recv().await.expect("requeue delivery");

        fake.set_launch(Ok(ProcessIdentityDigest::from_bytes([0x51; 32])));
        fake.push_adoption(ProviderAdoption::Absent);
        expect_in_progress(driver.reconcile(&mut f.ctx).await);
        let completed = f.effects.recv().await.expect("completion");
        assert!(matches!(
            completed.result,
            d2b_resource_runtime::context::EffectResult::Completed
        ));
    }

    // -- restart budget is runtime-only --------------------------------------

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn restart_budget_is_in_memory_only() {
        let policy = r#"{"backoffBase":"1s","backoffMax":"60s","backoffMultiplierMilli":2000,"maxRestarts":1,"resetAfter":"60s"}"#;
        let mut row = test_row();
        row.spec = spec_bytes(Some(policy));
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            launch: Err("provider-effect:launch-failed".to_owned()),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(row);
        let mut driver = driver(fake.clone()).await;

        // Budget allows one restart.
        expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        let completed = f.effects.recv().await.expect("completion");
        assert!(matches!(
            completed.result,
            d2b_resource_runtime::context::EffectResult::Failed(failure)
                if failure.class() == FailureClass::Retryable
        ));
        assert_eq!(driver.restart_count(), 1);

        // The next pass schedules exactly one runtime-only requeue with the
        // policy restart delay (spec section 32: in-memory, never persisted).
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::RetryScheduled
        );
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::AwaitingRestart { restart_count: 1 },
            "a row awaiting its relaunch reports the awaiting classification, never Ready"
        );
        let requeue = f.requeue_calls();
        assert_eq!(requeue.len(), 1, "exactly one requeue schedule");
        assert_eq!(requeue[0], Duration::from_secs(1), "restart backoff base");

        // After the backoff elapses the relaunch runs and exhausts the
        // budget: the failure is terminal.
        tokio::time::advance(Duration::from_secs(1)).await;
        yield_until_effects_settled().await;
        f.requeues.recv().await.expect("requeue delivery");

        fake.push_adoption(ProviderAdoption::Absent);
        expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        let completed = f.effects.recv().await.expect("completion");
        assert!(matches!(
            completed.result,
            d2b_resource_runtime::context::EffectResult::Failed(failure)
                if failure.class() == FailureClass::Terminal
        ));
        assert_eq!(fake.launch_calls().len(), 2);

        // A further pass refuses to launch: terminal, budget exhausted.
        fake.push_adoption(ProviderAdoption::Absent);
        let failure = driver.reconcile(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(fake.launch_calls().len(), 2, "no launch past the budget");
        assert_eq!(f.requeue_calls().len(), 1, "no requeue past the budget");
    }

    // -- durable observation: an exit leaves Ready and the policy runs ------

    /// The durable steady state: a row this actor adopted is observed through
    /// the liveness probe at the preserved 5s cadence (before this the pass
    /// returned without requeueing, so nothing ever re-entered and the row
    /// reported `Ready` over a process that was gone), and an exit consumes
    /// one budgeted restart and requeues the relaunch at the policy backoff.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn durable_exit_is_observed_and_restarts_under_the_policy() {
        let policy = r#"{"class":"on-failure","backoffBase":"1s","backoffMax":"60s","backoffMultiplierMilli":2000,"maxRestarts":2,"resetAfter":"300s"}"#;
        let mut row = test_row();
        row.spec = spec_bytes(Some(policy));
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(row);
        let mut driver = driver(fake.clone()).await;

        // First pass: the retained identity is adopted, the row reports Ready,
        // and the observation cadence is armed.
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Ready { adopted: true }
        );
        assert_eq!(
            f.requeue_calls(),
            [super::PROCESS_RESYNC],
            "a live durable row observes itself"
        );

        // Second pass: the steady state re-reads the identity through the
        // liveness probe (never a second adoption) and re-arms the cadence.
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(fake.call_order(), ["adopt", "probe"]);
        assert_eq!(
            f.requeue_calls(),
            [super::PROCESS_RESYNC, super::PROCESS_RESYNC]
        );

        // The process exits: the probe reports it, the row leaves Ready, and
        // the restart policy consumes one restart and requeues the relaunch at
        // the policy backoff. The pass reports a scheduled retry, not a
        // satisfied pass: the row has no live process until the relaunch.
        fake.push_liveness(ProviderLiveness::Exited);
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::RetryScheduled
        );
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::AwaitingRestart { restart_count: 1 }
        );
        assert_eq!(
            f.requeue_calls(),
            [
                super::PROCESS_RESYNC,
                super::PROCESS_RESYNC,
                Duration::from_secs(1)
            ],
            "the relaunch waits out the policy restart backoff"
        );
        assert_eq!(driver.restart_count(), 1, "the exit consumed one restart");

        // Past the backoff the adoption path runs again: absent -> relaunch.
        expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        let completed = f.effects.recv().await.expect("completion");
        assert!(matches!(
            completed.result,
            d2b_resource_runtime::context::EffectResult::Completed
        ));
        assert_eq!(
            fake.launch_calls().len(),
            1,
            "the exited process was relaunched"
        );
    }

    /// The same observation edge never laundered the exit into a first sight:
    /// when the restart policy forbids a restart, the exit is the row's
    /// terminal reading, and a later trigger reports the exit again instead of
    /// launching a process the policy forbade. The pass refuses (terminal,
    /// spent restart budget) instead of reporting a satisfied row: a
    /// satisfied pass publishes wire `Ready`, and a phase gate such as
    /// `DaemonGpuLifecyclePort::declared_worker` mints the worker identity
    /// from that phase over a process that no longer exists.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn durable_exit_under_a_never_policy_is_terminal_and_never_relaunches() {
        let policy = r#"{"class":"never","backoffBase":"1s","backoffMax":"60s","backoffMultiplierMilli":2000,"resetAfter":"300s"}"#;
        let mut row = test_row();
        row.spec = spec_bytes(Some(policy));
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
            liveness: VecDeque::from([ProviderLiveness::Exited, ProviderLiveness::Exited]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(row);
        let mut driver = driver(fake.clone()).await;

        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Ready { adopted: true }
        );

        let failure = driver.reconcile(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Reconcile);
        assert_eq!(failure.kind().code(), "process-start-budget-exhausted");
        assert_eq!(failure.stage(), "observe/liveness");
        assert!(
            !failure.defers(),
            "a spent restart budget schedules no retry"
        );
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Failed {
                code: "process-exited"
            }
        );
        assert_eq!(
            d2b_resource_runtime::ResourceStatus::Failed(failure.clone()).wire_phase(),
            "Failed",
            "an exit no restart can follow is never a readiness claim"
        );
        assert_eq!(
            driver.restart_count(),
            0,
            "a refused restart consumes nothing"
        );
        assert_eq!(
            f.requeue_calls(),
            [super::PROCESS_RESYNC],
            "a terminal exit arms no observation cadence"
        );
        assert!(
            fake.launch_calls().is_empty(),
            "no relaunch past the policy"
        );

        // A later trigger re-observes the exit; it never becomes a launch.
        let failure = driver.reconcile(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.kind().code(), "process-start-budget-exhausted");
        assert!(
            fake.launch_calls().is_empty(),
            "no relaunch past the policy"
        );
    }

    /// An identity the liveness probe can no longer verify (old
    /// `observe_liveness` Unknown) is the same ambiguity the adoption
    /// classification refuses: the row reports the terminal
    /// `process-identity-ambiguous` refusal, so the actor publishes wire
    /// `Failed` and never `Ready`. Before this the arm returned `Satisfied`
    /// and the runtime mapped that to wire `Ready` over a process this daemon
    /// could not identify.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn durable_liveness_ambiguity_refuses_terminally_and_never_reads_ready() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
            liveness: VecDeque::from([ProviderLiveness::Unknown]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        // The retained identity is adopted: `Ready` only while the probe
        // verifies it.
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );

        // The probe no longer verifies the identity: the pass refuses.
        let failure = driver.reconcile(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Reconcile);
        assert_eq!(failure.kind().code(), "process-identity-ambiguous");
        assert_eq!(failure.stage(), "observe/liveness");
        assert!(!failure.defers(), "a refusal schedules no retry");

        // The driver's classification and the runtime phase it publishes
        // agree: `Failed`, never `Ready`.
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Failed {
                code: "identity-ambiguous"
            }
        );
        assert_eq!(
            d2b_resource_runtime::ResourceStatus::Failed(failure).wire_phase(),
            "Failed",
            "an ambiguous liveness observation is never a readiness claim"
        );

        // Nothing re-enters the pass and the ambiguity stays untouched: no
        // second observation cadence, no relaunch, no signal to the candidate.
        assert_eq!(fake.call_order(), ["adopt", "probe"]);
        assert_eq!(
            f.requeue_calls(),
            [super::PROCESS_RESYNC],
            "the refusal arms no observation cadence"
        );
        assert!(
            fake.launch_calls().is_empty(),
            "an ambiguous identity is never launched"
        );
        assert!(
            fake.stop_calls().is_empty(),
            "no signal reaches the ambiguous candidate"
        );
    }

    // -- stopped desired lifecycle: realized only when observed stopped ------

    /// A row whose desired lifecycle is `stopped` establishes the stop through
    /// the same exact escalation every other path uses and reads its terminal
    /// `Succeeded{process-stopped}` only off the observed stop. Before this the
    /// pass asserted the state: it published the terminal status and returned
    /// `Satisfied` - wire `Ready`, every `WatchCondition::Ready` watcher fired -
    /// with nothing probed and nothing stopped.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn stopped_lifecycle_row_stops_the_live_process_before_reading_succeeded() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            liveness: VecDeque::from([ProviderLiveness::Exited]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(stopped_row());
        let mut driver = driver(fake.clone()).await;

        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            fake.call_order(),
            ["stop", "finalize", "probe"],
            "the live identity is stopped and the stop is probed, never asserted"
        );
        assert_eq!(fake.stop_calls().len(), 1, "the exact escalation ran");
        assert_eq!(
            fake.stop_calls()[0].term_timeout,
            Duration::from_millis(250),
            "the spec's drainTimeout bounds the term stage"
        );
        assert_eq!(
            fake.finalize_calls(),
            1,
            "the provider authority is released"
        );
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Succeeded {
                code: "process-stopped"
            }
        );
        assert!(
            f.requeue_calls().is_empty(),
            "a realized stop needs no re-check"
        );
    }

    /// The same row while the process is still running: the stop is not
    /// established, so the pass schedules its own re-check and reports the
    /// retry instead of a readiness claim. Only a later observed stop lets the
    /// row read `Succeeded`.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn stopped_lifecycle_row_still_live_defers_instead_of_claiming_satisfied() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            liveness: VecDeque::from([ProviderLiveness::Alive]),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(stopped_row());
        let mut driver = driver(fake.clone()).await;

        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::RetryScheduled
        );
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Stopping,
            "a stop this pass could not establish never reads as a realized row"
        );
        assert_eq!(
            f.requeue_calls(),
            [super::PROCESS_RESYNC],
            "the pass schedules its own re-check at the preserved resync"
        );
        assert_eq!(
            fake.call_order(),
            ["stop", "finalize", "probe"],
            "the live process is stopped, never left running behind a satisfied pass"
        );

        // The stop lands: the next pass probes it gone and only then reads the
        // terminal status.
        fake.set_active(false);
        fake.push_liveness(ProviderLiveness::Exited);
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            *f.ctx.status::<ProcessDriverStatus>().expect("status"),
            ProcessDriverStatus::Succeeded {
                code: "process-stopped"
            }
        );
        assert_eq!(fake.stop_calls().len(), 1, "nothing is signalled twice");
    }

    // -- a durable launch no retry can resolve fails terminally --------------

    /// A durable launch ticket the trusted bundle can never mint refuses
    /// terminally: the in-memory budget cannot make any of these launchable, so
    /// the row fails instead of relaunching (and warning) forever under the
    /// default policy with no bounded `maxRestarts`. Before this the driver
    /// classified from the budget alone, consumed a restart, and requeued.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn durable_unresolvable_launch_ticket_refuses_terminally() {
        for (error, kind) in [
            (
                "provider-ticket:template-not-found",
                FailureKinds::PROCESS_TEMPLATE_UNAVAILABLE,
            ),
            (
                "resolution-failed",
                FailureKinds::PROCESS_RESOLUTION_REFUSED,
            ),
            (
                "provider-ticket:guest-process-not-vmm",
                FailureKinds::PROCESS_GUEST_PROCESS_NOT_VMM,
            ),
        ] {
            let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
                adoption: VecDeque::from([ProviderAdoption::Absent]),
                launch: Err(error.to_owned()),
                ..FakeFacetsConfig::default()
            }));
            let mut f = fixture(test_row());
            let mut driver = driver(fake.clone()).await;

            expect_in_progress(driver.reconcile(&mut f.ctx).await);
            yield_until_effects_settled().await;
            let completed = f.effects.recv().await.expect("completion");
            let d2b_resource_runtime::context::EffectResult::Failed(failure) = completed.result
            else {
                panic!("{error} must refuse the launch");
            };
            assert_eq!(
                failure.class(),
                FailureClass::Terminal,
                "{error} never becomes launchable"
            );
            assert_eq!(failure.op(), DriverOp::Reconcile);
            assert_eq!(failure.report().code(), kind.code());
            assert_eq!(driver.restart_count(), 0, "{error} consumes no restart");
            assert!(
                f.requeue_calls().is_empty(),
                "{error} arms no restart backoff"
            );
        }
    }

// -- validate ------------------------------------------------------------

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn validate_rejects_an_unsupported_provider() {
        let mut row = test_row();
        row.spec = br#"{"providerRef":"Provider/other","executionRef":"Host/host-system","processClass":"worker","template":"reaction"}"#
            .to_vec();
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig::default()));
        let mut f = fixture(row);
        let mut driver = driver(fake).await;

        let failure = driver.validate(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Validate);
    }

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn validate_rejects_a_malformed_spec() {
        let mut row = test_row();
        row.spec = b"{not-json".to_vec();
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig::default()));
        let mut f = fixture(row);
        let mut driver = driver(fake).await;

        let failure = driver.validate(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Validate);
    }

// -- execution-target gate -------------------------------------------------

/// A Guest `executionRef` under a Host-mode driver is refused
/// `process-execution-unsupported`: the Host driver drives Host-executing
/// rows only, and the whole Guest-mode gate is otherwise unpinned.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
    async fn validate_rejects_a_guest_execution_under_a_host_driver() {
        let mut row = test_row();
        row.spec = br#"{"providerRef":"Provider/system-minijail","executionRef":"Guest/vm-a","processClass":"worker","template":"reaction","drainTimeout":"250ms"}"#
            .to_vec();
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig::default()));
        let mut f = fixture(row);
        let mut driver = driver(fake).await;

        let failure = driver.validate(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Validate);
        assert!(
            failure.to_string().contains("process-execution-unsupported"),
            "failure names the execution gate: {failure}"
        );
    }

// -- restart backoff arithmetic -------------------------------------------

/// The preserved restart backoff: `base * multiplier^(count-1)`, capped at
/// `backoff_max`. Every restart test observes count == 1 (delay == base),
/// so the 2nd+ restart arithmetic and the cap are pinned here.
#[test]
    fn restart_delay_backs_off_exponentially_and_caps_at_the_maximum() {
        let spec: ProcessSpec = serde_json::from_slice(
            br#"{"executionRef":"Host/host-system","processClass":"worker","template":"reaction","restartPolicy":{"backoffBase":"1s","backoffMax":"60s","backoffMultiplierMilli":2000,"maxRestarts":2,"resetAfter":"300s"}}"#,
        )
        .expect("spec decodes");
        assert_eq!(restart_delay(&spec, 1), Duration::from_secs(1), "first restart waits the base");
        assert_eq!(restart_delay(&spec, 2), Duration::from_secs(2), "2x multiplier");
        assert_eq!(restart_delay(&spec, 3), Duration::from_secs(4), "4x multiplier");
        assert_eq!(
            restart_delay(&spec, 7),
            Duration::from_secs(60),
            "the exponential backoff is capped at backoff_max"
        );
    }

    // -- Guest target transport (R18, R19, R21, R29) ------------------------

    /// The Guest this suite's rows target.
    fn guest_target() -> d2b_resource_runtime::target::TargetRef {
        d2b_resource_runtime::target::TargetRef::guest("test-vm").expect("guest target ref")
    }

    /// One `Process` row committed to that Guest instead of the Host.
    fn guest_row() -> StoredDesiredResource {
        let mut row = test_row();
        row.spec = br#"{"providerRef":"Provider/system-minijail","executionRef":"Guest/test-vm","processClass":"worker","template":"reaction","drainTimeout":"250ms"}"#
            .to_vec();
        row
    }

    /// One frame the driver asked the Guest to realize: the source row, the
    /// exact resolved spec bytes, and the digest and handle it committed to.
    type RealizedFrame = (ResourceKey, Vec<u8>, String, String);

    /// One recorded target-control session: exactly what the driver asked the
    /// Guest to apply, and the answers it scripts back.
    #[derive(Debug)]
    struct FakeGuestTarget {
        realized: parking_lot::Mutex<Vec<RealizedFrame>>,
        observed: parking_lot::Mutex<VecDeque<TargetObservation>>,
        adopted: parking_lot::Mutex<VecDeque<GuestAdoption>>,
        deleted: parking_lot::Mutex<Vec<ResourceKey>>,
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl FakeGuestTarget {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                realized: parking_lot::Mutex::new(Vec::new()),
                observed: parking_lot::Mutex::new(VecDeque::new()),
                adopted: parking_lot::Mutex::new(VecDeque::new()),
                deleted: parking_lot::Mutex::new(Vec::new()),
            })
        }

        fn script_adoption(&self, adoption: GuestAdoption) {
            self.adopted.lock().push_back(adoption); // async-gate-allow: fixture scripts an answer under a short guard and holds no await
        }

        fn script_observation(&self, observation: TargetObservation) {
            self.observed.lock().push_back(observation); // async-gate-allow: fixture scripts an answer under a short guard and holds no await
        }

        fn realized(&self) -> Vec<RealizedFrame> {
            self.realized.lock().clone()
        }

        fn deleted(&self) -> Vec<ResourceKey> {
            self.deleted.lock().clone()
        }

        /// One live target-local realization of this suite's row.
        fn live(session_generation: u64) -> GuestAdoption {
            GuestAdoption::Adopted(TargetResourceInstance::new(
                ResourceKey::new("work", PROCESS_TYPE_NAME, "worker"),
                [0x42; 16],
                3,
                session_generation,
                "d2b/process/Process/worker".to_owned(),
                "sha256:probe".to_owned(),
                TargetInstanceState::Ready,
            ))
        }
    }

    #[async_trait::async_trait]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl d2b_resource_runtime::guest_target::GuestTargetControl for FakeGuestTarget {
        async fn realize(
            &self,
            request: d2b_resource_runtime::guest_target::GuestRealizeRequest,
        ) -> Result<TargetResourceInstance, d2b_resource_runtime::guest_target::GuestTargetError>
        {
            let digest = request.spec_digest().to_owned();
            let handle = request.local_handle().to_owned();
            self.realized.lock().push(( // async-gate-allow: fixture records the frame under a short guard and holds no await
                request.source().clone(),
                request.spec().to_vec(),
                digest.clone(),
                handle.clone(),
            ));
            Ok(TargetResourceInstance::new(
                request.source().clone(),
                *request.source_uid(),
                request.assignment_generation(),
                request.session_generation(),
                handle,
                digest,
                TargetInstanceState::Ready,
            ))
        }

        async fn observe(
            &self,
            assignment: &d2b_resource_runtime::guest_target::TargetControlAssignment,
        ) -> Result<TargetObservation, d2b_resource_runtime::guest_target::GuestTargetError>
        {
            Ok(self
                .observed
                .lock() // async-gate-allow: fixture pops under a short guard and holds no await
                .pop_front()
                .unwrap_or(TargetObservation::Ready {
                    session_generation: assignment.session_generation(),
                }))
        }

        async fn delete(
            &self,
            assignment: &d2b_resource_runtime::guest_target::TargetControlAssignment,
        ) -> Result<(), d2b_resource_runtime::guest_target::GuestTargetError> {
            self.deleted.lock().push(assignment.source().clone()); // async-gate-allow: fixture records the delete under a short guard and holds no await
            Ok(())
        }

        async fn adopt(
            &self,
            _assignment: &d2b_resource_runtime::guest_target::TargetControlAssignment,
        ) -> Result<GuestAdoption, d2b_resource_runtime::guest_target::GuestTargetError> {
            Ok(self.adopted.lock().pop_front().unwrap_or(GuestAdoption::Missing)) // async-gate-allow: fixture pops under a short guard and holds no await
        }
    }

    /// A driver fixture whose row is committed to a live Guest target: the
    /// same context the manager builds, with the directory-backed binding the
    /// manager committed attached (R19, R29).
    struct GuestFixture {
        fixture: Fixture,
        directory: Arc<TargetDirectory>,
        target: Arc<FakeGuestTarget>,
    }

    impl GuestFixture {
        fn new(row: StoredDesiredResource) -> Self {
            Self::with(row, Arc::new(DeadManager))
        }

        fn with(row: StoredDesiredResource, manager: Arc<dyn ManagerEndpoint>) -> Self {
            let directory = Arc::new(TargetDirectory::new());
            let assignment = directory
                .assign(&row.key, &row.uid, row.generation, "Guest/test-vm")
                .expect("guest assignment");
            let target = FakeGuestTarget::new();
            directory
                .connect_guest(&guest_target(), 1, Arc::clone(&target) as Arc<_>)
                .expect("live guest session");
            let binding = TargetBinding::new(directory.as_ref().clone(), assignment);
            let fixture = fixture_targeted(row, manager, binding);
            Self { fixture, directory, target }
        }

        /// Drop the live session, exactly as the daemon's unbind path does.
        fn disconnect(&self) {
            self.directory
                .disconnect_guest(&guest_target(), 1)
                .expect("guest disconnect");
        }

        /// Bring a newer session up over the same directory (F5).
        fn reconnect(&self, generation: u64) {
            self.directory
                .connect_guest(&guest_target(), generation, Arc::clone(&self.target) as Arc<_>)
                .expect("guest reconnect");
        }
    }

    /// A live authenticated Guest target is where this row runs: the first
    /// pass discovers nothing, realizes the exact host-resolved realization,
    /// and only then reports readiness (R18, R29).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_guest_targeted_process_launches_through_the_session_and_reports_ready() {
        let mut f = GuestFixture::new(guest_row());
        f.target.script_adoption(GuestAdoption::Missing);
        f.target.script_adoption(FakeGuestTarget::live(1));
        let mut d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;

        assert!(matches!(
            d.reconcile(&mut f.fixture.ctx).await.expect("first pass"),
            ReconcileOutcome::InProgress { .. }
        ));
        yield_until_effects_settled().await;
        let realized = f.target.realized();
        assert_eq!(realized.len(), 1, "exactly one realize frame reached the Guest");
        assert_eq!(
            realized[0].0,
            ResourceKey::new("work", PROCESS_TYPE_NAME, "worker"),
            "the frame names this row's committed source, never a guest-local identity"
        );
        let realization = crate::worker_launch::GuestProcessRealization::decode(&realized[0].1)
            .expect("the host-resolved realization decodes on the target");
        assert_eq!(
            realization.process_ref(),
            "Process/worker",
            "the target-local realization names the Host-zone row"
        );
        assert!(!realization.spec().is_empty(), "the resolved spec travels verbatim");

        assert_eq!(
            d.reconcile(&mut f.fixture.ctx).await.expect("second pass"),
            ReconcileOutcome::Satisfied,
            "the exact live realization is adopted, never realized a second time"
        );
        assert_eq!(f.target.realized().len(), 1);

        f.target.script_observation(TargetObservation::Ready { session_generation: 1 });
        assert_eq!(
            d.reconcile(&mut f.fixture.ctx).await.expect("third pass"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            f.fixture.ctx.status::<ProcessDriverStatus>().copied(),
            Some(ProcessDriverStatus::Ready { adopted: true }),
            "the row reports actor readiness, in memory only"
        );
    }

    /// Deletion removes exactly this row's target-local realization and is
    /// idempotent under retry (F3, R20, R29).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn guest_delete_removes_only_the_exact_source_process_and_repeats_cleanly() {
        let mut f = GuestFixture::new(guest_row());
        f.target.script_adoption(FakeGuestTarget::live(1));
        f.target.script_adoption(FakeGuestTarget::live(1));
        let mut d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;

        d.recover(&mut f.fixture.ctx).await.expect("recovery adopts the live realization");
        d.delete(&mut f.fixture.ctx).await.expect("delete");
        assert_eq!(
            f.target.deleted(),
            vec![ResourceKey::new("work", PROCESS_TYPE_NAME, "worker")],
            "the teardown removed exactly this row's realization"
        );

        f.reconnect(2);
        f.target.script_adoption(GuestAdoption::Missing);
        d.delete(&mut f.fixture.ctx).await.expect("a repeated delete converges");
        assert_eq!(f.target.deleted().len(), 1, "an absent realization is not deleted twice");
    }

    /// Session loss makes the target unavailable: the row stops reading ready,
    /// no frame is issued, and the reconnect drives a fresh adoption (R21).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_lost_guest_session_quarantines_the_incarnation_until_the_reconnect_adopts() {
        let mut f = GuestFixture::new(guest_row());
        f.target.script_adoption(FakeGuestTarget::live(1));
        f.target.script_observation(TargetObservation::Ready { session_generation: 1 });
        let mut d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;
        d.recover(&mut f.fixture.ctx).await.expect("recovery adopts");
        d.reconcile(&mut f.fixture.ctx).await.expect("the live pass is satisfied");

        f.disconnect();
        let failure = d.reconcile(&mut f.fixture.ctx).await.expect_err("the target cannot answer");
        let report = failure.report();
        assert_eq!(report.code(), "process-provider-effect-failed");
        assert!(report.retryable(), "a lost session is not terminal");
        // The pass itself fails, which is what makes the row non-ready: a
        // driver that published readiness over an unreachable target would be
        // claiming an incarnation it cannot observe.
        assert!(
            f.target.realized().is_empty(),
            "no realization is issued over a target that cannot answer"
        );

        f.reconnect(2);
        f.target.script_adoption(FakeGuestTarget::live(2));
        assert_eq!(
            d.reconcile(&mut f.fixture.ctx).await.expect("the reconnected pass"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            f.fixture.ctx.status::<ProcessDriverStatus>().copied(),
            Some(ProcessDriverStatus::Ready { adopted: true }),
            "the reconnect re-observes evidence before the row reads ready again"
        );
    }

    /// Prepared `EndpointBinding` delivery reaches the Guest exactly when the
    /// sealed authority lease revalidated, and not one moment earlier
    /// (KTD6, R18).
    mod binding_delivery {
        use super::*;
        use crate::driver::PROCESS_RESYNC;
        use d2b_contracts_resource::v3::execution_policy::BoundedToken;
        use d2b_contracts_resource::v3::{
            BindingArbitration, BindingRealizationFacet, BindingSourceDecision,
            EndpointAttachmentKind, EndpointBindingSpec, RequestedRights,
        };
        use d2b_resource_runtime::context::{WatchId, WatchRegistration};
        use d2b_resource_runtime::manager::ResourceView;
        use d2b_resource_runtime::resource::ResourceStatus;

        const ENDPOINT_NAME: &str = "relay";
        const BINDING_NAME: &str = "relay";
        const CONSUMER: &str = "Process/worker";

        /// The one-shot row's own consumer reference: the one-shot arm is
        /// gated by the same evidence, for the consumer it names.
        const ONE_SHOT_CONSUMER: &str = "EphemeralProcess/activation-nixos--runner--gen-1";

        const SLOT: &str = "slot-0";
        const INCARNATION: &str = "incarnation-1";
        const DEPENDENCY: &str = "revision-1";
        const OWNER_UID: [u8; 16] = [0x51; 16];

        fn endpoint_key() -> ResourceKey {
            ResourceKey::new("work", "Endpoint", ENDPOINT_NAME)
        }

        fn binding_key() -> ResourceKey {
            ResourceKey::new("work", "EndpointBinding", BINDING_NAME)
        }

        /// The committed relationship bytes naming one exact consumer: the
        /// publication and the row must name the same consumer, or the gate
        /// reads the evidence as foreign.
        fn binding_spec_bytes_for(consumer: &str) -> Vec<u8> {
            let spec = EndpointBindingSpec::new(
                ResourceRef::parse(&format!("Endpoint/{ENDPOINT_NAME}")).expect("endpoint ref"),
                ResourceRef::parse(consumer).expect("consumer ref"),
                EndpointAttachmentKind::Connect,
                BoundedToken::parse(SLOT).expect("slot token"),
                BindingSourceDecision::new(
                    vec![RequestedRights::Consume],
                    BindingArbitration::Shared,
                    vec![BindingRealizationFacet::EndpointDescriptor],
                )
                .expect("source decision"),
            )
            .expect("binding spec");
            serde_json::to_vec(&spec).expect("a committed binding spec serializes")
        }

        fn binding_spec_bytes() -> Vec<u8> {
            binding_spec_bytes_for(CONSUMER)
        }

        fn view(
            key: ResourceKey,
            uid: [u8; 16],
            generation: u64,
            status_projection: Option<serde_json::Value>,
            spec: Vec<u8>,
        ) -> ResourceView {
            ResourceView {
                key,
                uid,
                generation,
                deleting: false,
                provenance: ResourceProvenance::Resource,
                spec,
                metadata: Vec::new(),
                owner_key: None,
                status: Some(ResourceStatus::Ready),
                status_generation: Some(generation),
                status_projection,
            }
        }

        fn endpoint_view_for(consumer: &str) -> ResourceView {
            view(
                endpoint_key(),
                [0x62; 16],
                2,
                Some(serde_json::json!({
                    "endpoint": {
                        "incarnation": INCARNATION,
                        "bindings": [{
                            "name": BINDING_NAME,
                            "endpoint": format!("Endpoint/{ENDPOINT_NAME}"),
                            "consumer": consumer,
                            "slot": SLOT,
                            "authorizationDigest": "authorization-1",
                            "dependencyRevision": DEPENDENCY,
                        }],
                    },
                })),
                Vec::new(),
            )
        }

        fn endpoint_view() -> ResourceView {
            endpoint_view_for(CONSUMER)
        }

        /// One relationship view for one consumer: delivered at this exact
        /// realization, or not delivered at all.
        fn binding_view_for(consumer: &str, incarnation: &str, delivered: bool) -> ResourceView {
            view(
                binding_key(),
                [0x61; 16],
                3,
                Some(serde_json::json!({
                    "binding": if delivered {
                        serde_json::json!({
                            "state": "delivered",
                            "generation": 3,
                            "incarnation": incarnation,
                        })
                    } else {
                        serde_json::json!({ "state": "undelivered" })
                    },
                })),
                binding_spec_bytes_for(consumer),
            )
        }

        fn binding_view_over(incarnation: &str, delivered: bool) -> ResourceView {
            binding_view_for(CONSUMER, incarnation, delivered)
        }

        fn binding_view(delivered: bool) -> ResourceView {
            binding_view_over(INCARNATION, delivered)
        }

        fn endpoint_row() -> StoredDesiredResource {
            StoredDesiredResource {
                key: endpoint_key(),
                uid: [0x62; 16],
                generation: 2,
                owner_uid: Some(OWNER_UID),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: Vec::new(),
                metadata: Vec::new(),
                created_at: 0,
            }
        }

        /// The owner-scoped manager one gated pass reads: the committed rows
        /// its owner holds, the views those rows published, and the evidence
        /// the launch gate revalidates immediately before the effect.
        struct BindingManager {
            rows: Vec<StoredDesiredResource>,
            views: parking_lot::Mutex<Vec<ResourceView>>,
            reads: parking_lot::Mutex<Vec<ResourceKey>>,
        }

        #[async_trait::async_trait]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        impl ManagerEndpoint for BindingManager {
            async fn ensure_child(
                &self,
                _parent: &ResourceKey,
                _child: ChildEnsure,
            ) -> Result<EnsureOutcome, ResourceError> {
                Err(ResourceError::ManagerRejected {
                    reason: "unexpected ensure_child".into(),
                })
            }

            async fn get(
                &self,
                key: &ResourceKey,
            ) -> Result<Option<StoredDesiredResource>, ResourceError> {
                Ok(self.rows.iter().find(|row| row.key == *key).cloned())
            }

            async fn view(
                &self,
                key: &ResourceKey,
            ) -> Result<Option<ResourceView>, ResourceError> {
                self.reads.lock().push(key.clone()); // async-gate-allow: fixture records under a short guard and holds no await
                Ok(self.views.lock().iter().find(|view| view.key == *key).cloned()) // async-gate-allow: fixture reads under a short guard and holds no await
            }

            async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
                Err(ResourceError::ManagerRejected {
                    reason: "unexpected delete".into(),
                })
            }

            async fn list_owned(
                &self,
                owner_uid: [u8; 16],
            ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
                Ok(self
                    .rows
                    .iter()
                    .filter(|row| row.owner_uid == Some(owner_uid))
                    .cloned()
                    .collect())
            }

            async fn register_watch(
                &self,
                _subscriber: &ResourceKey,
                _registration: WatchRegistration,
            ) -> Result<WatchId, ResourceError> {
                Ok(WatchId(1))
            }

            async fn cancel_watch(&self, _id: WatchId) -> Result<(), ResourceError> {
                Ok(())
            }
        }

        impl BindingManager {
            /// The keys whose view this manager served, in order: how a case
            /// observes that the gate read the evidence a second time.
            #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
            fn view_reads(&self) -> Vec<ResourceKey> {
                self.reads.lock().clone() // async-gate-allow: fixture reads under a short guard and holds no await
            }
        }

        fn gated_guest_fixture() -> (GuestFixture, Arc<BindingManager>) {
            let mut row = guest_row();
            row.owner_uid = Some(OWNER_UID);
            // The durable owner uid only resolves through the row's own
            // authored owner reference; the launch identity refuses a uid with
            // no reference to link it to.
            row.metadata =
                br#"{"annotations":{},"labels":{},"ownerRef":"Provider/runtime-local"}"#.to_vec();
            let manager = Arc::new(BindingManager {
                rows: vec![endpoint_row()],
                views: parking_lot::Mutex::new(vec![endpoint_view(), binding_view(true)]),
                reads: parking_lot::Mutex::new(Vec::new()),
            });
            (
                GuestFixture::with(row, Arc::clone(&manager) as Arc<dyn ManagerEndpoint>),
                manager,
            )
        }

        /// One endpoint row whose committed bytes name the producer that
        /// realized it, which is where the producer a barrier reads lives.
        fn endpoint_row_with_spec(spec: &[u8]) -> StoredDesiredResource {
            StoredDesiredResource {
                spec: spec.to_vec(),
                ..endpoint_row()
            }
        }

        /// The `Endpoint` spec document a display Provider commits, reduced
        /// to the one field the retirement barrier reads.
        fn endpoint_spec_naming(producer: &str) -> Vec<u8> {
            serde_json::to_vec(&serde_json::json!({
                "providerRef": "Provider/display-wayland",
                "producerRef": producer,
            }))
            .expect("an endpoint spec document")
        }

        /// The committed relationship the endpoint published, owned by the
        /// endpoint that published it - so it is a sibling of the consumer
        /// and never one of the consumer's own children.
        fn binding_row_for(consumer: &str) -> StoredDesiredResource {
            StoredDesiredResource {
                key: binding_key(),
                uid: [0x61; 16],
                generation: 3,
                owner_uid: Some([0x62; 16]),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: binding_spec_bytes_for(consumer),
                metadata: Vec::new(),
                created_at: 0,
            }
        }

        fn binding_row() -> StoredDesiredResource {
            binding_row_for(CONSUMER)
        }

        /// One host `Process` row owned by the same owner the `Endpoint` rows
        /// are, so the launch gate and the retirement barrier read one
        /// neighbourhood.
        fn owned_host_row() -> StoredDesiredResource {
            let mut row = test_row();
            row.owner_uid = Some(OWNER_UID);
            row.metadata =
                br#"{"annotations":{},"labels":{},"ownerRef":"Provider/runtime-local"}"#.to_vec();
            row
        }

        fn binding_manager(
            rows: Vec<StoredDesiredResource>,
            views: Vec<ResourceView>,
        ) -> Arc<BindingManager> {
            Arc::new(BindingManager {
                rows,
                views: parking_lot::Mutex::new(views),
                reads: parking_lot::Mutex::new(Vec::new()),
            })
        }

        /// Replace one published view, which is how a case states that the
        /// evidence under a live row moved.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn publish(manager: &BindingManager, view: ResourceView) {
            manager.views.lock().retain(|published| published.key != view.key); // async-gate-allow: fixture rewrites under a short guard and holds no await
            manager.views.lock().push(view); // async-gate-allow: fixture rewrites under a short guard and holds no await
        }

        /// The delivery a live Guest Process receives is exactly the sealed
        /// relationship set, and only because the lease revalidated.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        #[tokio::test]
        async fn prepared_endpoint_delivery_reaches_the_guest_only_with_a_live_lease() {
            let (mut f, _manager) = gated_guest_fixture();
            let mut d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;

            assert!(matches!(
                d.reconcile(&mut f.fixture.ctx).await.expect("the delivered pass"),
                ReconcileOutcome::InProgress { .. }
            ));
            yield_until_effects_settled().await;
            let realized = f.target.realized();
            assert_eq!(realized.len(), 1, "one realize frame carries the delivery");
            let realization = crate::worker_launch::GuestProcessRealization::decode(&realized[0].1)
                .expect("the delivered realization decodes");
            assert_eq!(
                realization.deliveries().len(),
                1,
                "exactly the sealed relationship travels"
            );
            assert_eq!(realization.deliveries()[0].binding_ref(), "EndpointBinding/relay");
            assert_eq!(realization.deliveries()[0].slot(), SLOT);
            assert_eq!(realization.deliveries()[0].incarnation(), INCARNATION);
        }

        /// The committed source row whose actor has published nothing for its
        /// current generation: the row and its view are both there, and the
        /// projection that states what it would grant is not.
        fn unpublished_endpoint_view() -> ResourceView {
            view(endpoint_key(), [0x62; 16], 2, None, Vec::new())
        }

        /// A source that PUBLISHED a projection, but not the layer naming the
        /// relationships it grants. The endpoint family writes that layer on
        /// every pass - an endpoint granting nothing writes an EMPTY array -
        /// so this is a publication that carries no statement either way.
        fn endpoint_view_without_the_publication_layer() -> ResourceView {
            view(
                endpoint_key(),
                [0x62; 16],
                2,
                Some(serde_json::json!({ "endpoint": { "readiness": "realizing" } })),
                Vec::new(),
            )
        }

        /// The guest row whose launch gate reads the same owner-scoped
        /// neighbourhood, over the manager the case chooses.
        fn guest_fixture_over(manager: &Arc<BindingManager>) -> GuestFixture {
            let mut row = guest_row();
            row.owner_uid = Some(OWNER_UID);
            // The durable owner uid only resolves through the row's own
            // authored owner reference; the launch identity refuses a uid with
            // no reference to link it to.
            row.metadata =
                br#"{"annotations":{},"labels":{},"ownerRef":"Provider/runtime-local"}"#.to_vec();
            GuestFixture::with(row, Arc::clone(manager) as Arc<dyn ManagerEndpoint>)
        }

        /// A source that has COMMITTED and published nothing proves nothing
        /// about what it would grant, so the launch defers: no launch effect,
        /// and no realize frame for a row committed to a Guest target.
        ///
        /// The relationship this case also commits is committed AND delivered,
        /// so nothing in the answer can be explained by that row being absent:
        /// the only unproven fact is the source's own silence. Reading that
        /// silence as "this Process requires no `EndpointBinding`" is what let
        /// a realize frame leave with an EMPTY delivery set and a row start
        /// over endpoint access it was never granted (R18, R21).
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        #[tokio::test]
        async fn a_committed_endpoint_that_published_nothing_blocks_the_launch() {
            let manager = binding_manager(
                vec![endpoint_row(), binding_row()],
                vec![unpublished_endpoint_view(), binding_view(true)],
            );
            let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
                active: false,
                ..FakeFacetsConfig::default()
            }));

            let mut host = fixture_with(
                owned_host_row(),
                Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            );
            let mut d = driver(Arc::clone(&fake)).await;
            assert_eq!(
                d.reconcile(&mut host.ctx).await.expect("the unproven pass"),
                ReconcileOutcome::RetryScheduled,
                "an unproven source defers the launch instead of admitting it needs no binding"
            );
            yield_until_effects_settled().await;
            assert!(
                fake.launch_calls().is_empty(),
                "a source that published nothing starts nothing: {:?}",
                fake.launch_calls()
            );
            assert!(
                manager.view_reads().contains(&endpoint_key()),
                "the pass READ that source's view rather than passing it by"
            );
            assert!(
                host.requeue_calls().contains(&PROCESS_RESYNC),
                "and it schedules the cadence that re-reads the source: {:?}",
                host.requeue_calls()
            );

            // The same evidence over a row committed to a Guest target. For
            // that row the realize frame IS its launch, so a frame carrying
            // no delivery is the fail-open arriving in the Guest.
            let mut guest = guest_fixture_over(&manager);
            let mut guest_driver = driver(Arc::clone(&fake)).await;
            assert_eq!(
                guest_driver
                    .reconcile(&mut guest.fixture.ctx)
                    .await
                    .expect("the unproven pass"),
                ReconcileOutcome::RetryScheduled
            );
            yield_until_effects_settled().await;
            assert!(
                guest.target.realized().is_empty(),
                "no realize frame crosses the session while the source has published nothing: {:?}",
                guest.target.realized()
            );
        }

        /// A projection that carries no `/endpoint/bindings` layer is not the
        /// source saying it grants nothing, because that source writes an
        /// EMPTY layer when it grants nothing. An unread publication is not a
        /// publication.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        #[tokio::test]
        async fn an_endpoint_publication_without_the_binding_layer_defers_the_launch() {
            let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
                active: false,
                ..FakeFacetsConfig::default()
            }));
            let manager = binding_manager(
                vec![endpoint_row(), binding_row()],
                vec![endpoint_view_without_the_publication_layer(), binding_view(true)],
            );
            let mut f = fixture_with(
                owned_host_row(),
                Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            );
            let mut d = driver(Arc::clone(&fake)).await;

            assert_eq!(
                d.reconcile(&mut f.ctx).await.expect("the unread publication pass"),
                ReconcileOutcome::RetryScheduled
            );
            yield_until_effects_settled().await;
            assert!(
                fake.launch_calls().is_empty(),
                "a publication this reader cannot take the publication set from grants nothing \
                 and starts nothing: {:?}",
                fake.launch_calls()
            );
        }

        /// The owner-scoped listing named an endpoint and the manager then
        /// answered that it holds no view for it. Skipping that row would drop
        /// an expectation this launch may owe, so the pass defers and reads
        /// the sibling set again on the next one.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        #[tokio::test]
        async fn an_endpoint_listed_and_then_absent_is_not_skipped_out_of_the_gate() {
            let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
                active: false,
                ..FakeFacetsConfig::default()
            }));
            // The relationship row is committed and delivered; only the source
            // view the listing named is missing.
            let manager = binding_manager(
                vec![endpoint_row(), binding_row()],
                vec![binding_view(true)],
            );
            let mut f = fixture_with(
                owned_host_row(),
                Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            );
            let mut d = driver(Arc::clone(&fake)).await;

            assert_eq!(
                d.reconcile(&mut f.ctx).await.expect("the torn sibling pass"),
                ReconcileOutcome::RetryScheduled
            );
            yield_until_effects_settled().await;
            assert!(
                fake.launch_calls().is_empty(),
                "an endpoint this pass could not prove is not an endpoint this launch does not \
                 need: {:?}",
                fake.launch_calls()
            );
            assert!(
                f.requeue_calls().contains(&PROCESS_RESYNC),
                "and the retry re-derives the sibling set from the manager: {:?}",
                f.requeue_calls()
            );
        }

        /// A lease that no longer revalidates delivers nothing: the launch
        /// defers and no realize frame - and so no endpoint delivery - is
        /// issued over the session.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        #[tokio::test]
        async fn a_lease_that_moved_before_the_effect_delivers_nothing() {
            let (mut f, manager) = gated_guest_fixture();
            let mut d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;

            assert!(matches!(
                d.reconcile(&mut f.fixture.ctx).await.expect("the delivered pass"),
                ReconcileOutcome::InProgress { .. }
            ));
            yield_until_effects_settled().await;
            assert_eq!(f.target.realized().len(), 1, "the first pass delivered");

            // The endpoint withdrew the delivery: its relationship row no
            // longer publishes one at the incarnation the lease sealed, so the
            // lease revalidation immediately before the effect fails closed.
            manager.views.lock().retain(|view| view.key != binding_key()); // async-gate-allow: fixture rewrites under a short guard and holds no await
            manager.views.lock().push(binding_view(false)); // async-gate-allow: fixture rewrites under a short guard and holds no await
            assert_eq!(
                d.reconcile(&mut f.fixture.ctx).await.expect("the moved pass"),
                ReconcileOutcome::RetryScheduled,
                "a relationship that is no longer delivered defers the launch"
            );
            assert_eq!(f.target.realized().len(), 1, "a revoked lease delivers nothing");
        }

        /// A delivery withdrawn from a running `Process` stops the incarnation
        /// that was launched over it, and nothing relaunches until the gate
        /// opens again (AE13, R21, R22).
        ///
        /// The withdrawal is an authority change, not a not-yet: the process
        /// is running over access its source no longer grants. The pass that
        /// observes it stops the exact identity it verified, publishes a
        /// non-ready classification in the same pass, and issues no launch -
        /// so a later pass that still sees the withdrawal stays stopped
        /// rather than restarting over revoked authority.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        #[tokio::test]
        async fn a_withdrawn_binding_stops_the_running_process_and_forbids_a_relaunch() {
            let fake = Arc::new(FakeFacets::new(FakeFacetsConfig::default()));
            let manager = binding_manager(
                vec![endpoint_row(), binding_row()],
                vec![endpoint_view(), binding_view(true)],
            );
            let mut f = fixture_with(
                owned_host_row(),
                Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            );
            let mut d = driver(Arc::clone(&fake)).await;

            expect_in_progress(d.reconcile(&mut f.ctx).await);
            yield_until_effects_settled().await;
            assert_eq!(fake.launch_calls().len(), 1, "the delivered gate admits one launch");

            // The next pass adopts the exact live identity and reads ready.
            fake.push_adoption(ProviderAdoption::Adopted(adopted_report()));
            assert_eq!(
                d.reconcile(&mut f.ctx).await.expect("the adopted pass"),
                ReconcileOutcome::Satisfied
            );
            assert_eq!(
                f.ctx.status::<ProcessDriverStatus>().copied(),
                Some(ProcessDriverStatus::Ready { adopted: true }),
                "a delivered relationship is what the row is ready over"
            );

            // The relationship stopped being delivered at the realization this
            // row launched over.
            publish(&manager, binding_view(false));
            assert_eq!(
                d.reconcile(&mut f.ctx).await.expect("the withdrawn pass"),
                ReconcileOutcome::RetryScheduled
            );
            assert_eq!(
                fake.stop_calls().len(),
                1,
                "the verified live incarnation stops the moment the delivery is withdrawn"
            );
            assert_eq!(
                f.ctx.status::<ProcessDriverStatus>().copied(),
                Some(ProcessDriverStatus::Succeeded {
                    code: "binding-delivery-withdrawn"
                }),
                "and the row stops reading ready behind an incarnation it no longer \
                 has authority for"
            );
            assert_eq!(
                fake.launch_calls().len(),
                1,
                "no relaunch is issued over a withdrawn delivery"
            );

            // Every later pass over the same withdrawal stays stopped.
            fake.set_active(false);
            assert_eq!(
                d.reconcile(&mut f.ctx).await.expect("the still-withdrawn pass"),
                ReconcileOutcome::RetryScheduled
            );
            assert_eq!(fake.launch_calls().len(), 1, "and still no relaunch");
        }

        /// Evidence published about ANOTHER incarnation is foreign, which is
        /// terminal for the launch - and it stops the live process on exactly
        /// the same terms a withdrawal does (AE13, R21).
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        #[tokio::test]
        async fn foreign_binding_evidence_stops_the_running_process_too() {
            let fake = Arc::new(FakeFacets::new(FakeFacetsConfig::default()));
            let manager = binding_manager(
                vec![endpoint_row(), binding_row()],
                vec![endpoint_view(), binding_view(true)],
            );
            let mut f = fixture_with(
                owned_host_row(),
                Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            );
            let mut d = driver(Arc::clone(&fake)).await;

            fake.push_adoption(ProviderAdoption::Adopted(adopted_report()));
            expect_in_progress(d.reconcile(&mut f.ctx).await);
            yield_until_effects_settled().await;
            assert_eq!(
                d.reconcile(&mut f.ctx).await.expect("the adopted pass"),
                ReconcileOutcome::Satisfied
            );
            assert_eq!(
                f.ctx.status::<ProcessDriverStatus>().copied(),
                Some(ProcessDriverStatus::Ready { adopted: true })
            );

            // A delivery somebody else re-derived: the evidence names a
            // different realization than the one this row launched over.
            publish(&manager, binding_view_over("incarnation-OTHER", true));
            d.reconcile(&mut f.ctx)
                .await
                .expect_err("foreign evidence refuses the pass");
            assert_eq!(
                fake.stop_calls().len(),
                1,
                "the live incarnation stops before the terminal refusal is reported"
            );
            assert_eq!(fake.launch_calls().len(), 1, "and nothing relaunches");
            assert!(
                !matches!(
                    f.ctx.status::<ProcessDriverStatus>().copied(),
                    Some(ProcessDriverStatus::Ready { .. })
                ),
                "the row is not ready behind an incarnation it can no longer attribute"
            );
        }

        /// Recovery answers the same launch binding gate the reconcile arms
        /// answer, and adoption is fenced exactly as the launch is (KTD6, R18).
        ///
        /// A restart is the pass that most needs the fence: a delivery
        /// withdrawn while this daemon was down is invisible to every effect
        /// it has already issued, so a survivor is re-admitted by evidence
        /// read NOW, never by the fact that its process still exists. A
        /// `Pending` answer adopts nothing, signals nothing, and reports
        /// `Missing` - a survivor is not a verified identity, so unproven
        /// evidence is no reason to stop one either.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        #[tokio::test]
        async fn recovery_adopts_nothing_over_a_delivery_withdrawn_while_it_was_down() {
            let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
                adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
                ..FakeFacetsConfig::default()
            }));
            let manager = binding_manager(
                vec![endpoint_row(), binding_row()],
                vec![endpoint_view(), binding_view(false)],
            );
            let mut f = fixture_with(
                owned_host_row(),
                Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            );
            let mut d = driver(Arc::clone(&fake)).await;

            assert_eq!(
                d.recover(&mut f.ctx)
                    .await
                    .expect("an unproven delivery is not a failure"),
                RecoveryOutcome::Missing,
                "nothing this pass cannot prove is adopted"
            );
            assert!(
                !fake.call_order().contains(&"adopt"),
                "the adoption classification never runs over withdrawn evidence"
            );
            assert!(
                fake.stop_calls().is_empty(),
                "and no signal reaches the survivor: recovery verified no identity"
            );
            assert!(
                !matches!(
                    f.ctx.status::<ProcessDriverStatus>().copied(),
                    Some(ProcessDriverStatus::Ready { .. })
                ),
                "the row never reads ready over access its source withdrew"
            );
        }

        /// The positive case the gate must not break: a survivor whose
        /// delivery still holds at the realization this row launched over is
        /// adopted - and the sealed lease is revalidated against freshly read
        /// evidence immediately before the adoption effect, so the readiness
        /// the pass publishes is the one the current evidence proves.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        #[tokio::test]
        async fn recovery_adopts_a_survivor_whose_delivery_revalidates() {
            let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
                adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
                ..FakeFacetsConfig::default()
            }));
            let manager = binding_manager(
                vec![endpoint_row(), binding_row()],
                vec![endpoint_view(), binding_view(true)],
            );
            let mut f = fixture_with(
                owned_host_row(),
                Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            );
            let mut d = driver(Arc::clone(&fake)).await;

            assert_eq!(
                d.recover(&mut f.ctx).await.expect("the revalidated survivor"),
                RecoveryOutcome::Adopted
            );
            assert!(fake.call_order().contains(&"adopt"));
            assert_eq!(
                f.ctx.status::<ProcessDriverStatus>().copied(),
                Some(ProcessDriverStatus::Ready { adopted: true })
            );
            assert!(fake.launch_calls().is_empty(), "adopted without launch");
            assert_eq!(
                manager
                    .view_reads()
                    .iter()
                    .filter(|key| *key == &binding_key())
                    .count(),
                2,
                "the sealed lease is revalidated immediately before the adoption effect"
            );
        }

        /// The one-shot arm answers the same gate in recovery: its survivor
        /// over a withdrawn delivery is not adopted either.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        #[tokio::test]
        async fn recovery_adopts_no_one_shot_survivor_over_a_withdrawn_delivery() {
            let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
                adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
                ..FakeFacetsConfig::default()
            }));
            let manager = binding_manager(
                vec![endpoint_row(), binding_row_for(ONE_SHOT_CONSUMER)],
                vec![
                    endpoint_view_for(ONE_SHOT_CONSUMER),
                    binding_view_for(ONE_SHOT_CONSUMER, INCARNATION, false),
                ],
            );
            let mut row = ephemeral_row();
            row.owner_uid = Some(OWNER_UID);
            row.metadata =
                br#"{"annotations":{},"labels":{},"ownerRef":"Provider/runtime-local"}"#.to_vec();
            let mut f = fixture_with(row, Arc::clone(&manager) as Arc<dyn ManagerEndpoint>);
            let mut d = driver(Arc::clone(&fake)).await;

            assert_eq!(
                d.recover(&mut f.ctx)
                    .await
                    .expect("an unproven delivery is not a failure"),
                RecoveryOutcome::Missing,
                "the one-shot arm adopts nothing it cannot prove either"
            );
            assert!(
                !fake.call_order().contains(&"adopt-ephemeral"),
                "the one-shot adoption classification never runs over withdrawn evidence"
            );
            assert!(
                fake.stop_calls().is_empty(),
                "and no signal reaches the one-shot survivor"
            );
        }

        /// The consumer identity outlives its own release (R22).
        ///
        /// The broker derives the principal a revoke names from the consumer's
        /// own committed reference, so the row must not retire while a
        /// relationship it consumes - or an `Endpoint` it produces - is still
        /// committed. The effect has already stopped by the time this runs;
        /// this is what keeps the identity available until they are gone.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        #[tokio::test]
        async fn process_row_retirement_waits_for_its_bindings_and_produced_endpoints() {
            // 1. A relationship this exact Process consumes is committed: the
            //    relationship row is owned by the ENDPOINT that published it,
            //    so the barrier has to resolve it from the publication rather
            //    than from the owner's own children.
            let manager = binding_manager(
                vec![
                    endpoint_row_with_spec(&endpoint_spec_naming("Process/other")),
                    binding_row(),
                ],
                vec![endpoint_view(), binding_view(true)],
            );
            let mut f = fixture_with(
                owned_host_row(),
                Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            );
            let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
                adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
                ..FakeFacetsConfig::default()
            }));
            let mut d = driver(Arc::clone(&fake)).await;
            let held = d
                .delete(&mut f.ctx)
                .await
                .expect_err("a committed relationship holds the consumer row");
            assert_eq!(
                held.class(),
                FailureClass::Retryable,
                "the barrier defers the row rather than failing it"
            );
            assert!(
                !fake.stop_calls().is_empty(),
                "the EFFECT stopped first: the row is held, the process is gone"
            );

            // 2. An `Endpoint` this exact Process produces holds it too, with
            //    no relationship row left at all.
            let manager = binding_manager(
                vec![endpoint_row_with_spec(&endpoint_spec_naming(CONSUMER))],
                vec![endpoint_view()],
            );
            let mut f = fixture_with(
                owned_host_row(),
                Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            );
            let mut d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;
            assert!(
                d.delete(&mut f.ctx).await.is_err(),
                "an endpoint this row produces holds its retirement"
            );

            // 3. Neither holds it: the endpoint belongs to another producer and
            //    the relationship this row consumed is gone.
            let manager = binding_manager(
                vec![endpoint_row_with_spec(&endpoint_spec_naming("Process/other"))],
                vec![endpoint_view()],
            );
            let mut f = fixture_with(
                owned_host_row(),
                Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            );
            let mut d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;
            let outcome = d.delete(&mut f.ctx).await;
            assert!(
                outcome.is_ok(),
                "with nothing of its own committed, the row retires: {outcome:?}"
            );

            // 4. A sibling whose committed bytes cannot be read fails closed:
            //    a row this pass cannot read is a row whose release this pass
            //    cannot prove.
            let manager = binding_manager(
                vec![endpoint_row_with_spec(b"not-an-endpoint-document")],
                vec![endpoint_view()],
            );
            let mut f = fixture_with(
                owned_host_row(),
                Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            );
            let mut d = driver(Arc::new(FakeFacets::new(FakeFacetsConfig::default()))).await;
            assert!(
                d.delete(&mut f.ctx).await.is_err(),
                "an unreadable endpoint row retains the consumer row"
            );
        }
    }
}
