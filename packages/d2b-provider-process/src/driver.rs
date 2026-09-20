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

use crate::effects::{ProcessDriverEffects, ProviderAdoption, ProviderLiveness};
use crate::effects_service::{PROCESS_EFFECTS_SERVICE, ProcessEffectsService};
use crate::execution::{ExecutionMode, execution_target_allowed};
use crate::facets::ProcessEffectFacets;
use crate::identity::{ProcessFamilySpec, ProcessResourceIdentity};
use crate::launch_identity::{LaunchRow, resolve_launch_identity};
use crate::operations::process_family_operations;
use crate::worker_launch::{ServingWorkerLaunch, ServingWorkerRoot};
use d2b_contracts_resource::v3::{
    AdoptionPolicy, ControllerGeneration, ResourceGeneration, ResourceName, ResourceRef,
    ResourceSpec, ResourceTypeName as ContractResourceTypeName, ResourceUid, ZoneId,
    process::{DesiredLifecycle, EphemeralProcessSpec, ProcessSpec, RestartClass},
};
use d2b_process_conformance::{GuestExecutionBinding, ProcessStatusReport};
use d2b_resource_runtime::context::{
    EffectCompleted, EffectResult, ResourceContext, SpecDecoder, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_types::{AllowedSources, DriverDescriptor, OperationDef, ServiceDecl, WellKnownType};

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
    let mut bytes = *bytes;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    );
    ResourceUid::parse(text).ok()
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
        self.count.load(Ordering::SeqCst)
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
        self.count.fetch_add(1, Ordering::SeqCst);
    }

    /// Record one consumed restart; the next reconcile pass schedules the
    /// policy backoff exactly once.
    fn record_restart(&self) {
        self.consume_restart();
        self.restart_scheduled.store(true, Ordering::SeqCst);
    }

    fn take_restart_scheduled(&self) -> bool {
        self.restart_scheduled.swap(false, Ordering::SeqCst)
    }

    fn mark_exhausted(&self) {
        self.exhausted.store(true, Ordering::SeqCst);
    }

    fn is_exhausted(&self) -> bool {
        self.exhausted.load(Ordering::SeqCst)
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

/// The resource verbs both Process-family types support.
///
/// Derived from the v3 resource plane's converted-type verb surface: the
/// closed `RoleResourceVerb` set minus the two Credential-scoped credential
/// verbs (`use-credential`, `admin-credential`), which the plane gates to the
/// `Credential` type. Every converted type is served by the same manager
/// verbs, and Role rules and the typed CLI nouns resolve their gating from
/// this declaration.
pub(crate) const PROCESS_FAMILY_VERBS: &[&str] = &[
    "get",
    "list",
    "watch",
    "create",
    "update-spec",
    "update-status",
    "update-metadata",
    "update-finalizers",
    "delete",
];

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
/// its owning `Guest`; and a controller row binds its committed `Provider`
/// identity (KTD7).
pub(crate) const PROCESS_FAMILY_READS: &[WellKnownType] = &[
    WellKnownType::VOLUME_BINDING,
    WellKnownType::VOLUME,
    WellKnownType::DEVICE,
    WellKnownType::GUEST,
    WellKnownType::PROVIDER,
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
            verbs: PROCESS_FAMILY_VERBS,
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
    started_at: parking_lot::Mutex<Option<tokio::time::Instant>>,
    /// The terminal outcome, once observed: the clock for the retention TTL.
    completed: parking_lot::Mutex<Option<EphemeralCompletion>>,
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
        self.started.load(Ordering::SeqCst)
    }

    fn mark_started(&self) {
        let mut started_at = self.started_at.lock();
        if started_at.is_none() {
            *started_at = Some(tokio::time::Instant::now());
        }
        self.started.store(true, Ordering::SeqCst);
    }

    fn started_at(&self) -> Option<tokio::time::Instant> {
        *self.started_at.lock()
    }

    /// Record the one-shot terminal state once; a second observation keeps
    /// the first (the TTL clock must not restart).
    fn finish(&self, failed: bool, code: &'static str) -> EphemeralCompletion {
        let mut completed = self.completed.lock();
        *completed.get_or_insert(EphemeralCompletion {
            failed,
            code,
            at: tokio::time::Instant::now(),
        })
    }

    fn completed(&self) -> Option<EphemeralCompletion> {
        *self.completed.lock()
    }

    /// Test-only: backdate the runtime-deadline clock.
    #[cfg(test)]
    fn backdate_started(&self, elapsed: Duration) {
        let mut started_at = self.started_at.lock();
        if let Some(at) = started_at.as_mut() {
            *at -= elapsed;
        }
        self.started.store(true, Ordering::SeqCst);
    }

    /// Test-only: backdate the retention TTL clock.
    #[cfg(test)]
    fn backdate_completed(&self, elapsed: Duration) {
        if let Some(completion) = self.completed.lock().as_mut() {
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
        self.watching.load(Ordering::SeqCst)
    }

    fn mark_watching(&self) {
        self.watching.store(true, Ordering::SeqCst);
    }

    /// The observed process is gone and the pass that saw the exit hands the
    /// relaunch back to the adoption path, so the next pass launches instead
    /// of probing an identity the provider has already released.
    fn mark_exited(&self) {
        self.watching.store(false, Ordering::SeqCst);
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
                    let binding = serde_json::from_slice::<ResourceSpec>(&row.spec)
                        .ok()
                        .and_then(|envelope| {
                            serde_json::from_slice::<
                                d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec,
                            >(&envelope.base().to_canonical_bytes())
                            .ok()
                        });
                    if let Some(binding) = binding.as_ref() {
                        worker_launch = self.serving_worker_launch(ctx, binding, op).await;
                    }
                    binding.map(|binding| binding.execution_ref().clone())
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
    ) -> Option<ServingWorkerLaunch> {
        let (_, spec) = self.decoded_spec(ctx, op).ok()?;
        if spec.execution().template().as_str() != d2b_provider_volume_virtiofs::WORKER_TEMPLATE {
            return None;
        }
        let volume_key = ResourceKey::new(
            self.zone.as_str(),
            "Volume",
            binding.volume_ref().name().as_str(),
        );
        let row = ctx.get(&volume_key).await.ok().flatten()?;
        let volume = serde_json::from_slice::<ResourceSpec>(&row.spec)
            .ok()
            .and_then(|envelope| {
                serde_json::from_slice::<d2b_contracts_resource::v3::volume::VolumeSpec>(
                    &envelope.base().to_canonical_bytes(),
                )
                .ok()
            })?;
        let view = volume.views().get(binding.view().as_str())?;
        let attachment = volume
            .attachments()
            .iter()
            .find(|attachment| attachment.execution_ref() == binding.execution_ref())?;
        let settings = attachment.settings();
        let source = volume.source();
        let root = match source.settings().kind() {
            d2b_contracts_resource::v3::volume::SourceKind::LocalPath => {
                let policy = source.settings().source_policy_id()?.as_str().to_owned();
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
        Some(ServingWorkerLaunch {
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
        })
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
    fn spawn_launch(
        &mut self,
        ctx: &mut ResourceContext,
        identity: ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ReconcileOutcome, ProcessDriverError> {
        if self.budget.is_exhausted() {
            return Err(self.error(ProcessDriverErrorKind::StartExhausted, DriverOp::Reconcile));
        }
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
            let effect_result = match effects.launch(&identity, &task_spec, LAUNCH_TIMEOUT).await {
                Ok(_) => {
                    if arm_observation {
                        durable.mark_watching();
                    }
                    EffectResult::Completed
                }
                Err(error) => {
                    // The closed classification is what reaches status, and
                    // status is memory-only (R11), so the journal is the only
                    // place the provider's reason for refusing the launch is
                    // observable.
                    // `ResourceRef`'s `Display` is the redaction stub, so both
                    // refs render canonically: the redacting form would make
                    // the only diagnostic for a refused launch unreadable.
                    tracing::warn!(
                        resource = %identity.resource_ref.to_canonical_string(),
                        provider = %identity.provider_ref.to_canonical_string(),
                        error = %error,
                        "process launch failed"
                    );
                    let kind = provider_error_kind(&error);
                    if kind.is_unresolvable_launch() {
                        // The closed spellings no retry can reverse
                        // (`template-not-found`, `resolution-failed`,
                        // `guest-process-not-vmm`): the in-memory budget
                        // cannot mint the missing ticket, so the row fails
                        // instead of relaunching (and warning) forever. The
                        // ephemeral arm classifies its launch the same way.
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
                    } else if budget.allows(&task_spec) {
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
    async fn recover_ephemeral(
        &mut self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<RecoveryOutcome, ProcessDriverError> {
        match self.effects.adopt_ephemeral(identity, spec).await {
            Ok(ProviderAdoption::Adopted(_)) => {
                self.ephemeral.mark_started();
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

    /// The durable arm: preserved adopt/launch/stop-stale behavior.
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
        if spec.desired_lifecycle() == DesiredLifecycle::Stopped {
            // A live identity is stopped through the same exact escalation
            // every other path uses; with no verified identity there is
            // nothing this daemon may signal.
            if self.effects.has_active(
                &identity.zone,
                identity.zone_uid.as_ref(),
                &identity.resource_ref,
            ) {
                self.stop_and_finalize(&identity, spec, DriverOp::Reconcile)
                    .await?;
            }
            return match self.effects.probe(&identity, spec).await {
                // The observed stop: the desired state is realized, so the row
                // may read its terminal status. The identity is handed back to
                // the adoption path (a later `running` spec adopts/launches
                // afresh instead of probing an identity the provider already
                // released).
                Ok(ProviderLiveness::Exited) => {
                    self.durable.mark_exited();
                    ctx.set_status(ProcessDriverStatus::Succeeded {
                        code: "process-stopped",
                    });
                    Ok(ReconcileOutcome::Satisfied)
                }
                // Still live (a process that came up behind the stop, or one
                // the stop left running) or an identity the provider no longer
                // confirms: the stop is not established, so the pass schedules
                // its own re-check and reports the retry - never `Satisfied`
                // with a live process behind it.
                Ok(ProviderLiveness::Alive | ProviderLiveness::Unknown) => {
                    ctx.set_status(ProcessDriverStatus::Stopping);
                    let _ = ctx.requeue_after(PROCESS_RESYNC);
                    Ok(ReconcileOutcome::RetryScheduled)
                }
                Err(error) => Err(map_provider_error(error, DriverOp::Reconcile)),
            };
        }

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
            if self.effects.has_active(
                &identity.zone,
                identity.zone_uid.as_ref(),
                &identity.resource_ref,
            ) {
                self.stop_and_finalize(&identity, spec, DriverOp::Reconcile)
                    .await?;
            }
            ctx.set_status(ProcessDriverStatus::Launching);
            return self.spawn_launch(ctx, identity, spec);
        }

        // Steady state: a row this actor saw live is observed through the
        // liveness probe (old `observe_liveness`), so an exit leaves `Ready`
        // and the restart policy decides what happens next. The self-requeue
        // is the preserved 5s descriptor resync: without it nothing would ever
        // re-enter the pass and the row would report `Ready` over a process
        // that is gone.
        if self.durable.watching() {
            return match self.effects.probe(&identity, spec).await {
                Ok(ProviderLiveness::Alive) => {
                    ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                    let _ = ctx.requeue_after(PROCESS_RESYNC);
                    Ok(ReconcileOutcome::Satisfied)
                }
                Ok(ProviderLiveness::Exited) => self.durable_exit(ctx, &identity, spec),
                // An identity no longer verifies (old `observe_liveness`
                // Unknown): the same terminal `process-identity-ambiguous`
                // refusal the adoption classification reports, so the row
                // publishes `Failed` - a `Satisfied` pass here would publish
                // wire `Ready` over a process this daemon cannot identify.
                // Nothing re-enters the pass, no relaunch happens, and no
                // signal ever reaches the unverifiable candidate.
                Ok(ProviderLiveness::Unknown) => {
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
                Err(error) => Err(map_provider_error(error, DriverOp::Reconcile)),
            };
        }

        match self.effects.adopt(&identity, spec).await {
            Ok(ProviderAdoption::Adopted(_)) => {
                // The live identity is observed from here on: this pass arms
                // the observation cadence and every later pass probes liveness
                // instead of re-adopting.
                self.durable.mark_watching();
                ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                let _ = ctx.requeue_after(PROCESS_RESYNC);
                Ok(ReconcileOutcome::Satisfied)
            }
            Ok(ProviderAdoption::Absent) => {
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_launch(ctx, identity, spec)
            }
            Ok(ProviderAdoption::ControllerBootstrapMissing) => {
                // The Provider owns the exact stop and finalization before the
                // replacement launch (preserved controller-bootstrap effect
                // ordering).
                self.stop_and_finalize(&identity, spec, DriverOp::Reconcile)
                    .await?;
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_launch(ctx, identity, spec)
            }
            Ok(ProviderAdoption::Stale { candidate }) => {
                self.effects
                    .stop_stale(&identity.provider_ref, &candidate)
                    .await
                    .map_err(|error| map_provider_error(error, DriverOp::Reconcile))?;
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_launch(ctx, identity, spec)
            }
            Ok(ProviderAdoption::Quarantined(report)) => {
                Err(self.identity_ambiguous(DriverOp::Reconcile, &report))
            }
            Err(error) => Err(map_provider_error(error, DriverOp::Reconcile)),
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
        if let Some(completion) = self.ephemeral.completed() {
            return self.ephemeral_retention(ctx, spec, completion).await;
        }

        // Runtime deadline (old `ephemeral runtime-deadline` arm): the process
        // this actor started outlived its bounded run, so it stops exactly and
        // the terminal outcome is `Failed`.
        if self.ephemeral.started()
            && let Some(started_at) = self.ephemeral.started_at()
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
            let completion = self.ephemeral.finish(true, "runtime-deadline");
            Self::publish_ephemeral_outcome(ctx, completion);
            ctx.set_status(ProcessDriverStatus::Failed {
                code: "runtime-deadline",
            });
            return self.ephemeral_retention(ctx, spec, completion).await;
        }

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
                    let completion = self.ephemeral.finish(false, "process-exited");
                    Self::publish_ephemeral_outcome(ctx, completion);
                    ctx.set_status(ProcessDriverStatus::Succeeded {
                        code: "process-exited",
                    });
                    self.ephemeral_retention(ctx, spec, completion).await
                }
                Ok(ProviderLiveness::Unknown) => {
                    let completion = self.ephemeral.finish(true, "identity-ambiguous");
                    Self::publish_ephemeral_outcome(ctx, completion);
                    ctx.set_status(ProcessDriverStatus::Failed {
                        code: "identity-ambiguous",
                    });
                    self.ephemeral_retention(ctx, spec, completion).await
                }
                Err(error) => Err(map_provider_error(error, DriverOp::Reconcile)),
            };
        }

        // First sight of the row (first pass, or the first after a daemon
        // restart): the preserved adoption classification decides adopt (an
        // already-live identity), launch (absent), exact stale replacement,
        // or a fail-closed refusal.
        match self.effects.adopt_ephemeral(identity, spec).await {
            Ok(ProviderAdoption::Adopted(_)) => {
                self.ephemeral.mark_started();
                ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                let _ = ctx.requeue_after(PROCESS_RESYNC);
                Ok(ReconcileOutcome::Satisfied)
            }
            Ok(ProviderAdoption::Absent) => {
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_ephemeral_launch(ctx, identity, spec)
            }
            Ok(ProviderAdoption::Stale { candidate }) => {
                self.effects
                    .stop_stale(&identity.provider_ref, &candidate)
                    .await
                    .map_err(|error| map_provider_error(error, DriverOp::Reconcile))?;
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_ephemeral_launch(ctx, identity, spec)
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
    fn spawn_ephemeral_launch(
        &mut self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<ReconcileOutcome, ProcessDriverError> {
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
                    ephemeral.mark_started();
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
        if !execution_target_allowed(self.authority.mode, spec.execution().execution_ref()) {
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
                &envelope.provider_ref.clone().expect("checked"),
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
                if process.adoption_policy() == AdoptionPolicy::NeverAdopt {
                    // NeverAdopt never adopts; an unexpected live identity is stopped
                    // exactly (preserved behavior) and the next launch starts fresh.
                    if self.effects.has_active(
                        &identity.zone,
                        identity.zone_uid.as_ref(),
                        &identity.resource_ref,
                    ) {
                        self.stop_and_finalize(&identity, process, DriverOp::Recover)
                            .await?;
                    }
                    return Ok(RecoveryOutcome::Missing);
                }

                match self.effects.adopt(&identity, process).await {
                    Ok(ProviderAdoption::Adopted(_)) => {
                        // The first reconcile pass right after recovery probes
                        // this identity: mark it so that pass observes
                        // liveness and arms the cadence instead of re-adopting.
                        self.durable.mark_watching();
                        ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                        Ok(RecoveryOutcome::Adopted)
                    }
                    Ok(ProviderAdoption::Absent) => Ok(RecoveryOutcome::Missing),
                    // A static controller without its exact bootstrap endpoint:
                    // nothing to adopt; reconcile restarts it.
                    Ok(ProviderAdoption::ControllerBootstrapMissing) => {
                        Ok(RecoveryOutcome::Missing)
                    }
                    Ok(ProviderAdoption::Stale { .. }) | Ok(ProviderAdoption::Quarantined(_)) => {
                        ctx.set_status(ProcessDriverStatus::Quarantined {
                            code: "identity-ambiguous",
                        });
                        Ok(RecoveryOutcome::Quarantined)
                    }
                    Err(error) => Err(map_provider_error(error, DriverOp::Recover)),
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
                &envelope.provider_ref.clone().expect("checked"),
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
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let Ok((envelope, spec)) = self.decoded_spec(ctx, DriverOp::Delete) else {
            // Nothing launchable to clean up (old behavior: absent record
            // converged).
            return Ok(());
        };
        let identity = match self
            .identity(
                ctx,
                &envelope.provider_ref.clone().expect("checked"),
                DriverOp::Delete,
            )
            .await
        {
            Ok(identity) => identity,
            Err(_) => return Ok(()),
        };

        match &spec {
            ProcessFamilySpec::Ephemeral(ephemeral) => {
                return self.delete_ephemeral(&identity, ephemeral).await;
            }
            ProcessFamilySpec::Process(process) => {
                if process.adoption_policy() == AdoptionPolicy::NeverAdopt {
                    // NeverAdopt never adopts; an unexpected live identity stops
                    // exactly through its retained authority.
                    if self.effects.has_active(
                        &identity.zone,
                        identity.zone_uid.as_ref(),
                        &identity.resource_ref,
                    ) {
                        self.stop_and_finalize(&identity, process, DriverOp::Delete)
                            .await?;
                    }
                    return Ok(());
                }

                match self.effects.adopt(&identity, process).await {
                    Ok(ProviderAdoption::Adopted(_)) => {
                        self.stop_and_finalize(&identity, process, DriverOp::Delete)
                            .await
                    }
                    Ok(ProviderAdoption::Stale { candidate }) => self
                        .effects
                        .stop_stale(&identity.provider_ref, &candidate)
                        .await
                        .map_err(|error| map_provider_error(error, DriverOp::Delete)),
                    Ok(ProviderAdoption::Absent)
                    | Ok(ProviderAdoption::ControllerBootstrapMissing) => {
                        // Nothing this daemon can stop exactly (old deletion treated
                        // a missing exact identity as converged without effects).
                        Ok(())
                    }
                    Ok(ProviderAdoption::Quarantined(report)) => {
                        Err(self.identity_ambiguous(DriverOp::Delete, &report))
                    }
                    Err(error) => Err(map_provider_error(error, DriverOp::Delete)),
                }
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
    use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceUid, ZoneId};
    use d2b_process_conformance::testing::fixtures;
    use d2b_process_conformance::{
        AdoptionCandidate, AdoptionCondition, IdentityBinding, ObservedIdentity,
        ProcessIdentityDigest, ProcessPhaseClass, ProcessStatusReport, WaitReapOwner,
    };
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext,
        WatchRegistration,
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
    use d2b_resource_runtime::target::TargetHandle;
    use parking_lot::Mutex;
    use tokio::sync::mpsc;

    use super::{
        AllowedSources, EPHEMERAL_PROCESS_TYPE_NAME, PROCESS_TYPE_NAME, ProcessDriver,
        ProcessDriverArgs, ProcessDriverErrorKind, ProcessDriverFactory, ProcessDriverStatus,
        process_family_descriptors, process_spec_decoder,
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
            Err(ResourceError::ManagerRpc("dead".into()))
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerRpc("dead".into()))
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            Err(ResourceError::ManagerRpc("dead".into()))
        }

        async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerRpc("dead".into()))
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Err(ResourceError::ManagerRpc("dead".into()))
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<d2b_resource_runtime::context::WatchId, ResourceError> {
            Err(ResourceError::ManagerRpc("dead".into()))
        }

        async fn cancel_watch(
            &self,
            _watch: d2b_resource_runtime::context::WatchId,
        ) -> Result<(), ResourceError> {
            Err(ResourceError::ManagerRpc("dead".into()))
        }
    }

    /// Owner-scoped manager double for the finalize gate: one scripted owned
    /// row set; `delete` records the retirement nudge and removes the row.
    struct OwnershipManager {
        owned: parking_lot::Mutex<Vec<StoredDesiredResource>>,
        rows: parking_lot::Mutex<Vec<StoredDesiredResource>>,
        deleted: parking_lot::Mutex<Vec<ResourceKey>>,
    }

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
    impl ManagerEndpoint for OwnershipManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            Err(ResourceError::ManagerRpc("unexpected ensure_child".into()))
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self.rows.lock().iter().find(|row| row.key == *key).cloned())
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            Err(ResourceError::ManagerRpc("unexpected view".into()))
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.deleted.lock().push(key.clone());
            self.owned.lock().retain(|row| row.key != *key);
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
            Err(ResourceError::ManagerRpc(
                "unexpected register_watch".into(),
            ))
        }

        async fn cancel_watch(
            &self,
            _watch: d2b_resource_runtime::context::WatchId,
        ) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    /// Recording requeue scheduler over tokio paused time: every schedule is
    /// recorded with its exact delay and delivers one id after the backoff.
    #[derive(Clone)]
    struct RecordingRequeue {
        inner: Arc<Mutex<RecordingRequeueInner>>,
    }

    struct RecordingRequeueInner {
        calls: Vec<(ResourceKey, Duration)>,
        next: u64,
        delivered_tx: Option<mpsc::UnboundedSender<u64>>,
    }

    impl RecordingRequeue {
        fn new() -> (Self, mpsc::UnboundedReceiver<u64>) {
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

        fn recorded(&self) -> Vec<(ResourceKey, Duration)> {
            self.inner.lock().calls.clone()
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
            TargetHandle::Host,
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

    impl Fixture {
        fn requeue_calls(&self) -> Vec<Duration> {
            self.requeue
                .recorded()
                .into_iter()
                .map(|(_, after)| after)
                .collect()
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
        fn backdate_runtime(&self, elapsed: Duration) {
            self.typed.ephemeral.backdate_started(elapsed);
        }

        /// Test-only: backdate the one-shot retention clock.
        fn backdate_completion(&self, elapsed: Duration) {
            self.typed.ephemeral.backdate_completed(elapsed);
        }

        fn ephemeral_completed(&self) -> bool {
            self.typed.ephemeral.completed().is_some()
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
                "mountPath": "/mnt/data",
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
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
            source.consulted.load(std::sync::atomic::Ordering::SeqCst),
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
            source.consulted.load(std::sync::atomic::Ordering::SeqCst),
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
            source.consulted.load(std::sync::atomic::Ordering::SeqCst),
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
            manager.deleted.lock().is_empty(),
            "the retention window has not elapsed"
        );

        // TTL elapsed: the driver asks the manager to retire its own row.
        driver.backdate_completion(Duration::from_secs(3600));
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(manager.deleted.lock().clone(), vec![f.row.key.clone()]);
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

        driver.backdate_runtime(Duration::from_secs(300));
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

        driver.backdate_runtime(Duration::from_secs(300));
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
            driver.ephemeral_completed(),
            "the terminal state is recorded"
        );

        driver.backdate_completion(Duration::from_secs(365 * 24 * 3600));
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert!(
            manager.deleted.lock().is_empty(),
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
            manager.deleted.lock().len(),
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

    /// The terminal set is the closed unresolvable-ticket spellings, not every
    /// launch error: a genuine provider-effect refusal - the identity the
    /// ticket path could not bind yet is the case the seeding exists for -
    /// still drains the restart budget and retries at the policy backoff.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn durable_provider_effect_launch_failure_still_retries_under_the_budget() {
        let fake = Arc::new(FakeFacets::new(FakeFacetsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            launch: Err("provider-controller-provider-identity-missing".to_owned()),
            ..FakeFacetsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        let completed = f.effects.recv().await.expect("completion");
        assert!(matches!(
            completed.result,
            d2b_resource_runtime::context::EffectResult::Failed(failure)
                if failure.class() == FailureClass::Retryable
        ));
        assert_eq!(driver.restart_count(), 1);

        // The next pass schedules exactly one policy-backoff requeue, and it
        // reports the scheduled retry (never Ready: no process exists yet).
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::RetryScheduled
        );
        assert_eq!(f.requeue_calls(), [Duration::from_secs(1)]);
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
}
