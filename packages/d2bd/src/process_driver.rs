//! Process resource driver (U6): the v3 `ResourceDriver` conversion of the
//! daemon-owned generic Process path (R3, R4, R15, R16; KTD7, KTD12).
//!
//! The driver keeps the old `ProcessResourceReconciler` behavior and nothing
//! else: recover probes and classifies adopt/missing/quarantine per the
//! preserved `ProviderAdoption` classification (same Adopt/Quarantine/Missing
//! shape as the `d2bd-runtime` supervisor precedent), reconcile launches
//! through the preserved signed provider-ticket path (the ticket machinery
//! stays inside `ProductionProcessProviders`), and delete reuses the exact
//! term-then-kill escalation with pidfd retry.
//!
//! Ticket inputs (KTD7) come from the factory's zone-authority wiring - the
//! bundle resolver and `ZoneAuthorityIdentity` path - never from the spec
//! store. The restart budget is runtime-only (spec section 32): the old
//! persisted restart-generation annotation is deliberately not ported.
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`ProcessDriverFactory`] registration under `Process`.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`] plus reconcile probing.
//! - `prepare`/`execute`/`finalize` -> [`ResourceDriver::delete`].
//! - `RequeueAt` -> `ctx.requeue_after` (runtime-only, R13).
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11).
// U9 wires this factory into the composition; until the cutover the module
// surface is only exercised from its tests.
#![allow(dead_code)]
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

use d2b_contracts_resource::v3::{
    AdoptionPolicy, ControllerGeneration, ResourceGeneration, ResourceName, ResourceRef,
    ResourceSpec, ResourceTypeName as ContractResourceTypeName, ResourceUid, ZoneId, ZoneRevision,
    process::{DesiredLifecycle, ProcessClass, ProcessSpec, RestartClass},
};
use d2b_process_conformance::{AdoptionCandidate, GuestExecutionBinding, ProcessIdentityDigest};
use d2b_resource_runtime::context::{
    EffectCompleted, EffectResult, ResourceContext, SpecDecoder, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2bd_runtime::target_runtime::DaemonMode;

use crate::process_provider_runtime::{
    ProcessResourceContext, ProductionProcessProviders, ProviderAdoption, execution_target_allowed,
};

/// The one resource type this factory serves (KTD4 Phase A). `EphemeralProcess`
/// stays on the old reconciler until its conversion unit.
pub(crate) const PROCESS_TYPE_NAME: &str = "Process";

const MINIJAIL_PROVIDER: &str = "system-minijail";
const SYSTEMD_PROVIDER: &str = "system-systemd";

/// Preserved launch budget for durable Process resources (old
/// `launch_timeout`).
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Preserved kill budget after the drain timeout elapses (old stop
/// escalation).
const KILL_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessDriverErrorKind {
    /// The durable spec did not decode as the closed Process contract.
    SpecInvalid,
    /// The spec selected a Provider this driver does not own.
    ProviderUnsupported,
    /// The spec's execution target is not drivable in this daemon mode.
    ExecutionUnsupported,
    /// The trusted bundle did not contain the requested template binding.
    TemplateUnavailable,
    /// A process identity was ambiguous; quarantine per policy (R15).
    IdentityAmbiguous,
    /// A Provider effect failed transiently.
    ProviderEffect,
    /// The in-memory restart budget is exhausted (spec section 32).
    StartExhausted,
}

impl ProcessDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::ProviderEffect => FailureClass::Retryable,
            Self::SpecInvalid
            | Self::ProviderUnsupported
            | Self::ExecutionUnsupported
            | Self::TemplateUnavailable
            | Self::IdentityAmbiguous
            | Self::StartExhausted => FailureClass::Terminal,
        }
    }
}

/// Typed driver failure; redacted at the erased boundary through
/// [`ResourceDriver::classify_error`] (R13).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProcessDriverError {
    kind: ProcessDriverErrorKind,
    op: DriverOp,
}

impl ProcessDriverError {
    fn new(kind: ProcessDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }
}

impl core::fmt::Display for ProcessDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            ProcessDriverErrorKind::SpecInvalid => "process-spec-invalid",
            ProcessDriverErrorKind::ProviderUnsupported => "process-provider-unsupported",
            ProcessDriverErrorKind::ExecutionUnsupported => "process-execution-unsupported",
            ProcessDriverErrorKind::TemplateUnavailable => "process-template-unavailable",
            ProcessDriverErrorKind::IdentityAmbiguous => "process-identity-ambiguous",
            ProcessDriverErrorKind::ProviderEffect => "process-provider-effect-failed",
            ProcessDriverErrorKind::StartExhausted => "process-start-budget-exhausted",
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
    /// The exact process is live; `adopted` records whether the Provider
    /// adopted it instead of launching it.
    Ready { adopted: bool },
    /// A restart backoff timer is scheduled (runtime-only requeue).
    AwaitingRestart { restart_count: u32 },
    /// The process reached a state that satisfies its desired lifecycle.
    Succeeded { code: &'static str },
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

/// The manager-wired decode hook for Process rows.
pub(crate) fn process_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| ProcessSpecEnvelope {
            raw: bytes.to_vec(),
            provider_ref: spec.provider_ref().cloned(),
            base: spec.base().clone(),
        })
        .map_err(|error| SpecDecodeFailure { reason: error.to_string() })
    })
}

// ---------------------------------------------------------------------------
// Adoption / launch identity (KTD7)
// ---------------------------------------------------------------------------

/// The identity inputs every provider ticket is derived from: the durable
/// adoption identity (zone, type, name, uid, generation) plus the
/// zone-authority ticket inputs from the bundle resolver /
/// `ZoneAuthorityIdentity` path (KTD7). Nothing is read from or written to the
/// spec store at runtime.
#[derive(Debug, Clone)]
pub(crate) struct ProcessResourceIdentity {
    pub(crate) zone: ZoneId,
    pub(crate) resource_ref: ResourceRef,
    pub(crate) resource_uid: ResourceUid,
    pub(crate) resource_generation: ResourceGeneration,
    /// Spec `processClass` (decoded from the same durable row). Only
    /// controller rows take the committed controller-provider identity, and
    /// the effects cannot re-read the spec at finalize time (KTD7).
    pub(crate) process_class: ProcessClass,
    pub(crate) provider_ref: ResourceRef,
    /// Semantic owner (binding workers, static controllers, guest VMM
    /// processes); the Phase A host driver leaves owner/target wiring to the
    /// composition unit, which owns owner-key resolution.
    pub(crate) owner_ref: Option<ResourceRef>,
    pub(crate) owner_uid: Option<ResourceUid>,
    pub(crate) target_ref: Option<ResourceRef>,
    pub(crate) zone_uid: Option<ResourceUid>,
    pub(crate) policy_revision: Option<u64>,
    pub(crate) provider_assignment_generation: Option<ResourceGeneration>,
    pub(crate) controller_generation: ControllerGeneration,
    pub(crate) controller_provider_uid: Option<ResourceUid>,
    pub(crate) controller_provider_generation: Option<ResourceGeneration>,
    pub(crate) guest_execution: Option<GuestExecutionBinding>,
    /// Binding-declared serving-worker launch inputs (VolumeBinding-owned
    /// virtiofsd workers only).
    pub(crate) worker_launch: Option<crate::process_provider_runtime::ServingWorkerLaunch>,
}

impl ProcessResourceIdentity {
    /// Build the borrowed provider-layer context. The ticket machinery is
    /// entirely inside the provider layer; the driver never assembles a
    /// ticket.
    fn resource_context(&self) -> ProcessResourceContext<'_> {
        ProcessResourceContext::new(
            self.zone.clone(),
            &self.resource_ref,
            &self.resource_uid,
            self.resource_generation,
            // The new store has no zone-wide commit revision: the durable
            // revision of a row is its generation. The launch ticket requires
            // a non-zero resource revision, and the provider identity fence
            // compares generations, not revisions, so the row generation is
            // the honest binding here.
            ZoneRevision::new(self.resource_generation.get()),
            &self.provider_ref,
            self.controller_generation,
            self.target_ref.clone(),
        )
        .with_guest_execution(self.guest_execution.as_ref())
        .with_lifecycle_identity(
            self.zone_uid.clone(),
            self.policy_revision,
            self.provider_assignment_generation,
        )
        .with_owner_ref(self.owner_ref.clone())
        .with_owner_uid(self.owner_uid.clone())
        .with_provider_identity(
            self.controller_provider_uid.as_ref(),
            self.controller_provider_generation,
        )
        .with_worker_launch(self.worker_launch.clone())
    }
}

/// Map the new store's 16-byte deterministic uid onto the contracts crate's
/// UUIDv4-shaped `ResourceUid` (version nibble 4, RFC 9562 variant).
fn resource_uid_from_bytes(bytes: &[u8; 16]) -> Result<ResourceUid, ()> {
    let mut bytes = *bytes;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).map_err(|_| ())
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The provider-facing effect surface the Process driver needs. The production
/// implementation delegates to the already-composed
/// [`ProductionProcessProviders`]; test doubles implement the same seam
/// (R4: the conversion is mechanical, the provider effects are preserved).
///
/// Object-erased on purpose: the driver holds the port as
/// `Arc<dyn ProcessDriverEffects>` so one factory serves every Process row.
#[async_trait::async_trait]
pub(crate) trait ProcessDriverEffects: Send + Sync + 'static {
    /// Launch through the signed provider-ticket path.
    async fn launch(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String>;

    /// Probe-and-adopt over pidfd/proc evidence with the preserved
    /// Adopt/Stale/Quarantined classification.
    async fn adopt(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String>;

    /// Preserved term-then-kill escalation with pidfd retry; `Ok(killed)`
    /// reports whether the kill stage ran.
    async fn stop(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String>;

    /// Stop one exactly-identified stale candidate before a fresh launch.
    async fn stop_stale(
        &self,
        provider_ref: &ResourceRef,
        candidate: &AdoptionCandidate,
    ) -> Result<(), String>;

    /// Remove the provider's exact local authority after a terminal exit.
    async fn finalize(&self, identity: &ProcessResourceIdentity) -> Result<(), String>;

    /// Whether this zone retains a verified identity for the resource.
    fn has_active(
        &self,
        zone: &ZoneId,
        zone_uid: Option<&ResourceUid>,
        resource_ref: &ResourceRef,
    ) -> bool;
}

/// KTD7 committed Provider identity source: the committed `Provider` rows
/// (uid + generation) the per-zone plane publishes, resolved from the old
/// plane's durable authority (the new store carries no `Provider` rows). The
/// Process layer never reads a store itself (KTD7); a reference this source
/// does not retain stays unbound, so the provider ticket path refuses the
/// controller launch closed (`provider-controller-provider-identity-missing`).
pub(crate) trait CommittedProviderIdentitySource: Send + Sync + 'static {
    /// The committed row's identity for one `Provider` reference.
    fn committed_provider_identity(
        &self,
        provider_ref: &ResourceRef,
    ) -> Option<(ResourceUid, ResourceGeneration)>;
}

/// Production effects over the composed fixed Providers.
pub(crate) struct ProductionProcessDriverEffects {
    providers: Arc<ProductionProcessProviders>,
    /// Committed Provider identities (KTD7), wired by the plane's
    /// construction path from the composition-resolved snapshot.
    committed_provider_identities: Option<Arc<dyn CommittedProviderIdentitySource>>,
}

impl ProductionProcessDriverEffects {
    pub(crate) fn new(providers: Arc<ProductionProcessProviders>) -> Self {
        Self {
            providers,
            committed_provider_identities: None,
        }
    }

    /// Attach the committed Provider identity source (KTD7). Unwired effects
    /// keep the driver-derived (unbound) identity, which the provider ticket
    /// path refuses closed.
    pub(crate) fn with_committed_provider_identities(
        mut self,
        source: Arc<dyn CommittedProviderIdentitySource>,
    ) -> Self {
        self.committed_provider_identities = Some(source);
        self
    }

    /// The provider-layer context for one row, with the committed
    /// controller-provider identity bound (KTD7).
    fn resource_context<'a>(
        &self,
        identity: &'a ProcessResourceIdentity,
    ) -> ProcessResourceContext<'a> {
        bind_committed_controller_provider_identity(
            identity,
            self.committed_provider_identities.as_deref(),
        )
    }
}

/// Bind the committed Provider row's identity (KTD7) onto one controller
/// row's provider context: a controller Process owned by a `Provider` takes
/// that Provider's committed uid/generation when the driver left the identity
/// unbound. Every other row - another process class, another owner type, an
/// already-bound identity, or a Provider with no committed row - keeps the
/// driver-derived context, so a genuinely missing row still refuses closed.
fn bind_committed_controller_provider_identity<'a>(
    identity: &'a ProcessResourceIdentity,
    source: Option<&dyn CommittedProviderIdentitySource>,
) -> ProcessResourceContext<'a> {
    let context = identity.resource_context();
    if identity.process_class != ProcessClass::Controller
        || identity.controller_provider_uid.is_some()
        || identity.controller_provider_generation.is_some()
    {
        return context;
    }
    let Some(provider_owner) = identity
        .owner_ref
        .as_ref()
        .filter(|owner| owner.resource_type().as_str() == "Provider")
    else {
        return context;
    };
    match source.and_then(|source| source.committed_provider_identity(provider_owner)) {
        Some((uid, generation)) => context.with_provider_identity(Some(&uid), Some(generation)),
        None => context,
    }
}

#[async_trait::async_trait]
impl ProcessDriverEffects for ProductionProcessDriverEffects {
    async fn launch(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String> {
        let context = self.resource_context(identity);
        self.providers
            .launch_resource(context, spec, timeout)
            .await
            .map(|launch| launch.identity)
    }

    async fn adopt(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.providers
            .adopt_resource(self.resource_context(identity), spec)
            .await
    }

    async fn stop(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.providers
            .stop_resource(
                self.resource_context(identity),
                spec,
                term_timeout,
                kill_timeout,
            )
            .await
    }

    async fn stop_stale(
        &self,
        provider_ref: &ResourceRef,
        candidate: &AdoptionCandidate,
    ) -> Result<(), String> {
        self.providers
            .stop_stale_resource(provider_ref, candidate)
            .await
    }

    async fn finalize(&self, identity: &ProcessResourceIdentity) -> Result<(), String> {
        self.providers
            .finalize_resource(self.resource_context(identity))
            .await
    }

    fn has_active(
        &self,
        zone: &ZoneId,
        zone_uid: Option<&ResourceUid>,
        resource_ref: &ResourceRef,
    ) -> bool {
        self.providers
            .has_active_resource_in_zone(zone, zone_uid, resource_ref)
    }
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

    /// Record one consumed restart; the next reconcile pass schedules the
    /// policy backoff exactly once.
    fn record_restart(&self) {
        self.count.fetch_add(1, Ordering::SeqCst);
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

/// Everything the composition unit (U9) must construct to instantiate the
/// Process driver factory for one zone: the composed provider effects plus
/// the KTD7 zone-authority inputs.
pub(crate) struct ProcessDriverArgs {
    pub(crate) zone: ZoneId,
    pub(crate) effects: Arc<dyn ProcessDriverEffects>,
    /// Zone authority uid (bundle resolver / ZoneAuthorityIdentity path).
    pub(crate) zone_uid: Option<ResourceUid>,
    /// Zone policy revision from the authority path.
    pub(crate) policy_revision: Option<u64>,
    /// Provider assignment generation (guest execution sessions).
    pub(crate) provider_assignment_generation: Option<ResourceGeneration>,
    pub(crate) controller_generation: ControllerGeneration,
    pub(crate) guest_execution: Option<GuestExecutionBinding>,
    pub(crate) mode: DaemonMode,
}

/// [`ResourceDriverFactory`] for the `Process` resource type. Construction is
/// infallible by contract: resource-specific failures surface through the
/// driver's validate/recover where the actor owns retry policy (R3).
pub(crate) struct ProcessDriverFactory {
    types: [ResourceTypeName; 1],
    args: ProcessDriverArgs,
}

impl ProcessDriverFactory {
    pub(crate) fn new(args: ProcessDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(PROCESS_TYPE_NAME)],
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
            effects,
            zone_uid,
            policy_revision,
            provider_assignment_generation,
            controller_generation,
            guest_execution,
            mode,
        } = &self.args;
        Box::new(ProcessDriver::new(ProcessDriverArgs {
            zone: zone.clone(),
            effects: Arc::clone(effects),
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
// Driver
// ---------------------------------------------------------------------------

/// One Process resource's driver. Effects run through the injected
/// [`ProcessDriverEffects`] port; the actor owns scheduling, retries, and
/// status publication.
#[derive(Clone)]
pub(crate) struct ProcessDriver {
    zone: ZoneId,
    authority: ProcessZoneAuthority,
    effects: Arc<dyn ProcessDriverEffects>,
    budget: Arc<RestartBudget>,
}

/// Zone-authority inputs folded into every derived identity (KTD7).
#[derive(Clone)]
struct ProcessZoneAuthority {
    zone_uid: Option<ResourceUid>,
    policy_revision: Option<u64>,
    provider_assignment_generation: Option<ResourceGeneration>,
    controller_generation: ControllerGeneration,
    guest_execution: Option<GuestExecutionBinding>,
    mode: DaemonMode,
}

impl ProcessDriver {
    pub(crate) fn new(args: ProcessDriverArgs) -> Self {
        let ProcessDriverArgs {
            zone,
            effects,
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
            effects,
            budget: Arc::new(RestartBudget::default()),
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

    /// Decode the stored envelope and the typed spec in one step.
    fn decoded_spec(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<(ProcessSpecEnvelope, ProcessSpec), ProcessDriverError> {
        let envelope = ctx
            .spec::<ProcessSpecEnvelope>()
            .map_err(|_| self.error(ProcessDriverErrorKind::SpecInvalid, op))?;
        let spec = serde_json::from_slice::<ProcessSpec>(&envelope.base.to_canonical_bytes())
            .map_err(|_| self.error(ProcessDriverErrorKind::SpecInvalid, op))?;
        Ok((envelope.clone(), spec))
    }

    /// Provider reference checks (old `desired()`): a Process spec must select
    /// one of the daemon-owned fixed Providers.
    fn check_provider(
        &self,
        envelope: &ProcessSpecEnvelope,
        op: DriverOp,
    ) -> Result<(), ProcessDriverError> {
        let Some(provider_ref) = envelope.provider_ref.as_ref() else {
            return Err(self.error(ProcessDriverErrorKind::SpecInvalid, op));
        };
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(self.error(ProcessDriverErrorKind::SpecInvalid, op));
        }
        if !matches!(
            provider_ref.name().as_str(),
            MINIJAIL_PROVIDER | SYSTEMD_PROVIDER
        ) {
            return Err(self.error(ProcessDriverErrorKind::ProviderUnsupported, op));
        }
        Ok(())
    }

    /// Re-derive the adoption/launch identity (KTD7) from the durable row plus
    /// the zone-authority inputs. Target-local evidence (pidfd, /proc
    /// starttime, socket path) stays behind the provider effect port.
    ///
    /// A binding-owned virtiofsd worker executes on the host (the signed
    /// `virtiofsd-worker` template binds the Host execution reference) and is
    /// targeted at the attachment's Guest through the ticket's target ref
    /// (KTD7) - the host-exec/guest-target split the old binding runner
    /// preserved. The authoritative target input is the owning VolumeBinding
    /// row's declared execution ref; every other owner keeps an unbound
    /// target.
    async fn identity(
        &self,
        ctx: &mut ResourceContext,
        provider_ref: &ResourceRef,
        op: DriverOp,
    ) -> Result<ProcessResourceIdentity, ProcessDriverError> {
        let key = ctx.key();
        let resource_type = ContractResourceTypeName::parse(&key.type_name)
            .map_err(|_| self.error(ProcessDriverErrorKind::SpecInvalid, op))?;
        let name = ResourceName::parse(&key.name)
            .map_err(|_| self.error(ProcessDriverErrorKind::SpecInvalid, op))?;
        let zone = ZoneId::parse(&key.zone)
            .map_err(|_| self.error(ProcessDriverErrorKind::SpecInvalid, op))?;
        let resource_uid = resource_uid_from_bytes(ctx.uid())
            .map_err(|_| self.error(ProcessDriverErrorKind::SpecInvalid, op))?;
        let resource_generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(ProcessDriverErrorKind::SpecInvalid, op))?;
        // The row's process class rides the identity: the production effects
        // bind the committed controller-provider identity for controller rows
        // and have no spec to consult when they finalize (KTD7).
        let (_, spec) = self.decoded_spec(ctx, op)?;
        let process_class = spec.execution().process_class();
        let mut worker_launch = None;
        let target_ref = match ctx.owner_key().cloned() {
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
                        worker_launch = self
                            .serving_worker_launch(ctx, binding, op)
                            .await;
                    }
                    binding.map(|binding| binding.execution_ref().clone())
                }
                _ => None,
            },
            _ => None,
        };
        Ok(ProcessResourceIdentity {
            zone,
            resource_ref: ResourceRef::new(resource_type, name),
            resource_uid,
            resource_generation,
            process_class,
            provider_ref: provider_ref.clone(),
            owner_ref: ctx
                .owner_key()
                .and_then(|owner| ResourceRef::parse(&format!("{}/{}", owner.type_name, owner.name)).ok())
                // The manager resolves an owner key only for owners that are
                // its own rows; an owned child of an unconverted owner falls
                // back to the authored reference the row was ingested with.
                .or_else(|| crate::resource_plane_v3::decode_metadata_owner_ref(ctx.metadata())),
            owner_uid: ctx.owner().and_then(resource_uid_from_bytes_bytes),
            target_ref,
            zone_uid: self.authority.zone_uid.clone(),
            policy_revision: self.authority.policy_revision,
            provider_assignment_generation: self.authority.provider_assignment_generation,
            controller_generation: self.authority.controller_generation,
            controller_provider_uid: None,
            controller_provider_generation: None,
            guest_execution: self.authority.guest_execution.clone(),
            worker_launch,
        })
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
    ) -> Option<crate::process_provider_runtime::ServingWorkerLaunch> {
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
        let storage_path_id = match source.settings().kind() {
            d2b_contracts_resource::v3::volume::SourceKind::LocalPath => {
                let policy = source.settings().source_policy_id()?.as_str().to_owned();
                Some(if policy == "state-root" || policy == "default-state" {
                    "path:state-root".to_owned()
                } else {
                    format!("path:{policy}")
                })
            }
            _ => None,
        };
        Some(crate::process_provider_runtime::ServingWorkerLaunch {
            volume_ref: binding.volume_ref().clone(),
            view: binding.view().clone(),
            guest_ref: binding.execution_ref().clone(),
            storage_path_id,
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
    /// and the next pass applies the policy backoff.
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
        tokio::spawn(async move {
            let effect_result = match effects.launch(&identity, &task_spec, LAUNCH_TIMEOUT).await {
                Ok(_) => EffectResult::Completed,
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
                    if budget.allows(&task_spec) {
                        budget.record_restart();
                        EffectResult::Failed(DriverFailure::retryable(DriverOp::Reconcile))
                    } else {
                        budget.mark_exhausted();
                        EffectResult::Failed(DriverFailure::terminal(DriverOp::Reconcile))
                    }
                }
            };
            let _ = effect_sender.send(EffectCompleted { operation, result: effect_result });
        });
        Ok(ReconcileOutcome::InProgress { operation })
    }
}

fn resource_uid_from_bytes_bytes(bytes: &[u8; 16]) -> Option<d2b_contracts_resource::v3::ResourceUid> {
    resource_uid_from_bytes(bytes).ok()
}

/// Map preserved provider error spellings onto the closed driver kinds (the
/// same classification as the old `map_provider_error`).
fn map_provider_error(error: String, op: DriverOp) -> ProcessDriverError {
    tracing::warn!(operation = ?op, error = %error, "process provider effect failed");
    let kind = if error.contains("template-not-found") {
        ProcessDriverErrorKind::TemplateUnavailable
    } else if error.contains("quarantined")
        || error.contains("identity")
        || error.contains("ambiguous")
    {
        ProcessDriverErrorKind::IdentityAmbiguous
    } else {
        ProcessDriverErrorKind::ProviderEffect
    };
    ProcessDriverError::new(kind, op)
}

#[async_trait::async_trait]
impl ResourceDriver for ProcessDriver {
    type Error = ProcessDriverError;

    fn classify_error(&self, error: &ProcessDriverError) -> DriverFailure {
        match error.kind.class() {
            FailureClass::Retryable => DriverFailure::retryable(error.op),
            FailureClass::Terminal => DriverFailure::terminal(error.op),
        }
    }

    /// Spec decode plus provider reference and execution-target checks.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let (envelope, spec) = self.decoded_spec(ctx, DriverOp::Validate)?;
        self.check_provider(&envelope, DriverOp::Validate)?;
        if !execution_target_allowed(self.authority.mode, spec.execution().execution_ref()) {
            return Err(self.error(
                ProcessDriverErrorKind::ExecutionUnsupported,
                DriverOp::Validate,
            ));
        }
        Ok(())
    }

    /// Probe and classify on the realization target (R15, R16): exact match
    /// adopts, missing waits for reconcile, drifted/ambiguous quarantines.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let (envelope, spec) = self.decoded_spec(ctx, DriverOp::Recover)?;
        self.check_provider(&envelope, DriverOp::Recover)?;
        let identity = self.identity(ctx, &envelope.provider_ref.clone().expect("checked"), DriverOp::Recover).await?;

        if spec.desired_lifecycle() == DesiredLifecycle::Stopped {
            ctx.set_status(ProcessDriverStatus::Succeeded { code: "process-stopped" });
            return Ok(RecoveryOutcome::Missing);
        }
        if spec.adoption_policy() == AdoptionPolicy::NeverAdopt {
            // NeverAdopt never adopts; an unexpected live identity is stopped
            // exactly (preserved behavior) and the next launch starts fresh.
            if self
                .effects
                .has_active(&identity.zone, identity.zone_uid.as_ref(), &identity.resource_ref)
            {
                self.stop_and_finalize(&identity, &spec, DriverOp::Recover).await?;
            }
            return Ok(RecoveryOutcome::Missing);
        }

        match self.effects.adopt(&identity, &spec).await {
            Ok(ProviderAdoption::Adopted(_)) => {
                ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                Ok(RecoveryOutcome::Adopted)
            }
            Ok(ProviderAdoption::Absent) => Ok(RecoveryOutcome::Missing),
            // A static controller without its exact bootstrap endpoint:
            // nothing to adopt; reconcile restarts it.
            Ok(ProviderAdoption::ControllerBootstrapMissing) => Ok(RecoveryOutcome::Missing),
            Ok(ProviderAdoption::Stale { .. }) | Ok(ProviderAdoption::Quarantined(_)) => {
                ctx.set_status(ProcessDriverStatus::Quarantined { code: "identity-ambiguous" });
                Ok(RecoveryOutcome::Quarantined)
            }
            Err(error) => Err(map_provider_error(error, DriverOp::Recover)),
        }
    }

    /// One reconcile pass: probe, then adopt/launch/stop-stale per the
    /// preserved classification. Launches spawn as long effects (R5).
    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
        let (envelope, spec) = self.decoded_spec(ctx, DriverOp::Reconcile)?;
        self.check_provider(&envelope, DriverOp::Reconcile)?;
        let identity = self.identity(ctx, &envelope.provider_ref.clone().expect("checked"), DriverOp::Reconcile).await?;
        if spec.desired_lifecycle() == DesiredLifecycle::Stopped {
            ctx.set_status(ProcessDriverStatus::Succeeded { code: "process-stopped" });
            return Ok(ReconcileOutcome::Satisfied);
        }

        // A retryable launch failure from the previous pass: schedule exactly
        // one runtime-only requeue with the policy restart delay (R13; spec
        // section 32 - nothing is persisted).
        if self.budget.take_restart_scheduled() {
            let restart_count = self.budget.count();
            let delay = restart_delay(&spec, restart_count);
            let _ = ctx.requeue_after(delay);
            ctx.set_status(ProcessDriverStatus::AwaitingRestart { restart_count });
            return Ok(ReconcileOutcome::Satisfied);
        }

        if spec.adoption_policy() == AdoptionPolicy::NeverAdopt {
            if self
                .effects
                .has_active(&identity.zone, identity.zone_uid.as_ref(), &identity.resource_ref)
            {
                self.stop_and_finalize(&identity, &spec, DriverOp::Reconcile).await?;
            }
            ctx.set_status(ProcessDriverStatus::Launching);
            return self.spawn_launch(ctx, identity, &spec);
        }

        match self.effects.adopt(&identity, &spec).await {
            Ok(ProviderAdoption::Adopted(_)) => {
                ctx.set_status(ProcessDriverStatus::Ready { adopted: true });
                Ok(ReconcileOutcome::Satisfied)
            }
            Ok(ProviderAdoption::Absent) => {
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_launch(ctx, identity, &spec)
            }
            Ok(ProviderAdoption::ControllerBootstrapMissing) => {
                // The Provider owns the exact stop and finalization before the
                // replacement launch (preserved controller-bootstrap effect
                // ordering).
                self.stop_and_finalize(&identity, &spec, DriverOp::Reconcile).await?;
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_launch(ctx, identity, &spec)
            }
            Ok(ProviderAdoption::Stale { candidate }) => {
                self.effects
                    .stop_stale(&identity.provider_ref, &candidate)
                    .await
                    .map_err(|error| map_provider_error(error, DriverOp::Reconcile))?;
                ctx.set_status(ProcessDriverStatus::Launching);
                self.spawn_launch(ctx, identity, &spec)
            }
            Ok(ProviderAdoption::Quarantined(_)) => Err(self.error(
                ProcessDriverErrorKind::IdentityAmbiguous,
                DriverOp::Reconcile,
            )),
            Err(error) => Err(map_provider_error(error, DriverOp::Reconcile)),
        }
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
        let identity = match self.identity(ctx, &envelope.provider_ref.clone().expect("checked"), DriverOp::Delete).await {
            Ok(identity) => identity,
            Err(_) => return Ok(()),
        };

        if spec.adoption_policy() == AdoptionPolicy::NeverAdopt {
            // NeverAdopt never adopts; an unexpected live identity stops
            // exactly through its retained authority.
            if self
                .effects
                .has_active(&identity.zone, identity.zone_uid.as_ref(), &identity.resource_ref)
            {
                self.stop_and_finalize(&identity, &spec, DriverOp::Delete).await?;
            }
            return Ok(());
        }

        match self.effects.adopt(&identity, &spec).await {
            Ok(ProviderAdoption::Adopted(_)) => {
                self.stop_and_finalize(&identity, &spec, DriverOp::Delete).await
            }
            Ok(ProviderAdoption::Stale { candidate }) => {
                self.effects
                    .stop_stale(&identity.provider_ref, &candidate)
                    .await
                    .map_err(|error| map_provider_error(error, DriverOp::Delete))
            }
            Ok(ProviderAdoption::Absent) | Ok(ProviderAdoption::ControllerBootstrapMissing) => {
                // Nothing this daemon can stop exactly (old deletion treated
                // a missing exact identity as converged without effects).
                Ok(())
            }
            Ok(ProviderAdoption::Quarantined(_)) => Err(self.error(
                ProcessDriverErrorKind::IdentityAmbiguous,
                DriverOp::Delete,
            )),
            Err(error) => Err(map_provider_error(error, DriverOp::Delete)),
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

    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::{
        ControllerGeneration, ProcessSpec, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
        process::ProcessClass,
    };
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
    use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;
    use d2bd_runtime::target_runtime::DaemonMode;
    use parking_lot::Mutex;
    use tokio::sync::mpsc;

    use super::{
        ProcessDriver, ProcessDriverArgs, ProcessDriverFactory, ProcessDriverStatus,
        process_spec_decoder,
    };
    use crate::process_provider_runtime::ProviderAdoption;

    // -- fake effect port ----------------------------------------------------

    /// One recorded launch with the ticket inputs the driver derived.
    #[derive(Clone, Debug)]
    struct RecordedLaunch {
        resource_ref: String,
        resource_uid: String,
        generation: u64,
        zone: ZoneId,
        zone_uid: Option<ResourceUid>,
        policy_revision: Option<u64>,
        provider_ref: String,
        template: String,
        execution_ref: String,
    }

    #[derive(Clone, Debug)]
    struct RecordedStop {
        term_timeout: Duration,
        kill_timeout: Duration,
    }

    #[derive(Clone)]
    struct FakeEffectsConfig {
        /// Scripted adoption results; the last one repeats once exhausted.
        adoption: VecDeque<ProviderAdoption>,
        launch: Result<ProcessIdentityDigest, String>,
        /// Whether the fake reports a live retained identity.
        active: bool,
    }

    impl Default for FakeEffectsConfig {
        fn default() -> Self {
            Self {
                adoption: VecDeque::from([ProviderAdoption::Absent]),
                launch: Ok(ProcessIdentityDigest::from_bytes([0x51; 32])),
                active: true,
            }
        }
    }

    /// Scripted [`ProcessDriverEffects`] double: records every call with the
    /// exact ticket inputs the driver derived (KTD7) and replays a scripted
    /// adoption sequence.
    struct FakeEffects {
        config: Mutex<FakeEffectsConfig>,
        calls: Mutex<Vec<&'static str>>,
        launches: Mutex<Vec<RecordedLaunch>>,
        stops: Mutex<Vec<RecordedStop>>,
        finalizes: Mutex<usize>,
    }

    impl FakeEffects {
        fn new(config: FakeEffectsConfig) -> Self {
            Self {
                config: Mutex::new(config),
                calls: Mutex::new(Vec::new()),
                launches: Mutex::new(Vec::new()),
                stops: Mutex::new(Vec::new()),
                finalizes: Mutex::new(0),
            }
        }

        fn push_adoption(&self, adoption: ProviderAdoption) {
            self.config.lock().adoption.push_back(adoption);
        }

        fn set_launch(&self, result: Result<ProcessIdentityDigest, String>) {
            self.config.lock().launch = result;
        }

        fn launch_calls(&self) -> Vec<RecordedLaunch> {
            self.launches.lock().clone()
        }

        fn stop_calls(&self) -> Vec<RecordedStop> {
            self.stops.lock().clone()
        }

        fn finalize_calls(&self) -> usize {
            *self.finalizes.lock()
        }

        fn call_order(&self) -> Vec<&'static str> {
            self.calls.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl super::ProcessDriverEffects for FakeEffects {
        async fn launch(
            &self,
            identity: &super::ProcessResourceIdentity,
            spec: &ProcessSpec,
            _timeout: Duration,
        ) -> Result<ProcessIdentityDigest, String> {
            self.calls.lock().push("launch");
            self.launches.lock().push(RecordedLaunch {
                resource_ref: identity.resource_ref.to_canonical_string(),
                resource_uid: identity.resource_uid.as_str().to_owned(),
                generation: identity.resource_generation.get(),
                zone: identity.zone.clone(),
                zone_uid: identity.zone_uid.clone(),
                policy_revision: identity.policy_revision,
                provider_ref: identity.provider_ref.to_canonical_string(),
                template: spec.execution().template().as_str().to_owned(),
                execution_ref: spec.execution().execution_ref().to_canonical_string(),
            });
            self.config.lock().launch.clone()
        }

        async fn adopt(
            &self,
            _identity: &super::ProcessResourceIdentity,
            _spec: &ProcessSpec,
        ) -> Result<ProviderAdoption, String> {
            self.calls.lock().push("adopt");
            let mut config = self.config.lock();
            Ok(config.adoption.pop_front().unwrap_or(ProviderAdoption::Absent))
        }

        async fn stop(
            &self,
            _identity: &super::ProcessResourceIdentity,
            _spec: &ProcessSpec,
            term_timeout: Duration,
            kill_timeout: Duration,
        ) -> Result<bool, String> {
            self.calls.lock().push("stop");
            self.stops.lock().push(RecordedStop { term_timeout, kill_timeout });
            Ok(true)
        }

        async fn stop_stale(
            &self,
            _provider_ref: &ResourceRef,
            _candidate: &AdoptionCandidate,
        ) -> Result<(), String> {
            self.calls.lock().push("stop-stale");
            Ok(())
        }

        async fn finalize(&self, _identity: &super::ProcessResourceIdentity) -> Result<(), String> {
            self.calls.lock().push("finalize");
            *self.finalizes.lock() += 1;
            Ok(())
        }

        fn has_active(
            &self,
            _zone: &ZoneId,
            _zone_uid: Option<&ResourceUid>,
            _resource_ref: &ResourceRef,
        ) -> bool {
            self.config.lock().active
        }
    }

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
        let (effects_tx, effects_rx) = mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = mpsc::unbounded_channel();
        let (requeue, requeue_rx) = RecordingRequeue::new();
        let ctx = ResourceContext::new(
            row.clone(),
            TargetHandle::Host,
            process_spec_decoder(),
            Arc::new(DeadManager),
            Arc::new(requeue.clone()),
            effects_tx,
            notify_tx,
        );
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
            self.requeue.recorded().into_iter().map(|(_, after)| after).collect()
        }
    }

    fn driver_args(effects: Arc<FakeEffects>) -> ProcessDriverArgs {
        ProcessDriverArgs {
            zone: ZoneId::parse("work").expect("zone"),
            zone_uid: Some(ResourceUid::parse(ZONE_UID).expect("zone uid")),
            policy_revision: Some(7),
            provider_assignment_generation: None,
            controller_generation: ControllerGeneration::new(1).expect("controller generation"),
            guest_execution: None,
            mode: DaemonMode::Host,
            effects,
        }
    }

    /// The driver under test: the erased boundary the actor holds, plus the
    /// typed handle for in-memory assertions.
    struct DriverUnderTest {
        erased: Box<dyn DynResourceDriver>,
        typed: ProcessDriver,
    }

    impl DriverUnderTest {
        async fn validate(
            &mut self,
            ctx: &mut ResourceContext,
        ) -> Result<(), DriverFailure> {
            self.erased.validate(ctx).await
        }

        async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, DriverFailure> {
            self.erased.recover(ctx).await
        }

        async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, DriverFailure> {
            self.erased.reconcile(ctx).await
        }

        async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
            self.erased.delete(ctx).await
        }

        fn restart_count(&self) -> u32 {
            self.typed.restart_count()
        }
    }

    async fn driver(effects: Arc<FakeEffects>) -> DriverUnderTest {
        let args = driver_args(effects);
        let typed = ProcessDriver::new(args);
        let erased: Box<dyn DynResourceDriver> = Box::new(typed.clone());
        DriverUnderTest { erased, typed }
    }

    fn expect_in_progress(outcome: Result<ReconcileOutcome, DriverFailure>) -> d2b_resource_runtime::context::OperationId {
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
    #[tokio::test]
    async fn identity_falls_back_to_the_authored_owner_reference() {
        let mut row = test_row();
        row.metadata =
            br#"{"annotations":{},"labels":{},"ownerRef":"Provider/network-local"}"#.to_vec();
        let mut f = fixture(row);
        assert!(f.ctx.owner_key().is_none(), "fixture resolves no owner key");
        let d = driver(Arc::new(FakeEffects::new(FakeEffectsConfig::default()))).await;
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
            identity.owner_ref,
            Some(ResourceRef::parse("Provider/network-local").expect("owner ref"))
        );
    }

    // -- committed controller-provider identity (KTD7) -----------------------

    /// The committed Provider uid the test source publishes.
    const COMMITTED_PROVIDER_UID: &str = "123e4567-e89b-42d3-a456-426614174010";
    /// The committed Provider generation the test source publishes.
    const COMMITTED_PROVIDER_GENERATION: u64 = 4;

    /// A controller-class Process row owned by a Provider: the row shape the
    /// controller ticket needs its owner's committed identity for.
    fn controller_row() -> StoredDesiredResource {
        let mut row = test_row();
        row.key = ResourceKey::new("work", "Process", "controller");
        row.metadata =
            br#"{"annotations":{},"labels":{},"ownerRef":"Provider/network-local"}"#.to_vec();
        row.spec = br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"controller","template":"reaction","drainTimeout":"250ms"}"#.to_vec();
        row
    }

    async fn controller_identity(f: &mut Fixture) -> super::ProcessResourceIdentity {
        let d = driver(Arc::new(FakeEffects::new(FakeEffectsConfig::default()))).await;
        d.typed
            .identity(
                &mut f.ctx,
                &ResourceRef::parse("Provider/system-minijail").expect("provider ref"),
                DriverOp::Reconcile,
            )
            .await
            .expect("identity")
    }

    /// Fixed committed Provider rows: the view the plane registry publishes
    /// after Nix bundle ingestion (KTD7).
    #[derive(Default)]
    struct FixedProviderIdentities(
        std::collections::BTreeMap<String, (ResourceUid, ResourceGeneration)>,
    );

    impl FixedProviderIdentities {
        fn with(mut self, provider_ref: &str, uid: &str, generation: u64) -> Self {
            self.0.insert(
                provider_ref.to_owned(),
                (
                    ResourceUid::parse(uid).expect("provider uid"),
                    ResourceGeneration::new(generation).expect("provider generation"),
                ),
            );
            self
        }
    }

    impl super::CommittedProviderIdentitySource for FixedProviderIdentities {
        fn committed_provider_identity(
            &self,
            provider_ref: &ResourceRef,
        ) -> Option<(ResourceUid, ResourceGeneration)> {
            self.0.get(&provider_ref.to_canonical_string()).cloned()
        }
    }

    /// KTD7: a controller row owned by a Provider takes the owner's committed
    /// uid/generation into the provider context, so the controller bootstrap
    /// ticket forms instead of refusing with
    /// `provider-controller-provider-identity-missing`.
    #[tokio::test]
    async fn controller_provider_identity_binds_the_committed_provider_row() {
        let mut f = fixture(controller_row());
        let identity = controller_identity(&mut f).await;
        assert_eq!(
            identity.process_class,
            ProcessClass::Controller,
            "the row decodes as a controller"
        );
        assert_eq!(
            identity.owner_ref,
            Some(ResourceRef::parse("Provider/network-local").expect("owner ref"))
        );

        let source = FixedProviderIdentities::default().with(
            "Provider/network-local",
            COMMITTED_PROVIDER_UID,
            COMMITTED_PROVIDER_GENERATION,
        );
        let context = super::bind_committed_controller_provider_identity(
            &identity,
            Some(&source as &dyn super::CommittedProviderIdentitySource),
        );
        assert_eq!(
            context.provider_uid.as_ref().map(ResourceUid::as_str),
            Some(COMMITTED_PROVIDER_UID)
        );
        assert_eq!(
            context.provider_generation,
            Some(ResourceGeneration::new(COMMITTED_PROVIDER_GENERATION).expect("generation"))
        );
    }

    /// KTD7: a Provider the daemon retains no committed row for - and an
    /// unwired production effects value - leaves the identity unbound, so the
    /// ticket path still refuses closed instead of inventing an identity.
    #[tokio::test]
    async fn controller_provider_identity_stays_unbound_without_a_committed_row() {
        let mut f = fixture(controller_row());
        let identity = controller_identity(&mut f).await;

        let empty = FixedProviderIdentities::default();
        let unretained = super::bind_committed_controller_provider_identity(
            &identity,
            Some(&empty as &dyn super::CommittedProviderIdentitySource),
        );
        assert_eq!(unretained.provider_uid, None);
        assert_eq!(unretained.provider_generation, None);

        let unwired = super::bind_committed_controller_provider_identity(&identity, None);
        assert_eq!(unwired.provider_uid, None);
        assert_eq!(unwired.provider_generation, None);
    }

    #[tokio::test]
    async fn factory_registers_only_the_process_resource_type() {
        let args = driver_args(Arc::new(FakeEffects::new(FakeEffectsConfig::default())));
        let factory = ProcessDriverFactory::new(args);
        assert_eq!(factory.resource_types().len(), 1);
        assert_eq!(factory.resource_types()[0].as_str(), "Process");
        let key = ResourceKey::new("work", "Process", "worker");
        let _erased = factory.create(&key).await;
    }

    // -- launch happy path ---------------------------------------------------

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn launch_reaches_ready_with_expected_ticket_inputs() {
        let fake = Arc::new(FakeEffects::new(FakeEffectsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            ..FakeEffectsConfig::default()
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
        assert_eq!(launch.zone_uid.as_ref().map(ResourceUid::as_str), Some(ZONE_UID));
        assert_eq!(launch.policy_revision, Some(7));
        assert_eq!(launch.provider_ref, "Provider/system-minijail");
        assert_eq!(launch.template, "reaction");
        assert_eq!(launch.execution_ref, "Host/host-system");

        let completed = f.effects.recv().await.expect("typed completion");
        assert_eq!(completed.operation, operation);
        assert!(matches!(completed.result, d2b_resource_runtime::context::EffectResult::Completed));

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

    // -- recover: adoption / quarantine / missing ----------------------------

    #[tokio::test]
    async fn recover_adopts_a_live_matching_process_without_launching() {
        let fake = Arc::new(FakeEffects::new(FakeEffectsConfig {
            adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
            ..FakeEffectsConfig::default()
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

    #[tokio::test]
    async fn recover_classifies_drifted_and_ambiguous_processes_as_quarantined() {
        for adoption in [
            ProviderAdoption::Stale { candidate: stale_candidate() },
            ProviderAdoption::Quarantined(quarantined_report()),
        ] {
            let fake = Arc::new(FakeEffects::new(FakeEffectsConfig {
                adoption: VecDeque::from([adoption]),
                ..FakeEffectsConfig::default()
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

    // -- delete: term then kill ----------------------------------------------

    #[tokio::test]
    async fn delete_stops_term_then_kill_and_finalizes() {
        let fake = Arc::new(FakeEffects::new(FakeEffectsConfig {
            adoption: VecDeque::from([ProviderAdoption::Adopted(adopted_report())]),
            ..FakeEffectsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        driver.delete(&mut f.ctx).await.expect("delete");

        let stops = fake.stop_calls();
        assert_eq!(stops.len(), 1, "one preserved stop escalation");
        assert_eq!(stops[0].term_timeout, Duration::from_millis(250), "drain timeout from the spec");
        assert_eq!(stops[0].kill_timeout, Duration::from_secs(30), "preserved kill budget");
        assert_eq!(fake.finalize_calls(), 1, "finalize after the exact stop");
        assert_eq!(fake.call_order(), ["adopt", "stop", "finalize"]);
    }

    #[tokio::test]
    async fn delete_without_a_live_process_is_a_noop() {
        let fake = Arc::new(FakeEffects::new(FakeEffectsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            ..FakeEffectsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        driver.delete(&mut f.ctx).await.expect("delete");
        assert!(fake.stop_calls().is_empty());
        assert_eq!(fake.finalize_calls(), 0);
    }

    #[tokio::test]
    async fn delete_stops_an_exact_stale_candidate_after_restart() {
        let fake = Arc::new(FakeEffects::new(FakeEffectsConfig {
            adoption: VecDeque::from([ProviderAdoption::Stale { candidate: stale_candidate() }]),
            ..FakeEffectsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        driver.delete(&mut f.ctx).await.expect("delete");
        assert_eq!(fake.call_order(), ["adopt", "stop-stale"]);
    }

    #[tokio::test]
    async fn delete_refuses_an_ambiguous_identity() {
        let fake = Arc::new(FakeEffects::new(FakeEffectsConfig {
            adoption: VecDeque::from([ProviderAdoption::Quarantined(quarantined_report())]),
            ..FakeEffectsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        let failure = driver.delete(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Delete);
    }

    // -- retryable reconcile failure -> exactly one requeue with backoff -----

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn retryable_reconcile_failure_requeues_exactly_once_with_backoff() {
        let fake = Arc::new(FakeEffects::new(FakeEffectsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            launch: Err("provider-effect:launch-failed".to_owned()),
            ..FakeEffectsConfig::default()
        }));
        let mut f = fixture(test_row());
        let mut driver = driver(fake.clone()).await;

        // First launch attempt fails; the spawned effect reports a retryable
        // failure (the restart policy allows restarts).
        expect_in_progress(driver.reconcile(&mut f.ctx).await);
        yield_until_effects_settled().await;
        let completed = f.effects.recv().await.expect("completion");
        assert!(matches!(completed.result, d2b_resource_runtime::context::EffectResult::Failed(_)));
        if let d2b_resource_runtime::context::EffectResult::Failed(failure) = completed.result {
            assert_eq!(failure.class(), FailureClass::Retryable);
            assert_eq!(failure.op(), DriverOp::Reconcile);
        }

        // The next pass schedules exactly one runtime-only requeue with the
        // policy restart delay (in-memory restart budget, spec section 32).
        assert_eq!(
            driver.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
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

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn restart_budget_is_in_memory_only() {
        let policy = r#"{"backoffBase":"1s","backoffMax":"60s","backoffMultiplierMilli":2000,"maxRestarts":1,"resetAfter":"60s"}"#;
        let mut row = test_row();
        row.spec = spec_bytes(Some(policy));
        let fake = Arc::new(FakeEffects::new(FakeEffectsConfig {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            launch: Err("provider-effect:launch-failed".to_owned()),
            ..FakeEffectsConfig::default()
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
            ReconcileOutcome::Satisfied
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

    // -- validate ------------------------------------------------------------

    #[tokio::test]
    async fn validate_rejects_an_unsupported_provider() {
        let mut row = test_row();
        row.spec = br#"{"providerRef":"Provider/other","executionRef":"Host/host-system","processClass":"worker","template":"reaction"}"#
            .to_vec();
        let fake = Arc::new(FakeEffects::new(FakeEffectsConfig::default()));
        let mut f = fixture(row);
        let mut driver = driver(fake).await;

        let failure = driver.validate(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Validate);
    }

    #[tokio::test]
    async fn validate_rejects_a_malformed_spec() {
        let mut row = test_row();
        row.spec = b"{not-json".to_vec();
        let fake = Arc::new(FakeEffects::new(FakeEffectsConfig::default()));
        let mut f = fixture(row);
        let mut driver = driver(fake).await;

        let failure = driver.validate(&mut f.ctx).await.unwrap_err();
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.op(), DriverOp::Validate);
    }
}
