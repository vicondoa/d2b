//! Daemon-owned composition of the fixed process Providers.
//!
//! The Provider crates remain pure controllers: this module constructs the
//! composed runtime from the authenticated broker transport and the trusted
//! bundle, and that runtime crosses the provider boundary as the family's
//! declared [`ProcessProviderRuntime`] facet (U1) - the family's effects
//! implementation consumes it, and the daemon never hands a Provider a
//! broker socket or a bundle resolver directly.

use std::{
    collections::{BTreeMap, BTreeSet},
    os::fd::{AsFd, OwnedFd},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::Mutex;

use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_resource::v3::execution_policy::{BoundedToken, ExecutionDomain};
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceSpec, ResourceUid,
    SchemaFingerprint, ZoneId,
    process::ReadinessClass,
    process::{EphemeralProcessSpec, ProcessClass, ProcessSpec},
    volume::{AttachmentAccess, AttachmentCache},
};
use d2b_core::{
    bundle_resolver::BundleResolver,
    processes::{ProcessExecutionDomain, ProcessNode, ProcessRole},
    site::SiteJson,
};
use d2b_provider_credential::{
    ENTRA_BACKEND_REF, MANAGED_IDENTITY_BACKEND_REF, MANAGED_IDENTITY_PROVIDER_REF,
    SECRET_SERVICE_BACKEND_REF,
};
use d2b_provider_process::{
    CommittedProviderIdentitySource, DeviceWorkerFamily, DeviceWorkerLaunch, ExecutionMode,
    GpuWorkerParams, LaunchRow, ProcessFamilySpec, ProcessProviderRuntime,
    ProcessResourceContext, ProcessResourceIdentity, ProviderAdoption, ProviderLaunch,
    ProviderLiveness, ServingWorkerLaunch, ServingWorkerRoot, SwtpmFlushParams, SwtpmWorkerParams,
    VideoWorkerParams, device_worker_family, device_worker_vm, execution_target_allowed,
    resolve_launch_identity, resource_uid_from_bytes,
};
use d2b_resource_runtime::context::ResourceContext;
use d2b_resource_runtime::identity::ResourceKey;
use d2b_process_conformance::{
    AdoptionCandidate, AdoptionOutcome, CompiledDigests, ConfigurationDigest,
    GuestExecutionBinding, IdentityBinding, LaunchIdentity, LaunchTicket, OperationBinding,
    ProcessConformanceError, ProcessIdentityDigest, ProcessLaunchEffectPort, ProcessProvider,
    ReadinessExpectation, SandboxCompiler, StopClass, execution_commitment,
    runtime_scope_commitment,
};
use d2b_provider_supervisor::{
    BrokerProcessBackend, BrokerSystemdEffectOwner, BundleBackedLaunchResolver, ProviderSupervisor,
    SystemdProcessBackend,
};
use d2b_provider_host::{HostEffectFacets, MinijailPlatformGateSource};
use d2b_provider_process_minijail::{MinijailProcessProvider, launch::PlatformGate};
use d2b_provider_process_systemd::SystemdProcessProvider;
use d2b_provider_toolkit::CredentialDeliveryKeyHandoff;
use d2b_session::AuthenticatedSessionRouteBinding;
use d2b_session_unix::{PeerCredentials, SeqpacketSocket, prearmed_seqpacket_pair};
use d2bd_runtime::supervisor::readiness_liveness::RunnerLiveness;
use d2bd_runtime::target_runtime::DaemonMode;
use d2bd_runtime::vm_start_support::{
    is_durable_wayland_process_node, is_guest_owned_process_node,
};
use sha2::{Digest, Sha256};

use crate::provider_effects::FixedEffectAdapter;

/// The fixed process Provider names wired by the daemon.
pub const FIXED_PROCESS_PROVIDER_NAMES: [&str; 2] = ["system-minijail", "system-systemd"];
pub(crate) const GUEST_EXECUTION_UNAVAILABLE: &str = "provider-ticket:guest-execution-unavailable";

type BrokerProcessSupervisor = ProviderSupervisor<BrokerProcessBackend<BundleBackedLaunchResolver>>;
type BrokerSystemdSupervisor = ProviderSupervisor<SystemdProcessBackend<BrokerSystemdEffectOwner>>;
pub(crate) type ControllerSessionReconcileWake = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

async fn wait_for_controller_bootstrap_endpoint(
    endpoint: OwnedFd,
    timeout: Duration,
) -> Result<OwnedFd, String> {
    // Readiness-driven AsyncFd wait (plan U13 classification), not a worker
    // seat: a seat would serialize concurrent provider bootstraps head-of-line.
    // The reactor owns readiness, so the descriptor must be non-blocking.
    let flags = rustix::fs::fcntl_getfl(&endpoint)
        .map_err(|_| "provider-controller-bootstrap-wait-failed".to_owned())?;
    if !flags.contains(rustix::fs::OFlags::NONBLOCK) {
        rustix::fs::fcntl_setfl(&endpoint, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(|_| "provider-controller-bootstrap-wait-failed".to_owned())?;
    }
    let endpoint = tokio::io::unix::AsyncFd::new(endpoint)
        .map_err(|_| "provider-controller-bootstrap-wait-failed".to_owned())?;
    let mut guard = tokio::time::timeout(timeout, endpoint.readable())
        .await
        .map_err(|_| "provider-controller-bootstrap-timeout".to_owned())?
        .map_err(|_| "provider-controller-bootstrap-wait-failed".to_owned())?;
    // The reactor observed POLLIN/POLLERR/POLLHUP readiness (the same
    // interest set the poll wait used); the readiness is consumed and the
    // endpoint returned, so the caller's read observes the frame or the
    // peer's closure.
    guard.clear_ready();
    Ok(endpoint.into_inner())
}

/// Probe the host posture needed by the daemon-owned minijail Provider.
///
/// The Provider receives this bounded snapshot through its constructor; it
/// never reads host paths or cgroup state itself.
pub(crate) fn detect_minijail_platform_gate() -> PlatformGate {
    // Genuinely synchronous path (plan R11): the gate is read by the sync
    // `ProductionProcessProviders::new_for_mode` constructor (whose callers
    // are outside this unit's scope) and by the Host family's declared
    // minijail-gate facet adapter (U5, `resource_plane_v3.rs`), which
    // supplies the same bounded snapshot to the host provider crate;
    // converting to tokio::fs would require an async constructor and
    // out-of-scope caller changes. The reads are two short
    // /proc stats (osrelease, cgroup), negligible blocking-pool pressure.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    let (kernel_major, kernel_minor) = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .and_then(|release| {
            let mut components = release.split('.');
            let major = components.next()?.parse().ok()?;
            let minor = components
                .next()
                .and_then(|component| {
                    component
                        .split(|character: char| !character.is_ascii_digit())
                        .next()
                })
                .filter(|component| !component.is_empty())?
                .parse()
                .ok()?;
            Some((major, minor))
        })
        .unwrap_or((0, 0));
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    let cgroup_kill_available = std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|cgroup| {
            let relative = cgroup
                .lines()
                .find_map(|line| line.strip_prefix("0::"))?
                .trim()
                .trim_start_matches('/')
                .to_owned();
            let path = std::path::Path::new("/sys/fs/cgroup")
                .join(relative)
                .join("cgroup.kill");
            Some(path)
        })
        .is_some_and(|path| path.is_file());
    PlatformGate::from_observed(kernel_major, kernel_minor, cgroup_kill_available)
}

/// Production `MinijailPlatformGateSource` (U5): the Host family's declared
/// facet implementation supplies the daemon's own bounded minijail platform
/// gate - the same kernel/cgroup posture probe the daemon-owned minijail
/// Provider is constructed from - converted to the system-core gate type
/// the family reads. The family crate receives the bounded snapshot and
/// never calls a daemon function or reads a daemon path.
#[derive(Debug, Clone, Copy, Default)]
struct ProductionMinijailPlatformGateSource;

impl MinijailPlatformGateSource for ProductionMinijailPlatformGateSource {
    fn platform_gate(&self) -> d2b_provider_host::MinijailPlatformGate {
        convert_platform_gate(detect_minijail_platform_gate())
    }
}

/// Convert the daemon's own minijail `PlatformGate` snapshot into the Host
/// family's gate type: the same bounded kernel/cgroup posture, field for
/// field.
fn convert_platform_gate(gate: PlatformGate) -> d2b_provider_host::MinijailPlatformGate {
    d2b_provider_host::MinijailPlatformGate::new(
        gate.kernel_major,
        gate.kernel_minor,
        gate.cgroup_kill_available,
    )
}

/// The Host family's declared facet set, composed beside the gate probe it
/// wraps (U5): the daemon's own minijail platform gate is the one
/// daemon-owned read the family's probe needs; every other probe input is
/// host state the family crate reads itself. The facet carries the crate's
/// production probe built over that gate source.
pub(crate) fn production_host_facets() -> HostEffectFacets {
    HostEffectFacets {
        probe: d2b_provider_host::production_probe(Arc::new(
            ProductionMinijailPlatformGateSource,
        )),
    }
}

fn retryable_stop_error(error: &str) -> bool {
    matches!(
        error,
        "stop-failed"
            | "observe-failed"
            | "launch-failed"
            | "effect-adapter-busy"
            | "deadline-exceeded"
            | "process-fate-unknown"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedProvider {
    Minijail,
    Systemd,
}

#[derive(Debug, Clone, Copy)]
struct ManagedProcess {
    provider: ManagedProvider,
    identity: ProcessIdentityDigest,
}

#[derive(Clone)]
struct ManagedResource {
    zone: ZoneId,
    zone_uid: Option<ResourceUid>,
    resource_ref: ResourceRef,
    provider: ManagedProvider,
    provider_ref: ResourceRef,
    provider_uid: Option<ResourceUid>,
    provider_generation: Option<ResourceGeneration>,
    owner_ref: Option<ResourceRef>,
    owner_uid: Option<ResourceUid>,
    template: BoundedToken,
    identity: ProcessIdentityDigest,
    uid: ResourceUid,
    generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    execution_ref: ResourceRef,
    target_ref: Option<ResourceRef>,
    runtime_scope: Option<ConfigurationDigest>,
}

impl core::fmt::Debug for ManagedResource {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ManagedResource(<redacted>)")
    }
}

type ManagedResourceKey = (ZoneId, Option<ResourceUid>, ResourceRef);

fn resource_identity_matches(
    managed: &ManagedResource,
    context: &ProcessResourceContext<'_>,
) -> bool {
    resource_identity_mismatches(managed, context).is_empty()
}

/// One identity field comparison between the identity this daemon retained
/// when it realized (or adopted) a process and the identity the current pass
/// resolves.
///
/// Required fields are always known on both sides; the per-pass resolved
/// inputs (KTD7 - the committed zone/provider/owner identities and the
/// target selector a source may or may not retain) are optional, and an
/// unbound reference is *unknown*, never a value.
struct IdentityField {
    field: &'static str,
    managed: Option<String>,
    requested: Option<String>,
}

impl IdentityField {
    /// Any difference, including one side unknown: the strict comparison the
    /// destructive identity gates use (`finalize`, `stop`, `has_active`), so
    /// an unresolved request refuses rather than proceeds.
    fn mismatch(&self) -> bool {
        self.managed != self.requested
    }

    /// A *proven* change: both sides known and different. An input this pass
    /// cannot resolve is unknown, so it never invalidates the identity a
    /// better-informed pass established (2026-09-11: the manager-served
    /// Guest's VMM was retired and relaunched on every daemon restart because
    /// the first recovery pass could not resolve the owning Guest's uid and
    /// the next one could).
    fn proven_change(&self) -> bool {
        matches!(
            (&self.managed, &self.requested),
            (Some(managed), Some(requested)) if managed != requested
        )
    }

    /// The preserved mismatch rendering (`none` for an unbound side).
    fn render(&self) -> String {
        format!(
            "{}(managed={},requested={})",
            self.field,
            self.managed.as_deref().unwrap_or("none"),
            self.requested.as_deref().unwrap_or("none"),
        )
    }
}

fn resource_identity_fields(
    managed: &ManagedResource,
    context: &ProcessResourceContext<'_>,
) -> Vec<IdentityField> {
    fn required(
        fields: &mut Vec<IdentityField>,
        field: &'static str,
        managed: String,
        requested: String,
    ) {
        fields.push(IdentityField {
            field,
            managed: Some(managed),
            requested: Some(requested),
        });
    }
    fn optional(
        fields: &mut Vec<IdentityField>,
        field: &'static str,
        managed: Option<String>,
        requested: Option<String>,
    ) {
        fields.push(IdentityField {
            field,
            managed,
            requested,
        });
    }
    let mut fields = Vec::with_capacity(12);
    required(
        &mut fields,
        "zone",
        managed.zone.to_canonical_string(),
        context.zone.to_canonical_string(),
    );
    optional(
        &mut fields,
        "zone_uid",
        managed.zone_uid.as_ref().map(ResourceUid::to_canonical_string),
        context.zone_uid.as_ref().map(ResourceUid::to_canonical_string),
    );
    required(
        &mut fields,
        "resource_ref",
        managed.resource_ref.to_canonical_string(),
        context.resource_ref.to_canonical_string(),
    );
    required(
        &mut fields,
        "provider_ref",
        managed.provider_ref.to_canonical_string(),
        context.provider_ref.to_canonical_string(),
    );
    optional(
        &mut fields,
        "provider_uid",
        managed
            .provider_uid
            .as_ref()
            .map(ResourceUid::to_canonical_string),
        context
            .provider_uid
            .as_ref()
            .map(ResourceUid::to_canonical_string),
    );
    optional(
        &mut fields,
        "provider_generation",
        managed
            .provider_generation
            .map(|generation| generation.get().to_string()),
        context
            .provider_generation
            .map(|generation| generation.get().to_string()),
    );
    optional(
        &mut fields,
        "owner_ref",
        managed
            .owner_ref
            .as_ref()
            .map(ResourceRef::to_canonical_string),
        context
            .owner_ref
            .as_ref()
            .map(ResourceRef::to_canonical_string),
    );
    optional(
        &mut fields,
        "owner_uid",
        managed.owner_uid.as_ref().map(ResourceUid::to_canonical_string),
        context.owner_uid.as_ref().map(ResourceUid::to_canonical_string),
    );
    required(
        &mut fields,
        "resource_uid",
        managed.uid.to_canonical_string(),
        context.resource_uid.to_canonical_string(),
    );
    required(
        &mut fields,
        "resource_generation",
        managed.generation.get().to_string(),
        context.resource_generation.get().to_string(),
    );
    required(
        &mut fields,
        "controller_generation",
        managed.controller_generation.get().to_string(),
        context.controller_generation.get().to_string(),
    );
    optional(
        &mut fields,
        "target_ref",
        managed
            .target_ref
            .as_ref()
            .map(ResourceRef::to_canonical_string),
        context
            .target_ref
            .as_ref()
            .map(ResourceRef::to_canonical_string),
    );
    fields
}

fn resource_identity_mismatches(
    managed: &ManagedResource,
    context: &ProcessResourceContext<'_>,
) -> Vec<String> {
    resource_identity_fields(managed, context)
        .iter()
        .filter(|field| field.mismatch())
        .map(IdentityField::render)
        .collect()
}

/// The *proven* identity changes only: both sides known and different. The
/// retire-before-launch pre-flight uses this rule because it destroys a live
/// process, and an identity input the current pass cannot resolve is unknown
/// rather than changed; the target observation (broker registration plus the
/// Provider's own adoption classification) stays the authority on whether a
/// running process matches.
fn resource_identity_changes(
    managed: &ManagedResource,
    context: &ProcessResourceContext<'_>,
) -> Vec<String> {
    resource_identity_fields(managed, context)
        .iter()
        .filter(|field| field.proven_change())
        .map(IdentityField::render)
        .collect()
}
fn identity_changed_error(mismatches: Vec<String>) -> String {
    tracing::warn!(
        mismatches = %mismatches.join(","),
        "Process provider identity mismatch",
    );
    format!("provider-process-identities-changed:{}", mismatches.join(","))
}

const MAX_CONTROLLER_BOOTSTRAP_ENDPOINTS: usize = 256;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ControllerBootstrapContext {
    zone: ZoneId,
    zone_uid: Option<ResourceUid>,
    process_ref: ResourceRef,
    process_uid: ResourceUid,
    generation: ResourceGeneration,
    process_identity: ProcessIdentityDigest,
    process_provider_ref: ResourceRef,
    provider_owner_ref: ResourceRef,
    provider_uid: ResourceUid,
    provider_generation: ResourceGeneration,
    execution_ref: ResourceRef,
    user_ref: Option<ResourceRef>,
    controller_generation: ControllerGeneration,
}

impl std::fmt::Debug for ControllerBootstrapContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ControllerBootstrapContext(<redacted>)")
    }
}

impl ControllerBootstrapContext {
    fn from_resource_context(
        context: &ProcessResourceContext<'_>,
        execution_ref: &ResourceRef,
        process_identity: ProcessIdentityDigest,
    ) -> Result<Self, String> {
        let provider_owner_ref = context
            .controller_provider_ref
            .clone()
            .or_else(|| {
                context
                    .owner_ref
                    .as_ref()
                    .filter(|owner| owner.resource_type().as_str() == "Provider")
                    .cloned()
            })
            .filter(|owner| owner.resource_type().as_str() == "Provider")
            .ok_or_else(|| "provider-controller-owner-missing".to_owned())?;
        let provider_uid = context
            .provider_uid
            .clone()
            .ok_or_else(|| "provider-controller-provider-identity-missing".to_owned())?;
        let provider_generation = context
            .provider_generation
            .ok_or_else(|| "provider-controller-provider-identity-missing".to_owned())?;
        if context
            .user_ref
            .as_ref()
            .is_some_and(|user| user.resource_type().as_str() != "User")
        {
            return Err("provider-controller-user-identity-invalid".to_owned());
        }
        Ok(Self {
            zone: context.zone.clone(),
            zone_uid: context.zone_uid.clone(),
            process_ref: context.resource_ref.clone(),
            process_uid: context.resource_uid.clone(),
            generation: context.resource_generation,
            process_identity,
            process_provider_ref: context.provider_ref.clone(),
            provider_owner_ref,
            provider_uid,
            provider_generation,
            execution_ref: execution_ref.clone(),
            user_ref: context.user_ref.clone(),
            controller_generation: context.controller_generation,
        })
    }

    pub(crate) fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    pub(crate) fn zone(&self) -> &ZoneId {
        &self.zone
    }

    pub(crate) fn process_uid(&self) -> &ResourceUid {
        &self.process_uid
    }

    pub(crate) const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    pub(crate) const fn process_identity(&self) -> ProcessIdentityDigest {
        self.process_identity
    }

    pub(crate) fn process_provider_ref(&self) -> &ResourceRef {
        &self.process_provider_ref
    }

    pub(crate) fn provider_owner_ref(&self) -> &ResourceRef {
        &self.provider_owner_ref
    }

    pub(crate) fn provider_uid(&self) -> &ResourceUid {
        &self.provider_uid
    }

    pub(crate) const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    pub(crate) fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    pub(crate) const fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }
}

pub(crate) fn controller_session_needs_fence(
    current_bootstrap: Option<&ControllerBootstrapContext>,
    live_session: &ControllerBootstrapContext,
    service_task_finished: bool,
) -> bool {
    service_task_finished || current_bootstrap != Some(live_session)
}

/// Lifetime handle returned by the Guest-local Credential backend supervisor.
pub(crate) trait GuestCredentialBackendLease: Send + Sync {
    /// Bind the responder to the exact authenticated Provider route.
    fn bind_route(
        &self,
        route: &AuthenticatedSessionRouteBinding,
        user_ref: Option<&ResourceRef>,
        peer: Option<PeerCredentials>,
    ) -> Result<(), String>;

    /// Stop the responder and revoke its session-bound backend authority.
    fn cancel(&self);
}

/// One backend endpoint prepared by the Guest-local supervisor.
pub(crate) struct GuestCredentialBackendPreparation {
    pub(crate) child_endpoint: OwnedFd,
    pub(crate) delivery_key_handoff: CredentialDeliveryKeyHandoff,
    pub(crate) lease: Arc<dyn GuestCredentialBackendLease>,
}

/// Guest-local owner of Credential backend responders.
pub(crate) trait GuestCredentialBackendSupervisor: Send + Sync {
    fn prepare(
        &self,
        context: &ProcessResourceContext<'_>,
    ) -> Result<GuestCredentialBackendPreparation, String>;
}

pub(crate) struct ControllerBootstrapEndpoint {
    daemon_endpoint: OwnedFd,
    delivery_key_handoff: Option<CredentialDeliveryKeyHandoff>,
    backend_lease: Option<Arc<dyn GuestCredentialBackendLease>>,
    context: ControllerBootstrapContext,
}

impl ControllerBootstrapEndpoint {
    /// Duplicate the pre-armed bootstrap socket so each establish attempt
    /// wraps its own descriptor while the marker keeps the original open.
    pub(crate) fn daemon_socket(
        &self,
    ) -> Result<SeqpacketSocket, &'static str> {
        let dup = self
            .daemon_endpoint
            .try_clone()
            .map_err(|_| "provider-controller-bootstrap-dup")?;
        SeqpacketSocket::from_parent_prearmed(dup)
            .map_err(|_| "provider-controller-bootstrap-wrap")
    }

    /// Borrow the optional credential handoff and backend lease carried
    /// beside the bootstrap socket.
    pub(crate) fn handles(
        &self,
    ) -> (
        Option<CredentialDeliveryKeyHandoff>,
        Option<Arc<dyn GuestCredentialBackendLease>>,
    ) {
        (
            self.delivery_key_handoff.clone(),
            self.backend_lease.clone(),
        )
    }

    pub(crate) fn context(&self) -> &ControllerBootstrapContext {
        &self.context
    }

}

enum ControllerBootstrapMarker {
    Pending(ControllerBootstrapEndpoint),
    Establishing(ControllerBootstrapContext),
    Active(ControllerBootstrapContext),
}

impl ControllerBootstrapMarker {
    fn context(&self) -> &ControllerBootstrapContext {
        match self {
            Self::Pending(endpoint) => &endpoint.context,
            Self::Establishing(context) | Self::Active(context) => context,
        }
    }
}

/// Readiness-loop adapter for one Provider-managed process node.
///
/// The probe observes the node through the Provider's authenticated read-only
/// path, which is async, so it has a seat for each readiness wait: the async
/// wait awaits [`Self::probe_async`] and never drives a runtime from inside
/// the caller's, while the synchronous wait drives [`Self::probe`] on the
/// daemon runtime captured at construction (U13 synchronous path).
pub struct ProviderLivenessProbe {
    providers: Arc<ProductionProcessProviders>,
    vm: String,
    node: ProcessNode,
    /// The daemon's runtime, captured at construction (all constructors run
    /// on the runtime): the trait's mandatory sync `probe` seat drives its
    /// async observation on this handle (U13 synchronous path, R11 inventory
    /// note). Production readiness waits use `probe_async`; this sync seat
    /// exists for the sync `wait_for_readiness` caller.
    runtime_handle: tokio::runtime::Handle,
}

impl ProviderLivenessProbe {
    /// Bind a Provider composition to one immutable process-DAG node.
    pub fn new(
        providers: Arc<ProductionProcessProviders>,
        vm: impl Into<String>,
        node: &ProcessNode,
    ) -> Self {
        Self {
            providers,
            vm: vm.into(),
            node: node.clone(),
            runtime_handle: tokio::runtime::Handle::current(),
        }
    }

    fn classify(liveness: Result<ProviderLiveness, String>) -> RunnerLiveness {
        match liveness {
            Ok(ProviderLiveness::Alive) => RunnerLiveness::Alive,
            Ok(ProviderLiveness::Exited) => RunnerLiveness::Exited(None),
            Ok(ProviderLiveness::Unknown) | Err(_) => RunnerLiveness::Unknown,
        }
    }
}

#[async_trait::async_trait]
impl d2bd_runtime::supervisor::readiness_liveness::LivenessProbe for ProviderLivenessProbe {
    // U13: the sync seat is the trait's mandatory synchronous observation
    // (the sync `wait_for_readiness` caller), driven on the daemon runtime
    // captured at construction (R11 synchronous path inventory note);
    // production readiness waits use `probe_async` below.
    fn probe(&self) -> RunnerLiveness {
        Self::classify(crate::drive_sync(
            &self.runtime_handle,
            self.providers.probe_node(&self.vm, &self.node),
        ))
    }

    async fn probe_async(&self) -> RunnerLiveness {
        Self::classify(self.providers.probe_node(&self.vm, &self.node).await)
    }
}

/// Production process Provider controllers.
///
/// The daemon-side launched-runner observer: registers the kernel-spawned
/// runner in the authoritative pidfd table (the family handlers' runner
/// lookup) as soon as the broker backend confirms the spawn, before the
/// launch's readiness probe. Registration failure never fails the launch:
/// the stale-entry reap and the supervisor handle still cover signal and
/// liveness.
struct PidfdTableLaunchedObserver {
    pidfd_table: Arc<d2bd_runtime::supervisor::pidfd_table::PidfdTable>,
}

impl d2b_provider_supervisor::LaunchedObserver for PidfdTableLaunchedObserver {
    fn launched(
        &self,
        vm: &str,
        role: &str,
        pid: i32,
        start_time_ticks: u64,
        pidfd: std::os::fd::OwnedFd,
    ) {
        // A broker-confirmed spawn proves a live process now owns this
        // (vm, role). If the slot still holds a STALE entry from a failed
        // prior launch - the launch-failure cleanup stops the child but
        // never clears this table, and its dead pid is what the readiness
        // probe (`ObserveRunnerHandler::runner_lookup`) and the stop path
        // (`SignalRunnerHandler::runner_lookup`) both read - drop the dead
        // entry before registering. Otherwise the relaunch's registration
        // is refused as a duplicate, the probe reports `present: false` for
        // a `gone` pid while the fresh runner is live, the stop refuses
        // against the same gone pid, and the row is wedged (the broker's
        // duplicate-runner guard then refuses every retry spawn).
        //
        // `register` refuses a duplicate by design (the concurrent-spawn
        // guard for one broker runner), so the stale replacement has to
        // happen here. A LIVE duplicate is kept untouched: a second live
        // spawn for one broker runner is impossible (the broker's
        // duplicate-runner guard), and replacing a live entry would orphan
        // its pidfd.
        if self.pidfd_table.contains(vm, role)
            && !self.pidfd_table.still_alive_same_start_time(vm, role)
        {
            tracing::warn!(
                vm,
                role,
                "pidfd-table: dropping stale entry before relaunched runner registration"
            );
            self.pidfd_table.deregister(vm, role);
        }
        match self.pidfd_table.register(
            vm.to_owned(),
            role.to_owned(),
            d2bd_runtime::supervisor::pidfd_table::PidfdEntry {
                pidfd,
                pid,
                start_time_ticks,
            },
        ) {
            Ok(()) => {
                // Persist the registration: the table is restored from disk
                // on daemon restart, and the post-restart adoption of a
                // still-running controller depends on the entry surviving
                // (the old crash-consistent snapshot happened only on the
                // startup-adoption paths).
                if let Err(error) = self.pidfd_table.snapshot() {
                    tracing::warn!(
                        vm,
                        role,
                        error = %error,
                        "pidfd table snapshot failed after launched-runner registration"
                    );
                }
                let _ = pid;
            }
            Err(d2bd_runtime::supervisor::pidfd_table::PidfdTableError::DuplicateRegistration { .. }) => {
                tracing::warn!(
                    vm,
                    role,
                    "pidfd-table: registering a live duplicate for a launched runner"
                );
            }
            Err(error) => {
                tracing::warn!(
                    vm,
                    role,
                    error = %error,
                    "pidfd table registration failed for launched runner"
                );
            }
        }
    }
}

/// The concrete supervisors are retained by the daemon for its whole
/// lifetime. Their internal handles and broker effect owners never cross the
/// Provider boundary; Provider code sees only the
/// `ProcessLaunchEffectPort` implemented by `ProviderSupervisor`.
pub struct ProductionProcessProviders {
    minijail: MinijailProcessProvider<BrokerProcessSupervisor>,
    systemd: SystemdProcessProvider<BrokerSystemdSupervisor>,
    bundle: BundleResolver,
    /// Runtime root the private serving sockets of binding-owned workers live
    /// under (the broker socket's parent directory).
    socket_runtime_dir: PathBuf,
    mode: DaemonMode,
    fixed_effect: FixedEffectAdapter,
    guest_backend_supervisor: Option<Arc<dyn GuestCredentialBackendSupervisor>>,
    managed: Mutex<BTreeMap<(String, String), ManagedProcess>>,
    managed_resources: Mutex<BTreeMap<ManagedResourceKey, ManagedResource>>,
    controller_bootstrap: Mutex<BTreeMap<(ZoneId, ResourceRef), ControllerBootstrapMarker>>,
    controller_session_wakers: Mutex<BTreeMap<ZoneId, ControllerSessionReconcileWake>>,
}

impl std::fmt::Debug for ProductionProcessProviders {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductionProcessProviders")
            .field("providers", &FIXED_PROCESS_PROVIDER_NAMES)
            .finish()
    }
}

impl ProductionProcessProviders {
    /// Construct both fixed process Providers over the authenticated broker.
    pub fn new(
        bundle: BundleResolver,
        broker_socket: impl Into<PathBuf>,
        caller_role: BrokerCallerRole,
        pidfd_table: Arc<d2bd_runtime::supervisor::pidfd_table::PidfdTable>,
    ) -> Self {
        Self::new_for_mode(
            bundle,
            broker_socket,
            caller_role,
            DaemonMode::Host,
            pidfd_table,
        )
    }

    /// Construct both fixed process Providers over a mode-bound broker.
    ///
    /// The mode is selected once at construction. It is passed to both
    /// concrete broker adapters and cannot be widened by a Process ticket.
    pub fn new_for_mode(
        bundle: BundleResolver,
        broker_socket: impl Into<PathBuf>,
        caller_role: BrokerCallerRole,
        mode: DaemonMode,
        pidfd_table: Arc<d2bd_runtime::supervisor::pidfd_table::PidfdTable>,
    ) -> Self {
        let broker_socket = broker_socket.into();
        let socket_runtime_dir = broker_socket
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("/run/d2b"));
        let fixed_socket = broker_socket.clone();
        let daemon_uid = caller_uid(&caller_role);
        let resolver = BundleBackedLaunchResolver::new(bundle.clone()).with_observation_socket(
            broker_socket.clone(),
            Duration::from_secs(10),
            caller_role.clone(),
        );
        let mut minijail_backend = BrokerProcessBackend::with_socket_profile_and_role(
            resolver.clone(),
            broker_socket.clone(),
            Duration::from_secs(10),
            mode.broker_profile(),
            caller_role.clone(),
        );
        // The kernel-spawned runner must be visible to the family handlers'
        // runner lookup (the daemon's pidfd table) before the launch's
        // readiness probe runs, so the table registration rides the backend's
        // launch-success notification - not the driver's post-launch path,
        // which runs after the probe.
        minijail_backend.set_launched_observer(Box::new(PidfdTableLaunchedObserver {
            pidfd_table: pidfd_table.clone(),
        }));
        let systemd_owner = BrokerSystemdEffectOwner::with_socket_and_role(
            resolver,
            broker_socket,
            Duration::from_secs(10),
            caller_role,
        );
        let fixed_effect = FixedEffectAdapter::for_mode(mode, fixed_socket, daemon_uid);
        let platform_gate = detect_minijail_platform_gate();
        Self {
            minijail: MinijailProcessProvider::with_platform_gate(
                ProviderSupervisor::new(minijail_backend),
                platform_gate,
            ),
            systemd: SystemdProcessProvider::new(ProviderSupervisor::new(
                SystemdProcessBackend::new(systemd_owner),
            )),
            bundle,
            socket_runtime_dir,
            mode,
            fixed_effect,
            guest_backend_supervisor: None,
            managed: Mutex::new(BTreeMap::new()),
            managed_resources: Mutex::new(BTreeMap::new()),
            controller_bootstrap: Mutex::new(BTreeMap::new()),
            controller_session_wakers: Mutex::new(BTreeMap::new()),
        }
    }

    /// Return the fixed daemon mode bound to these Process Providers.
    pub const fn mode(&self) -> DaemonMode {
        self.mode
    }

    /// Return the broker profile sealed into both concrete Process adapters.
    pub const fn broker_profile(&self) -> d2b_contracts_broker::broker_wire::BrokerProfile {
        self.mode.broker_profile()
    }

    /// Borrow the daemon-owned minijail Provider.
    pub const fn minijail(&self) -> &MinijailProcessProvider<BrokerProcessSupervisor> {
        &self.minijail
    }

    /// Borrow the trusted bundle this composition was built from. The Process
    /// controller derives Device-worker launch parameters from the same
    /// trusted intent table the provider ticket resolves through.
    pub(crate) const fn bundle(&self) -> &BundleResolver {
        &self.bundle
    }

    /// The runtime root the daemon owns (the broker socket's parent), under
    /// which the per-VM device sockets live.
    pub(crate) fn socket_runtime_dir(&self) -> &std::path::Path {
        &self.socket_runtime_dir
    }

    /// Borrow the daemon-owned systemd Provider.
    pub const fn systemd(&self) -> &SystemdProcessProvider<BrokerSystemdSupervisor> {
        &self.systemd
    }

    /// Return the fixed Provider names in contract order.
    pub const fn provider_names() -> &'static [&'static str; 2] {
        &FIXED_PROCESS_PROVIDER_NAMES
    }

    /// Catalog-bound `Guest` setup descriptor digest for one owner reference.
    ///
    /// This is the same value the old runner snapshotted into its
    /// `set_guest_descriptor_digests` map from the loaded Guest setup
    /// descriptors; the private guest VMM intent lookup
    /// (`BundleResolver::find_guest_vmm_intent`) refuses a ticket without it,
    /// so a controller-owned `Process/<guest>-vmm` row cannot launch until the
    /// bundle's descriptor digest is bound.
    pub(crate) fn guest_setup_descriptor_digest(
        &self,
        zone: &ZoneId,
        guest_ref: &ResourceRef,
    ) -> Option<SchemaFingerprint> {
        self.bundle
            .guest_setup_descriptor_bytes(zone.as_str(), guest_ref.name().as_str())
            .and_then(|bytes| {
                d2b_provider_guest_cloud_hypervisor::GuestSetupDescriptor::from_canonical_bytes(
                    bytes,
                )
                .ok()
                .map(|descriptor| descriptor.descriptor_digest().clone())
            })
    }

    /// Borrow the fixed mode-bound effect adapter used by controller
    /// launches.
    pub const fn fixed_effect(&self) -> &FixedEffectAdapter {
        &self.fixed_effect
    }

    fn validate_execution_target(&self, target: &ResourceRef) -> Result<(), String> {
        let expected = match self.mode {
            DaemonMode::Host => "Host",
            DaemonMode::Guest => "Guest",
        };
        if target.resource_type().as_str() == expected {
            Ok(())
        } else {
            Err("process-execution-target-not-owned-by-daemon".to_owned())
        }
    }

    /// Return whether this node is a daemon-owned Provider process.
    pub fn supports_node(node: &ProcessNode) -> bool {
        !is_guest_owned_process_node(node)
            && !is_durable_wayland_process_node(node)
            && matches!(
                node.role,
                ProcessRole::SwtpmPreStartFlush
                    | ProcessRole::Swtpm
                    | ProcessRole::Virtiofsd
                    | ProcessRole::QemuMediaRunner
                    | ProcessRole::ActivationNixosRunner
                    | ProcessRole::Gpu
                    | ProcessRole::GpuRenderNode
                    | ProcessRole::Audio
                    | ProcessRole::Video
                    | ProcessRole::VsockRelay
                    | ProcessRole::OtelHostBridge
                    | ProcessRole::Usbip
                    | ProcessRole::WaylandProxy
            )
    }

    /// Return whether this node remains supervised after its start step.
    pub fn is_long_lived(node: &ProcessNode) -> bool {
        !matches!(
            node.role,
            ProcessRole::SwtpmPreStartFlush | ProcessRole::ActivationNixosRunner
        ) && Self::supports_node(node)
    }

    /// Return the stable role key used by the broker and daemon stop paths.
    pub fn tracked_role_id(node: &ProcessNode) -> String {
        if matches!(node.role, ProcessRole::CloudHypervisorRunner) {
            "ch-runner".to_owned()
        } else {
            node.id.0.clone()
        }
    }

    /// Return all Provider-managed long-lived roles declared for one VM.
    pub fn managed_role_ids(&self, vm: &str) -> Vec<String> {
        let Some(dag) = self.bundle.find_process_vm(vm) else {
            return Vec::new();
        };
        dag.nodes
            .iter()
            .filter(|node| Self::is_long_lived(node))
            .map(Self::tracked_role_id)
            .collect()
    }

    /// Return a cloned trusted process node for a tracked role key.
    pub fn node_for_role(&self, vm: &str, role_id: &str) -> Option<ProcessNode> {
        self.bundle
            .find_process_vm(vm)?
            .nodes
            .iter()
            .find(|node| Self::tracked_role_id(node) == role_id)
            .cloned()
    }

    /// Return every VM that has a process DAG in the trusted bundle.
    pub fn vm_ids(&self) -> Vec<String> {
        self.bundle
            .processes
            .vms
            .iter()
            .map(|dag| dag.vm.clone())
            .collect()
    }

    /// Return whether a Provider-managed identity is currently retained.
    pub fn has_active_role(&self, vm: &str, role_id: &str) -> bool {
        self.managed
            .try_lock()
            .ok()
            .map(|managed| managed.contains_key(&(vm.to_owned(), role_id.to_owned())))
            .unwrap_or(false)
    }

    /// Return whether any Provider-managed long-lived role is retained.
    pub fn has_active_vm(&self, vm: &str) -> bool {
        self.managed
            .try_lock()
            .ok()
            .map(|managed| managed.keys().any(|(managed_vm, _)| managed_vm == vm))
            .unwrap_or(false)
    }

    /// Return Provider role keys with retained exact local authority.
    pub fn active_role_ids(&self, vm: &str) -> Vec<String> {
        self.managed
            .try_lock()
            .ok()
            .map(|managed| {
                managed
                    .keys()
                    .filter(|(managed_vm, _)| managed_vm == vm)
                    .map(|(_, role)| role.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Launch one trusted process node through its selected fixed Provider.
    pub async fn launch_node(
        &self,
        vm: &str,
        node: &ProcessNode,
        timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        if !Self::supports_node(node) {
            return Err("provider-node-unsupported".to_owned());
        }
        let target = node
            .execution_ref
            .clone()
            .unwrap_or_else(|| d2b_core::bundle_resolver::default_execution_ref(vm, &node.role));
        self.validate_execution_target(
            &ResourceRef::parse(&target)
                .map_err(|_| "process-execution-target-invalid".to_owned())?,
        )?;
        let ticket = self.ticket_with_timeout(vm, node, timeout)?;
        let provider = self.provider_for(node);
        let report = match provider {
            ManagedProvider::Minijail => self
                .minijail
                .launch(&ticket)
                .await
                .map_err(provider_error)?,
            ManagedProvider::Systemd => {
                self.systemd.launch(&ticket).await.map_err(provider_error)?
            }
        };
        self.remember(vm, node, report.identity)?;
        Ok(ProviderLaunch {
            identity: report.identity,
        })
    }

    async fn cleanup_failed_resource_launch(
        &self,
        context: &ProcessResourceContext<'_>,
        provider: ManagedProvider,
        identity: ProcessIdentityDigest,
        execution_ref: &ResourceRef,
    ) {
        let _ = self
            .stop_provider_identity(provider, &identity, StopClass::Terminate)
            .await;
        let _ = self.finalize_resource(context.clone()).await;
        self.forget_resource_for_context(context, execution_ref);
    }

    /// Launch one durable Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn launch_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
        timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        let context = context
            .with_execution_ref(spec.execution().execution_ref())
            .with_user_ref(spec.execution().user_ref());
        self.validate_execution_target(spec.execution().execution_ref())?;
        let provider = managed_provider_from_ref(context.provider_ref)?;
        validate_resource_execution_target(self.mode, &context, spec.execution())?;
        let ticket = resource_ticket(
            &self.bundle,
            &context,
            ExecutionIntent {
                execution: spec.execution(),
                activation_input: None,
                spec_bytes: &serde_json::to_vec(spec)
                    .map_err(|_| "provider-ticket:serialization".to_owned())?,
                readiness: Some(spec.readiness().class()),
            },
            provider,
            self.mode,
            timeout,
        )?;
        // A binding-owned serving worker launches with the binding-declared
        // arguments; the template decides whether that is admitted at all
        // (the broker refuses supplied arguments on every other template).
        // A declared Device-owned worker row launches with the typed
        // parameters the Process controller derived from its owning Device
        // row, its trusted template, and the daemon's own runtime paths.
        let ticket = match (
            context.worker_launch.as_ref(),
            context.device_worker_launch.as_ref(),
        ) {
            (Some(launch), None) => ticket
                .with_launch_args(
                    serving_worker_launch_args(
                        &self.bundle,
                        &self.socket_runtime_dir,
                        &context.zone,
                        launch,
                    )
                    .await?,
                )
                .map_err(|_| "provider-ticket:serving-args-invalid".to_owned())?,
            (None, Some(launch)) => ticket
                .with_launch_args(device_worker_launch_args(
                    &self.socket_runtime_dir,
                    launch,
                )?)
                .map_err(|_| "provider-ticket:device-worker-args-invalid".to_owned())?,
            (None, None) => ticket,
            (Some(_), Some(_)) => {
                return Err("provider-ticket:launch-args-conflict".to_owned());
            }
        };
        self.retire_resource_if_identity_changed(
            &context,
            provider,
            ticket.template(),
            spec.execution().execution_ref(),
            ticket.runtime_scope(),
        )
        .await?;
        let controller_bootstrap = ticket.inherited_fd_table().count() != 0;
        if controller_bootstrap && provider != ManagedProvider::Minijail {
            return Err("provider-controller-bootstrap-unsupported".to_owned());
        }
        if controller_bootstrap {
            self.forget_controller_bootstrap_for_resource_context(&context);
        }
        // The daemon keeps its own copy of the escrow daemon end to wait
        // on: the kernel retains the attached descriptor as custody and
        // returns no duplicate (a dup of the caller's own descriptor is
        // refused by the forward carrier's anti-replay fence).
        let mut escrow_wait = None;
        let controller_endpoints = if controller_bootstrap {
            let (daemon_endpoint, child_endpoint) = prearmed_seqpacket_pair()
                .map_err(|_| "provider-controller-bootstrap-create".to_owned())?;
            escrow_wait = Some(
                daemon_endpoint
                    .try_clone()
                    .map_err(|_| "provider-controller-bootstrap-dup".to_owned())?,
            );
            let (child_fds, delivery_key_handoff, backend_lease) =
                if ticket.inherited_fd_table().count() == 2 {
                    let supervisor = self.guest_backend_supervisor.as_ref().ok_or_else(|| {
                        "provider-credential-backend-supervisor-unavailable".to_owned()
                    })?;
                    let preparation = supervisor.prepare(&context)?;
                    (
                        vec![child_endpoint, preparation.child_endpoint],
                        Some(preparation.delivery_key_handoff),
                        Some(preparation.lease),
                    )
                } else {
                    (vec![child_endpoint], None, None)
                };
            Some((
                daemon_endpoint,
                child_fds,
                delivery_key_handoff,
                backend_lease,
            ))
        } else {
            None
        };
        let mut delivery_key_handoff = None;
        let mut backend_lease = None;
        let report = match provider {
            ManagedProvider::Minijail => match controller_endpoints {
                Some((daemon_endpoint, child_fds, key_handoff, lease)) => {
                    delivery_key_handoff = key_handoff;
                    backend_lease = lease;
                    let mut inherited_fds = child_fds;
                    inherited_fds.push(daemon_endpoint);
                    self.minijail
                        .launch_with_inherited_fds(&ticket, inherited_fds)
                        .await
                        .map_err(provider_error)
                }
                None => self.minijail.launch(&ticket).await.map_err(provider_error),
            },
            ManagedProvider::Systemd => self.systemd.launch(&ticket).await.map_err(provider_error),
        };
        let report = match report {
            Ok(report) => report,
            Err(error) => {
                if controller_bootstrap {
                    self.forget_controller_bootstrap_for_resource_context(&context);
                }
                return Err(error);
            }
        };
        self.remember_resource(ManagedResource {
            zone: context.zone.clone(),
            zone_uid: context.zone_uid.clone(),
            resource_ref: context.resource_ref.clone(),
            provider,
            provider_ref: context.provider_ref.clone(),
            provider_uid: context.provider_uid.clone(),
            provider_generation: context.provider_generation,
            owner_ref: context.owner_ref.clone(),
            owner_uid: context.owner_uid.clone(),
            template: ticket.template().clone(),
            identity: report.identity,
            uid: context.resource_uid.clone(),
            generation: context.resource_generation,
            controller_generation: context.controller_generation,
            execution_ref: spec.execution().execution_ref().clone(),
            target_ref: context.target_ref.clone(),
            runtime_scope: ticket.runtime_scope(),
        })?;
        if controller_bootstrap {
            let daemon_endpoint = match escrow_wait {
                Some(endpoint) => endpoint,
                None => {
                    self.cleanup_failed_resource_launch(
                        &context,
                        provider,
                        report.identity,
                        spec.execution().execution_ref(),
                    )
                    .await;
                    return Err("provider-controller-bootstrap-missing".to_owned());
                }
            };
            let daemon_endpoint =
                match wait_for_controller_bootstrap_endpoint(daemon_endpoint, timeout).await {
                    Ok(endpoint) => endpoint,
                    Err(error) => {
                        self.cleanup_failed_resource_launch(
                            &context,
                            provider,
                            report.identity,
                            spec.execution().execution_ref(),
                        )
                        .await;
                        return Err(error);
                    }
                };
            let controller_context = match ControllerBootstrapContext::from_resource_context(
                &context,
                spec.execution().execution_ref(),
                report.identity,
            ) {
                Ok(controller_context) => controller_context,
                Err(error) => {
                    let _ = self
                        .stop_provider_identity(provider, &report.identity, StopClass::Terminate)
                        .await;
                    let _ = match provider {
                        ManagedProvider::Minijail => {
                            self.minijail
                                .port()
                                .finalize_identity(&report.identity)
                                .await
                        }
                        ManagedProvider::Systemd => {
                            self.systemd
                                .port()
                                .finalize_identity(&report.identity)
                                .await
                        }
                    };
                    self.forget_resource_for_context(&context, spec.execution().execution_ref());
                    return Err(error);
                }
            };
                if let Err(error) = self.remember_controller_bootstrap(ControllerBootstrapEndpoint {
                daemon_endpoint,
                delivery_key_handoff,
                backend_lease,
                context: controller_context,
            }) {
                let _ = self
                    .stop_provider_identity(provider, &report.identity, StopClass::Terminate)
                    .await;
                let _ = match provider {
                    ManagedProvider::Minijail => {
                        self.minijail
                            .port()
                            .finalize_identity(&report.identity)
                            .await
                    }
                    ManagedProvider::Systemd => {
                        self.systemd
                            .port()
                            .finalize_identity(&report.identity)
                            .await
                    }
                };
                self.forget_resource_for_context(&context, spec.execution().execution_ref());
                return Err(error);
            }
        }
        Ok(ProviderLaunch {
            identity: report.identity,
        })
    }

    /// Launch one ephemeral Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn launch_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
        timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        self.validate_execution_target(spec.execution().execution_ref())?;
        let provider = managed_provider_from_ref(context.provider_ref)?;
        validate_resource_execution_target(self.mode, &context, spec.execution())?;
        let ticket = ephemeral_launch_ticket(
            &self.bundle,
            &self.socket_runtime_dir,
            &context,
            ExecutionIntent {
                execution: spec.execution(),
                activation_input: spec.activation_input(),
                spec_bytes: &serde_json::to_vec(spec)
                    .map_err(|_| "provider-ticket:serialization".to_owned())?,
                readiness: None,
            },
            provider,
            self.mode,
            timeout,
        )?;
        self.retire_resource_if_identity_changed(
            &context,
            provider,
            ticket.template(),
            spec.execution().execution_ref(),
            ticket.runtime_scope(),
        )
        .await?;
        let report = match provider {
            ManagedProvider::Minijail => self
                .minijail
                .launch(&ticket)
                .await
                .map_err(provider_error)?,
            ManagedProvider::Systemd => {
                self.systemd.launch(&ticket).await.map_err(provider_error)?
            }
        };
        self.remember_resource(ManagedResource {
            zone: context.zone.clone(),
            zone_uid: context.zone_uid.clone(),
            resource_ref: context.resource_ref.clone(),
            provider,
            provider_ref: context.provider_ref.clone(),
            provider_uid: context.provider_uid.clone(),
            provider_generation: context.provider_generation,
            owner_ref: context.owner_ref.clone(),
            owner_uid: context.owner_uid.clone(),
            template: ticket.template().clone(),
            identity: report.identity,
            uid: context.resource_uid.clone(),
            generation: context.resource_generation,
            controller_generation: context.controller_generation,
            execution_ref: spec.execution().execution_ref().clone(),
            target_ref: context.target_ref.clone(),
            runtime_scope: ticket.runtime_scope(),
        })?;
        Ok(ProviderLaunch {
            identity: report.identity,
        })
    }

    #[cfg(test)]
    pub(crate) fn attach_pending_controller_provider_context_for_test(
        &self,
        daemon_endpoint: OwnedFd,
        controller: (
            ZoneId,
            ResourceRef,
            ResourceUid,
            ResourceGeneration,
            ResourceRef,
            ControllerGeneration,
        ),
        provider: (ResourceRef, ResourceRef, ResourceUid, ResourceGeneration),
    ) -> Result<(), String> {
        let (zone, process_ref, process_uid, process_generation, execution_ref, controller_generation) =
            controller;
        let (process_provider_ref, provider_owner_ref, provider_uid, provider_generation) =
            provider;
        let context = ControllerBootstrapContext {
            zone: zone.clone(),
            zone_uid: None,
            process_ref: process_ref.clone(),
            process_uid,
            generation: process_generation,
            process_identity: ProcessIdentityDigest::from_bytes([7; 32]),
            process_provider_ref,
            provider_owner_ref,
            provider_uid,
            provider_generation,
            execution_ref,
            user_ref: None,
            controller_generation,
        };
        self.remember_controller_bootstrap(ControllerBootstrapEndpoint {
            daemon_endpoint,
            delivery_key_handoff: None,
            backend_lease: None,
            context,
        })
    }

    /// Adopt one durable Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn adopt_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.adopt_resource_with_execution(
            context,
            spec.execution(),
            None,
            &serde_json::to_vec(spec).map_err(|_| "provider-ticket:serialization".to_owned())?,
            Some(spec.readiness().class()),
        )
        .await
    }

    async fn stop_resource_identity_with_retry(
        &self,
        managed: &ManagedResource,
        class: StopClass,
        deadline: Instant,
    ) -> Result<(), String> {
        loop {
            match self.stop_resource_identity(managed, class).await {
                Ok(()) => return Ok(()),
                Err(error) if error == "pidfd-unavailable" || error == "process-vanished" => {
                    return Err(error);
                }
                Err(error) if retryable_stop_error(&error) && Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Adopt one ephemeral Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn adopt_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.adopt_resource_with_execution(
            context,
            spec.execution(),
            spec.activation_input(),
            &serde_json::to_vec(spec).map_err(|_| "provider-ticket:serialization".to_owned())?,
            None,
        )
        .await
    }

    /// Probe one durable Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn probe_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        let liveness = self
            .probe_resource_with_execution(
                &context,
                spec.execution(),
                None,
                &serde_json::to_vec(spec)
                    .map_err(|_| "provider-ticket:serialization".to_owned())?,
                Some(spec.readiness().class()),
            )
        .await?;
        if liveness == ProviderLiveness::Exited {
                self.finalize_resource(context.clone()).await?;
        }
        Ok(liveness)
    }

    /// Probe one ephemeral Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn probe_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        let liveness = self
            .probe_resource_with_execution(
                &context,
                spec.execution(),
                spec.activation_input(),
                &serde_json::to_vec(spec)
                    .map_err(|_| "provider-ticket:serialization".to_owned())?,
                None,
            )
        .await?;
        if liveness == ProviderLiveness::Exited {
                self.finalize_resource(context.clone()).await?;
        }
        Ok(liveness)
    }

    /// Stop one exact generic Process identity with the controller
    /// generation rehydrated from the owning Zone store.
    pub(crate) async fn stop_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.stop_resource_with_execution(
            context,
            ExecutionIntent {
                execution: spec.execution(),
                activation_input: None,
                spec_bytes: &serde_json::to_vec(spec)
                    .map_err(|_| "provider-ticket:serialization".to_owned())?,
                readiness: Some(spec.readiness().class()),
            },
            term_timeout,
            kill_timeout,
        )
        .await
    }

    /// Stop one exact generic EphemeralProcess identity with the controller
    /// generation rehydrated from the owning Zone store.
    pub(crate) async fn stop_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.stop_resource_with_execution(
            context,
            ExecutionIntent {
                execution: spec.execution(),
                activation_input: spec.activation_input(),
                spec_bytes: &serde_json::to_vec(spec)
                    .map_err(|_| "provider-ticket:serialization".to_owned())?,
                readiness: None,
            },
            term_timeout,
            kill_timeout,
        )
        .await
    }

    /// Finalize one terminal generic Process identity.
    pub(crate) async fn finalize_resource(
        &self,
        context: ProcessResourceContext<'_>,
    ) -> Result<(), String> {
        let Some(managed) = self
            .managed_resources
            .lock()
            .await
            .get(&(
                context.zone.clone(),
                context.zone_uid.clone(),
                context.resource_ref.clone(),
            ))
            .cloned()
        else {
            self.forget_controller_bootstrap_for_resource_context(&context);
            return Ok(());
        };
        let mismatches = resource_identity_mismatches(&managed, &context);
        if !mismatches.is_empty() {
            return Err(identity_changed_error(mismatches));
        }
        if !execution_target_allowed(execution_mode(self.mode), &managed.execution_ref) {
            return Err(GUEST_EXECUTION_UNAVAILABLE.to_owned());
        }
        self.forget_controller_bootstrap_for_context(
            &context,
            &managed.execution_ref,
            managed.identity,
        );
        let result = match managed.provider {
            ManagedProvider::Minijail => self
                .minijail
                .port()
                .finalize_identity(&managed.identity)
                .await
                .map_err(provider_error),
            ManagedProvider::Systemd => self
                .systemd
                .port()
                .finalize_identity(&managed.identity)
                .await
                .map_err(provider_error),
        };
        match result {
            Ok(()) => {
                self.forget_resource_for_context(&context, &managed.execution_ref);
                Ok(())
            }
            Err(error) if error == "process-vanished" => {
                self.forget_resource_for_context(&context, &managed.execution_ref);
                Ok(())
            }
            Err(error) if error == "pidfd-unavailable" => Err(error),
            Err(error) => Err(error),
        }
    }

    /// Return whether a specific Zone retains a verified resource identity.
    pub fn has_active_resource_in_zone(
        &self,
        zone: &ZoneId,
        zone_uid: Option<&ResourceUid>,
        resource_ref: &ResourceRef,
    ) -> bool {
        self.managed_resources
            .try_lock()
            .ok()
            .map(|managed| {
                managed.contains_key(&(zone.clone(), zone_uid.cloned(), resource_ref.clone()))
            })
            .unwrap_or(false)
    }

    /// Return whether a retained resource identity matches every Guest VMM
    /// fence supplied by the authoritative Resource API.
    pub(crate) fn resource_identity_is_active(
        &self,
        zone: &ZoneId,
        zone_uid: &ResourceUid,
        resource_ref: &ResourceRef,
        row: (&ResourceUid, ResourceGeneration),
        binding: (&ResourceRef, &ResourceRef, &ResourceUid),
        execution_ref: &ResourceRef,
    ) -> bool {
        let (resource_uid, generation) = row;
        let (provider_ref, owner_ref, owner_uid) = binding;
        self.managed_resources
            .try_lock()
            .ok()
            .and_then(|managed| {
                managed
                    .get(&(zone.clone(), Some(zone_uid.clone()), resource_ref.clone()))
                    .cloned()
            })
            .is_some_and(|managed| {
                managed.uid == *resource_uid
                    && managed.generation == generation
                    && managed.provider_ref == *provider_ref
                    && managed.owner_ref.as_ref() == Some(owner_ref)
                    && managed.owner_uid.as_ref() == Some(owner_uid)
                    && managed.execution_ref == *execution_ref
            })
    }

    pub(crate) fn has_controller_bootstrap(
        &self,
        resource_ref: &ResourceRef,
        context: &ControllerBootstrapContext,
    ) -> bool {
        let key = (context.zone.clone(), resource_ref.clone());
        self.controller_bootstrap
            .try_lock()
            .ok()
            .and_then(|markers| markers.get(&key).map(|marker| marker.context() == context))
            .unwrap_or(false)
    }

    fn forget_controller_bootstrap_for_context(
        &self,
        context: &ProcessResourceContext<'_>,
        execution_ref: &ResourceRef,
        process_identity: ProcessIdentityDigest,
    ) {
        let Some(owner_ref) = context.owner_ref.as_ref() else {
            return;
        };
        let key = (context.zone.clone(), context.resource_ref.clone());
        if let Ok(mut markers) = self.controller_bootstrap.try_lock()
            && markers.get(&key).is_some_and(|marker| {
                let marker_context = marker.context();
                marker_context.zone == context.zone
                    && marker_context.zone_uid.as_ref() == context.zone_uid.as_ref()
                    && marker_context.process_ref == *context.resource_ref
                    && marker_context.process_uid == *context.resource_uid
                    && marker_context.generation == context.resource_generation
                    && marker_context.process_identity == process_identity
                    && marker_context.process_provider_ref == *context.provider_ref
                    && marker_context.provider_owner_ref == *owner_ref
                    && context
                        .provider_uid
                        .as_ref()
                        .is_some_and(|uid| marker_context.provider_uid == *uid)
                    && context
                        .provider_generation
                        .is_some_and(|generation| marker_context.provider_generation == generation)
                    && marker_context.execution_ref == *execution_ref
                    && marker_context.user_ref.as_ref() == context.user_ref.as_ref()
                    && marker_context.controller_generation == context.controller_generation
            })
        {
            markers.remove(&key);
        }
    }

    pub(crate) fn forget_controller_bootstrap_for_resource_context(
        &self,
        context: &ProcessResourceContext<'_>,
    ) {
        let Some(owner_ref) = context.owner_ref.as_ref() else {
            return;
        };
        let key = (context.zone.clone(), context.resource_ref.clone());
        if let Ok(mut markers) = self.controller_bootstrap.try_lock()
            && markers.get(&key).is_some_and(|marker| {
                let marker_context = marker.context();
                marker_context.process_ref == *context.resource_ref
                    && marker_context.zone_uid.as_ref() == context.zone_uid.as_ref()
                    && marker_context.process_uid == *context.resource_uid
                    && marker_context.generation == context.resource_generation
                    && marker_context.process_provider_ref == *context.provider_ref
                    && marker_context.provider_owner_ref == *owner_ref
                    && context
                        .provider_uid
                        .as_ref()
                        .is_none_or(|uid| marker_context.provider_uid == *uid)
                    && context
                        .provider_generation
                        .is_none_or(|generation| marker_context.provider_generation == generation)
                    && context.user_ref.as_ref() == marker_context.user_ref.as_ref()
                    && marker_context.controller_generation == context.controller_generation
            })
        {
            markers.remove(&key);
        }
    }

    fn remember_controller_bootstrap(
        &self,
        endpoint: ControllerBootstrapEndpoint,
    ) -> Result<(), String> {
        let zone = endpoint.context.zone.clone();
        let process_ref = endpoint.context.process_ref.clone();
        let key = (endpoint.context.zone.clone(), process_ref);
        let mut markers = self
            .controller_bootstrap
            .try_lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?;
        if markers.len() >= MAX_CONTROLLER_BOOTSTRAP_ENDPOINTS && !markers.contains_key(&key) {
            return Err("provider-controller-bootstrap-capacity".to_owned());
        }
        if markers
            .get(&key)
            .is_some_and(|current| current.context().zone_uid != endpoint.context.zone_uid)
        {
            return Err("provider-controller-bootstrap-zone-identity-conflict".to_owned());
        }
        if markers
            .get(&key)
            .is_some_and(|current| current.context().generation() > endpoint.context.generation())
        {
            return Err("provider-controller-bootstrap-stale-generation".to_owned());
        }
        markers.insert(key, ControllerBootstrapMarker::Pending(endpoint));
        drop(markers);
        self.wake_controller_session_reconcile(&zone)
    }

    pub(crate) fn set_controller_session_waker(
        &self,
        zone: ZoneId,
        wake: ControllerSessionReconcileWake,
    ) -> Result<(), String> {
        self.controller_session_wakers
            .try_lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?
            .insert(zone.clone(), Arc::clone(&wake));
        let pending = self
            .controller_bootstrap
            .try_lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?
            .values()
            .any(|marker| {
                matches!(marker, ControllerBootstrapMarker::Pending(_))
                    && marker.context().zone() == &zone
            });
        if pending {
            self.wake_controller_session_reconcile(&zone)?;
        }
        Ok(())
    }

    fn wake_controller_session_reconcile(&self, zone: &ZoneId) -> Result<(), String> {
        // A missing waker is not a launch failure: the zone's controller-
        // session coordinator registers its waker during activation, which
        // can race a controller launch, and the registration path wakes any
        // marker already pending for the zone (`set_controller_session_waker`
        // reconciles pending markers itself).
        let wake = match self
            .controller_session_wakers
            .try_lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?
            .get(zone)
            .cloned()
        {
            Some(wake) => wake,
            None => {
                tracing::info!(
                    zone = %zone.as_str(),
                    "controller-session coordinator wake deferred: waker not registered yet"
                );
                return Ok(());
            }
        };
        wake().map_err(|error| format!("provider-controller-session-wake-failed:{error}"))
    }

    pub(crate) fn controller_bootstrap_present(
        &self,
        zone: &ZoneId,
        process_ref: &ResourceRef,
    ) -> bool {
        self.controller_bootstrap
            .try_lock()
            .ok()
            .map(|markers| markers.contains_key(&(zone.clone(), process_ref.clone())))
            .unwrap_or(false)
    }

    pub(crate) fn controller_bootstrap_ready(
        &self,
        zone: &ZoneId,
        process_ref: &ResourceRef,
    ) -> bool {
        let Ok(markers) = self.controller_bootstrap.try_lock() else {
            return false;
        };
        let Some(ControllerBootstrapMarker::Pending(endpoint)) =
            markers.get(&(zone.clone(), process_ref.clone()))
        else {
            return false;
        };
        use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
        let interests = PollFlags::POLLIN | PollFlags::POLLERR | PollFlags::POLLHUP;
        let mut descriptors = [PollFd::new(
            endpoint.daemon_endpoint.as_fd(),
            interests,
        )];
        matches!(poll(&mut descriptors, PollTimeout::ZERO), Ok(count) if count > 0)
            && descriptors[0]
                .revents()
                .is_some_and(|events| events.intersects(interests))
    }

    pub(crate) fn controller_bootstrap_contexts(
        &self,
        zone: &ZoneId,
    ) -> Vec<ControllerBootstrapContext> {
        self.controller_bootstrap
            .try_lock()
            .ok()
            .map(|markers| {
                markers
                    .iter()
                    .filter(|((marker_zone, _), _)| marker_zone == zone)
                    .map(|(_, marker)| marker.context().clone())
                    .collect()
            })
            .unwrap_or_default()
    }


    pub(crate) fn controller_bootstrap_establishing_contexts(
        &self,
        zone: &ZoneId,
    ) -> Vec<ControllerBootstrapContext> {
        self.controller_bootstrap
            .try_lock()
            .ok()
            .map(|markers| {
                markers
                    .values()
                    .filter_map(|marker| match marker {
                        ControllerBootstrapMarker::Establishing(context)
                            if context.zone() == zone =>
                        {
                            Some(context.clone())
                        }
                        ControllerBootstrapMarker::Pending(_)
                        | ControllerBootstrapMarker::Establishing(_)
                        | ControllerBootstrapMarker::Active(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn controller_peer_matches(
        &self,
        context: &ControllerBootstrapContext,
        peer_pid: i32,
    ) -> Result<bool, String> {
        if context.process_provider_ref().name().as_str() != "system-minijail" {
            return Ok(false);
        }
        self.minijail
            .port()
            .matches_peer_process(&context.process_identity(), peer_pid)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn begin_controller_bootstrap_if_matches(
        &self,
        zone: &ZoneId,
        expected: &ControllerBootstrapContext,
    ) -> Option<ControllerBootstrapEndpoint> {
        if expected.zone() != zone {
            return None;
        }
        let mut markers = self.controller_bootstrap.try_lock().ok()?;
        let key = (zone.clone(), expected.process_ref().clone());
        let marker = markers.remove(&key)?;
        match marker {
            ControllerBootstrapMarker::Pending(endpoint)
                if endpoint.context() == expected =>
            {
                let context = endpoint.context().clone();
                markers.insert(key, ControllerBootstrapMarker::Establishing(context));
                Some(endpoint)
            }
            ControllerBootstrapMarker::Pending(endpoint) => {
                markers.insert(key, ControllerBootstrapMarker::Pending(endpoint));
                None
            }
            ControllerBootstrapMarker::Establishing(context) => {
                markers.insert(key, ControllerBootstrapMarker::Establishing(context));
                None
            }
            ControllerBootstrapMarker::Active(context) => {
                markers.insert(key, ControllerBootstrapMarker::Active(context));
                None
            }
        }
    }

    pub(crate) fn activate_controller_bootstrap(
        &self,
        context: &ControllerBootstrapContext,
    ) -> bool {
        let Ok(mut markers) = self.controller_bootstrap.try_lock() else {
            return false;
        };
        if !matches!(
            markers.get(&(context.zone.clone(), context.process_ref().clone())),
            Some(ControllerBootstrapMarker::Establishing(current)) if *current == *context
        ) {
            return false;
        }
        markers.insert(
            (context.zone.clone(), context.process_ref().clone()),
            ControllerBootstrapMarker::Active(context.clone()),
        );
        true
    }

    /// Return a bootstrap whose establishment failed transiently to the
    /// Pending state, so the next reconcile pass retries with the same
    /// pre-armed socket. The controller retries its send for as long as
    /// it lives; without this, one failed receive orphans it forever.
    pub(crate) fn rearm_controller_bootstrap(
        &self,
        endpoint: ControllerBootstrapEndpoint,
    ) -> bool {
        let Ok(mut markers) = self.controller_bootstrap.try_lock() else {
            return false;
        };
        let key = (
            endpoint.context().zone().clone(),
            endpoint.context().process_ref().clone(),
        );
        if matches!(
            markers.get(&key),
            Some(ControllerBootstrapMarker::Establishing(current))
                if *current == *endpoint.context()
        ) {
            markers.insert(key, ControllerBootstrapMarker::Pending(endpoint));
            true
        } else {
            false
        }
    }

    pub(crate) fn fail_controller_bootstrap(&self, context: &ControllerBootstrapContext) -> bool {
        let Ok(mut markers) = self.controller_bootstrap.try_lock() else {
            return false;
        };
        if markers
            .get(&(context.zone.clone(), context.process_ref().clone()))
            .is_some_and(|marker| marker.context() == context)
        {
            markers.remove(&(context.zone.clone(), context.process_ref().clone()));
            true
        } else {
            false
        }
    }

    fn forget_resource_for_context(
        &self,
        context: &ProcessResourceContext<'_>,
        execution_ref: &ResourceRef,
    ) {
        let process_identity = self
            .managed_resources
            .try_lock()
            .ok()
            .and_then(|managed| {
                managed
                    .get(&(
                        context.zone.clone(),
                        context.zone_uid.clone(),
                        context.resource_ref.clone(),
                    ))
                    .cloned()
            })
            .filter(|managed| {
                resource_identity_matches(managed, context)
                    && managed.execution_ref == *execution_ref
            })
            .map(|managed| managed.identity);
        if let Some(process_identity) = process_identity {
            self.forget_controller_bootstrap_for_context(context, execution_ref, process_identity);
        } else {
            self.forget_controller_bootstrap_for_resource_context(context);
        }
        if let Ok(mut managed) = self.managed_resources.try_lock()
            && managed
                .get(&(
                    context.zone.clone(),
                    context.zone_uid.clone(),
                    context.resource_ref.clone(),
                ))
                .is_some_and(|resource| {
                    resource_identity_matches(resource, context)
                        && resource.execution_ref == *execution_ref
                })
        {
            managed.remove(&(
                context.zone.clone(),
                context.zone_uid.clone(),
                context.resource_ref.clone(),
            ));
        }
    }

    async fn adopt_resource_with_execution(
        &self,
        context: ProcessResourceContext<'_>,
        execution: &d2b_contracts_resource::v3::process::ExecutionSpec,
        activation_input: Option<&d2b_contracts_resource::v3::ActivationRunnerInput>,
        spec_bytes: &[u8],
        readiness: Option<ReadinessClass>,
    ) -> Result<ProviderAdoption, String> {
        self.validate_execution_target(execution.execution_ref())?;
        let provider = managed_provider_from_ref(context.provider_ref)?;
        validate_resource_execution_target(self.mode, &context, execution)?;
        let ticket = resource_ticket(
            &self.bundle,
            &context,
            ExecutionIntent {
                execution,
                activation_input,
                spec_bytes,
                readiness,
            },
            provider,
            self.mode,
            Duration::from_secs(30),
        )?;
        self.retire_resource_if_identity_changed(
            &context,
            provider,
            ticket.template(),
            execution.execution_ref(),
            ticket.runtime_scope(),
        )
        .await?;
        let outcome = match provider {
            ManagedProvider::Minijail => {
                self.minijail.adopt(&ticket).await.map_err(provider_error)?
            }
            ManagedProvider::Systemd => {
                self.systemd.adopt(&ticket).await.map_err(provider_error)?
            }
        };
        let controller_bootstrap = ticket.inherited_fd_table().count() != 0;
        match outcome {
            AdoptionOutcome::Absent => {
                self.finalize_resource(context.clone()).await?;
                Ok(ProviderAdoption::Absent)
            }
            AdoptionOutcome::Adopted(report) => {
                self.remember_resource(ManagedResource {
                    zone: context.zone.clone(),
                    zone_uid: context.zone_uid.clone(),
                    resource_ref: context.resource_ref.clone(),
                    provider,
                    provider_ref: context.provider_ref.clone(),
                    provider_uid: context.provider_uid.clone(),
                    provider_generation: context.provider_generation,
                    owner_ref: context.owner_ref.clone(),
                    owner_uid: context.owner_uid.clone(),
                    template: ticket.template().clone(),
                    identity: report.identity,
                    uid: context.resource_uid.clone(),
                    generation: context.resource_generation,
                    controller_generation: context.controller_generation,
                    execution_ref: execution.execution_ref().clone(),
                    target_ref: context.target_ref.clone(),
                    runtime_scope: ticket.runtime_scope(),
                })?;
                if controller_bootstrap {
                    let controller_context = ControllerBootstrapContext::from_resource_context(
                        &context,
                        execution.execution_ref(),
                        report.identity,
                    )?;
                    // A controller this daemon launched has already armed its
                    // bootstrap endpoint, and session establishment consumed
                    // it: there is nothing left to take. Re-taking here
                    // reported the bootstrap missing on every post-launch
                    // pass, so the driver stopped and relaunched the
                    // controller forever and its Process row never left the
                    // launch pass (vmCheck fixtures, 2026-09-11). Adoption
                    // converges on the marker this daemon already holds; the
                    // session fence owns validating it against the committed
                    // Provider identity.
                    if self.controller_bootstrap_present(&context.zone, context.resource_ref) {
                        return Ok(ProviderAdoption::Adopted(report));
                    }
                    let Some(daemon_endpoint) = self
                        .minijail
                        .port()
                        .take_controller_bootstrap(&report.identity)
                        .await
                        .map_err(provider_error)?
                    else {
                        return Ok(ProviderAdoption::ControllerBootstrapMissing);
                    };
                    let bootstrap_timeout =
                        Duration::from_millis(u64::from(ticket.operation().deadline_ms()));
                    let daemon_endpoint =
                        match wait_for_controller_bootstrap_endpoint(daemon_endpoint, bootstrap_timeout)
                            .await
                        {
                            Ok(endpoint) => endpoint,
                            Err(_) => return Ok(ProviderAdoption::ControllerBootstrapMissing),
                        };
                    if ticket.inherited_fd_table().count() == 2 {
                        return Ok(ProviderAdoption::ControllerBootstrapMissing);
                    }
                    self.remember_controller_bootstrap(ControllerBootstrapEndpoint {
                        daemon_endpoint,
                        delivery_key_handoff: None,
                        backend_lease: None,
                        context: controller_context,
                    })?;
                }
                Ok(ProviderAdoption::Adopted(report))
            }
            AdoptionOutcome::Stale { candidate } => {
                self.forget_resource_in_zone(
                    &context.zone,
                    context.zone_uid.as_ref(),
                    context.resource_ref,
                );
                Ok(ProviderAdoption::Stale { candidate })
            }
            AdoptionOutcome::Quarantined(report) => {
                self.forget_resource_in_zone(
                    &context.zone,
                    context.zone_uid.as_ref(),
                    context.resource_ref,
                );
                Ok(ProviderAdoption::Quarantined(report))
            }
        }
    }

    async fn probe_resource_with_execution(
        &self,
        context: &ProcessResourceContext<'_>,
        execution: &d2b_contracts_resource::v3::process::ExecutionSpec,
        activation_input: Option<&d2b_contracts_resource::v3::ActivationRunnerInput>,
        spec_bytes: &[u8],
        readiness: Option<ReadinessClass>,
    ) -> Result<ProviderLiveness, String> {
        self.validate_execution_target(execution.execution_ref())?;
        let provider = managed_provider_from_ref(context.provider_ref)?;
        validate_resource_execution_target(self.mode, context, execution)?;
        let ticket = resource_ticket(
            &self.bundle,
            context,
            ExecutionIntent {
                execution,
                activation_input,
                spec_bytes,
                readiness,
            },
            provider,
            self.mode,
            Duration::from_secs(30),
        )?;
        let candidate = match provider {
            ManagedProvider::Minijail => self
                .minijail
                .port()
                .probe(&ticket)
                .await
                .map_err(provider_error)?,
            ManagedProvider::Systemd => self
                .systemd
                .port()
                .probe(&ticket)
                .await
                .map_err(provider_error)?,
        };
        let Some(candidate) = candidate else {
            return Ok(ProviderLiveness::Exited);
        };
        let (expected_owner, required) = match provider {
            ManagedProvider::Minijail => (
                self.minijail.profile().wait_reap_owner(),
                self.minijail.profile().required_identity_bindings(),
            ),
            ManagedProvider::Systemd => (
                self.systemd.profile().wait_reap_owner(),
                self.systemd.profile().required_identity_bindings(),
            ),
        };
        if candidate.wait_reap_owner != expected_owner || candidate.validate(required).is_err() {
            Ok(ProviderLiveness::Unknown)
        } else {
            Ok(ProviderLiveness::Alive)
        }
    }

    async fn stop_resource_with_execution(
        &self,
        context: ProcessResourceContext<'_>,
        intent: ExecutionIntent<'_>,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        let ExecutionIntent {
            execution,
            activation_input,
            spec_bytes,
            readiness,
        } = intent;
        validate_resource_execution_target(self.mode, &context, execution)?;
        let provider = managed_provider_from_ref(context.provider_ref)?;
        let ticket = resource_ticket(
            &self.bundle,
            &context,
            ExecutionIntent {
                execution,
                activation_input,
                spec_bytes,
                readiness,
            },
            provider,
            self.mode,
            Duration::from_secs(30),
        )?;
        let managed = self
            .managed_resources
            .lock()
            .await
            .get(&(
                context.zone.clone(),
                context.zone_uid.clone(),
                context.resource_ref.clone(),
            ))
            .cloned()
            .ok_or_else(|| "provider-process-not-found".to_owned())?;
        let mut mismatches = resource_identity_mismatches(&managed, &context);
        if managed.provider != provider {
            mismatches.push(format!(
                "provider(managed={:?},requested={provider:?})",
                managed.provider
            ));
        }
        if managed.template != *ticket.template() {
            mismatches.push(format!(
                "template(managed={:?},requested={:?})",
                managed.template,
                ticket.template()
            ));
        }
        if managed.execution_ref != *execution.execution_ref() {
            mismatches.push(format!(
                "execution_ref(managed={:?},requested={:?})",
                managed.execution_ref,
                execution.execution_ref()
            ));
        }
        if managed.runtime_scope != ticket.runtime_scope() {
            mismatches.push(format!(
                "runtime_scope(managed={:?},requested={:?})",
                managed.runtime_scope,
                ticket.runtime_scope()
            ));
        }
        if !mismatches.is_empty() {
            return Err(identity_changed_error(mismatches));
        }
        match self
            .stop_resource_identity_with_retry(
                &managed,
                StopClass::Drain,
                Instant::now() + term_timeout,
            )
            .await
        {
            Ok(()) => {}
            Err(error) if error == "process-vanished" => {}
            Err(error) if error == "pidfd-unavailable" => return Err(error),
            Err(error) => return Err(error),
        }
        let deadline = Instant::now() + term_timeout;
        loop {
            match self
                .probe_resource_with_execution(
                    &context,
                    execution,
                    activation_input,
                    spec_bytes,
                    readiness,
                )
                .await?
            {
                ProviderLiveness::Exited => {
                    self.finalize_resource(context.clone()).await?;
                    return Ok(false);
                }
                ProviderLiveness::Alive => {}
                ProviderLiveness::Unknown if Instant::now() >= deadline => break,
                ProviderLiveness::Unknown => {}
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        match self
            .stop_resource_identity_with_retry(
                &managed,
                StopClass::Terminate,
                Instant::now() + kill_timeout,
            )
            .await
        {
            Ok(()) => {}
            Err(error) if error == "process-vanished" => {}
            Err(error) if error == "pidfd-unavailable" => return Err(error),
            Err(error) => return Err(error),
        }
        let kill_deadline = Instant::now() + kill_timeout;
        loop {
            match self
                .probe_resource_with_execution(
                    &context,
                    execution,
                    activation_input,
                    spec_bytes,
                    readiness,
                )
                .await?
            {
                ProviderLiveness::Exited => {
                    self.finalize_resource(context.clone()).await?;
                    return Ok(true);
                }
                ProviderLiveness::Alive | ProviderLiveness::Unknown => {}
            }
            if Instant::now() >= kill_deadline {
                return Err("provider-process-kill-timeout".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Adopt one trusted process node after a daemon restart.
    pub async fn adopt_node(
        &self,
        vm: &str,
        node: &ProcessNode,
    ) -> Result<ProviderAdoption, String> {
        if !Self::supports_node(node) {
            return Err("provider-node-unsupported".to_owned());
        }
        let ticket = self.ticket(vm, node)?;
        let outcome = match self.provider_for(node) {
            ManagedProvider::Minijail => {
                self.minijail.adopt(&ticket).await.map_err(provider_error)?
            }
            ManagedProvider::Systemd => {
                self.systemd.adopt(&ticket).await.map_err(provider_error)?
            }
        };
        match outcome {
            AdoptionOutcome::Absent => Ok(ProviderAdoption::Absent),
            AdoptionOutcome::Adopted(report) => {
                self.remember(vm, node, report.identity)?;
                Ok(ProviderAdoption::Adopted(report))
            }
            AdoptionOutcome::Stale { candidate } => {
                self.forget(vm, node);
                Ok(ProviderAdoption::Stale { candidate })
            }
            AdoptionOutcome::Quarantined(report) => {
                self.forget(vm, node);
                Ok(ProviderAdoption::Quarantined(report))
            }
        }
    }

    /// Probe one node through the Provider's authenticated read-only path.
    ///
    /// Unlike adoption, a liveness probe does not open a pidfd, retain a
    /// handle, or stage an observation for a later adoption call.
    pub async fn probe_node(
        &self,
        vm: &str,
        node: &ProcessNode,
    ) -> Result<ProviderLiveness, String> {
        if !Self::supports_node(node) {
            return Ok(ProviderLiveness::Unknown);
        }
        let ticket = self.ticket(vm, node)?;
        let candidate = match self.provider_for(node) {
            ManagedProvider::Minijail => self
                .minijail
                .port()
                .probe(&ticket)
                .await
                .map_err(provider_error)?,
            ManagedProvider::Systemd => self
                .systemd
                .port()
                .probe(&ticket)
                .await
                .map_err(provider_error)?,
        };
        let Some(candidate) = candidate else {
            return Ok(ProviderLiveness::Exited);
        };
        let expected_owner = match self.provider_for(node) {
            ManagedProvider::Minijail => self.minijail.profile().wait_reap_owner(),
            ManagedProvider::Systemd => self.systemd.profile().wait_reap_owner(),
        };
        let required = match self.provider_for(node) {
            ManagedProvider::Minijail => self.minijail.profile().required_identity_bindings(),
            ManagedProvider::Systemd => self.systemd.profile().required_identity_bindings(),
        };
        if candidate.wait_reap_owner != expected_owner || candidate.validate(required).is_err() {
            Ok(ProviderLiveness::Unknown)
        } else {
            Ok(ProviderLiveness::Alive)
        }
    }

    /// Observe a Provider-owned OneShot until it exits, then release its
    /// exact pidfd or service-manager identity.
    pub async fn wait_node(
        &self,
        vm: &str,
        node: &ProcessNode,
        timeout: Duration,
    ) -> Result<(), String> {
        if !Self::supports_node(node) {
            return Err("provider-node-unsupported".to_owned());
        }
        let deadline = Instant::now() + timeout;
        loop {
            match self.probe_node(vm, node).await? {
                ProviderLiveness::Exited => {
                    self.finalize_node(vm, node).await?;
                    return Ok(());
                }
                ProviderLiveness::Alive => {}
                ProviderLiveness::Unknown => {
                    return Err("provider-process-identity-ambiguous".to_owned());
                }
            }
            if Instant::now() >= deadline {
                return Err("provider-process-exit-timeout".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Stop one exact Provider identity, escalating after the drain budget.
    pub async fn stop_node(
        &self,
        vm: &str,
        node: &ProcessNode,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        if !Self::supports_node(node) {
            return Err("provider-node-unsupported".to_owned());
        }
        let key = (vm.to_owned(), Self::tracked_role_id(node));
        let managed = self
            .managed
            .lock()
            .await
            .get(&key)
            .copied()
            .ok_or_else(|| "provider-process-not-found".to_owned())?;
        match self.stop_identity(managed, StopClass::Drain).await {
            Ok(()) => {}
            Err(error) if error == "process-vanished" => {}
            Err(error) if error == "pidfd-unavailable" => return Err(error),
            Err(error) => return Err(error),
        }
        let deadline = Instant::now() + term_timeout;
        loop {
            match self.probe_node(vm, node).await? {
                ProviderLiveness::Exited => {
                    self.finalize_node(vm, node).await?;
                    return Ok(false);
                }
                ProviderLiveness::Alive => {}
                ProviderLiveness::Unknown => {
                    if Instant::now() >= deadline {
                        break;
                    }
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        match self.stop_identity(managed, StopClass::Terminate).await {
            Ok(()) => {}
            Err(error) if error == "process-vanished" => {}
            Err(error) if error == "pidfd-unavailable" => return Err(error),
            Err(error) => return Err(error),
        }
        let kill_deadline = Instant::now() + kill_timeout;
        loop {
            match self.probe_node(vm, node).await? {
                ProviderLiveness::Exited => {
                    self.finalize_node(vm, node).await?;
                    return Ok(true);
                }
                ProviderLiveness::Alive | ProviderLiveness::Unknown => {}
            }
            if Instant::now() >= kill_deadline {
                return Err("provider-process-kill-timeout".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Finalize a terminal Provider process and remove its local authority.
    pub async fn finalize_node(&self, vm: &str, node: &ProcessNode) -> Result<(), String> {
        if !Self::supports_node(node) {
            return Err("provider-node-unsupported".to_owned());
        }
        let key = (vm.to_owned(), Self::tracked_role_id(node));
        let Some(managed) = self
            .managed
            .lock()
            .await
            .get(&key)
            .copied()
        else {
            return Ok(());
        };
        let result = match managed.provider {
            ManagedProvider::Minijail => self
                .minijail
                .port()
                .finalize_identity(&managed.identity)
                .await
                .map_err(provider_error),
            ManagedProvider::Systemd => self
                .systemd
                .port()
                .finalize_identity(&managed.identity)
                .await
                .map_err(provider_error),
        };
        match result {
            Ok(()) => {
                self.forget(vm, node);
                Ok(())
            }
            Err(error) if error == "process-vanished" => {
                self.forget(vm, node);
                Ok(())
            }
            Err(error) if error == "pidfd-unavailable" => Err(error),
            Err(error) => Err(error),
        }
    }

    /// Adopt only the long-lived process roles authorized by durable
    /// lifecycle snapshots for one VM.
    pub async fn adopt_vm(
        &self,
        vm: &str,
        eligible_roles: &BTreeSet<String>,
    ) -> Result<(), String> {
        let Some(dag) = self.bundle.find_process_vm(vm) else {
            return Ok(());
        };
        for node in dag
            .nodes
            .iter()
            .filter(|node| Self::is_long_lived(node))
            .filter(|node| eligible_roles.contains(&Self::tracked_role_id(node)))
        {
            match self.adopt_node(vm, node).await? {
                ProviderAdoption::Absent => {
                    self.forget(vm, node);
                }
                ProviderAdoption::Adopted(_) => {}
                ProviderAdoption::ControllerBootstrapMissing => {
                    tracing::warn!(
                        vm = %vm,
                        role = %Self::tracked_role_id(node),
                        "Provider startup adoption found a controller without bootstrap"
                    );
                }
                ProviderAdoption::Stale { .. } => {
                    tracing::warn!(
                        vm = %vm,
                        role = %Self::tracked_role_id(node),
                        "Provider startup adoption found a stale process"
                    );
                }
                ProviderAdoption::Quarantined(_) => {
                    tracing::warn!(
                        vm = %vm,
                        role = %Self::tracked_role_id(node),
                        "Provider startup adoption quarantined an ambiguous process"
                    );
                }
            }
        }
        Ok(())
    }

    fn provider_for(&self, node: &ProcessNode) -> ManagedProvider {
        if node.unit.is_some() {
            ManagedProvider::Systemd
        } else {
            ManagedProvider::Minijail
        }
    }

    fn remember(
        &self,
        vm: &str,
        node: &ProcessNode,
        identity: ProcessIdentityDigest,
    ) -> Result<(), String> {
        self.managed
            .try_lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?
            .insert(
                (vm.to_owned(), Self::tracked_role_id(node)),
                ManagedProcess {
                    provider: self.provider_for(node),
                    identity,
                },
            );
        Ok(())
    }

    fn remember_resource(
        &self,
        managed: ManagedResource,
    ) -> Result<(), String> {
        self.managed_resources
            .try_lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?
            .insert(
                (
                    managed.zone.clone(),
                    managed.zone_uid.clone(),
                    managed.resource_ref.clone(),
                ),
                managed,
            );
        Ok(())
    }

    fn forget(&self, vm: &str, node: &ProcessNode) {
        if let Ok(mut managed) = self.managed.try_lock() {
            managed.remove(&(vm.to_owned(), Self::tracked_role_id(node)));
        }
    }

    fn forget_resource_in_zone(
        &self,
        zone: &ZoneId,
        zone_uid: Option<&ResourceUid>,
        resource_ref: &ResourceRef,
    ) {
        if let Ok(mut markers) = self.controller_bootstrap.try_lock() {
            markers.retain(|(marker_zone, marker_ref), marker| {
                marker_zone != zone
                    || marker_ref != resource_ref
                    || marker.context().zone_uid.as_ref() != zone_uid
            });
        }
        if let Ok(mut managed) = self.managed_resources.try_lock() {
            managed.remove(&(zone.clone(), zone_uid.cloned(), resource_ref.clone()));
        }
    }

    async fn stop_provider_identity(
        &self,
        provider: ManagedProvider,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> Result<(), String> {
        match provider {
            ManagedProvider::Minijail => self
                .minijail
                .stop(identity, class)
                .await
                .map_err(provider_error),
            ManagedProvider::Systemd => self
                .systemd
                .stop(identity, class)
                .await
                .map_err(provider_error),
        }
    }

    pub(crate) async fn stop_stale_resource(
        &self,
        provider_ref: &ResourceRef,
        candidate: &AdoptionCandidate,
    ) -> Result<(), String> {
        let provider = managed_provider_from_ref(provider_ref)?;
        match provider {
            ManagedProvider::Minijail => self
                .minijail
                .stop_stale(candidate)
                .await
                .map_err(provider_error)?,
            ManagedProvider::Systemd => self
                .systemd
                .stop_stale(candidate)
                .await
                .map_err(provider_error)?,
        }
        let finalized = match provider {
            ManagedProvider::Minijail => self
                .minijail
                .port()
                .finalize_identity(&candidate.identity)
                .await
                .map_err(provider_error),
            ManagedProvider::Systemd => self
                .systemd
                .port()
                .finalize_identity(&candidate.identity)
                .await
                .map_err(provider_error),
        };
        match finalized {
            Ok(()) => Ok(()),
            Err(error) if error == "process-vanished" => Ok(()),
            Err(error) if error == "pidfd-unavailable" => Err(error),
            Err(error) => Err(error),
        }
    }

    async fn stop_identity(&self, managed: ManagedProcess, class: StopClass) -> Result<(), String> {
        self.stop_provider_identity(managed.provider, &managed.identity, class)
            .await
    }

    async fn stop_resource_identity(
        &self,
        managed: &ManagedResource,
        class: StopClass,
    ) -> Result<(), String> {
        self.stop_provider_identity(managed.provider, &managed.identity, class)
            .await
    }

    async fn retire_managed_resource(&self, managed: &ManagedResource) -> Result<(), String> {
        match self
            .stop_resource_identity_with_retry(
                managed,
                StopClass::Drain,
                Instant::now() + Duration::from_secs(30),
            )
            .await
        {
            Ok(()) => {}
            Err(error) if error == "process-vanished" => {}
            Err(error) if error == "pidfd-unavailable" => return Err(error),
            Err(error) if retryable_stop_error(&error) => {
                self.stop_resource_identity_with_retry(
                    managed,
                    StopClass::Terminate,
                    Instant::now() + Duration::from_secs(30),
                )
                .await
                .or_else(|error| {
                    if error == "process-vanished" {
                        Ok(())
                    } else {
                        Err(error)
                    }
                })?;
            }
            Err(error) => return Err(error),
        }
        let finalized = match managed.provider {
            ManagedProvider::Minijail => self
                .minijail
                .port()
                .finalize_identity(&managed.identity)
                .await
                .map_err(provider_error),
            ManagedProvider::Systemd => self
                .systemd
                .port()
                .finalize_identity(&managed.identity)
                .await
                .map_err(provider_error),
        };
        match finalized {
            Ok(()) => Ok(()),
            Err(error) if error == "process-vanished" => Ok(()),
            Err(error) if error == "pidfd-unavailable" => Err(error),
            Err(error) => Err(error),
        }
    }

    async fn retire_resource_if_identity_changed(
        &self,
        context: &ProcessResourceContext<'_>,
        provider: ManagedProvider,
        template: &BoundedToken,
        execution_ref: &ResourceRef,
        runtime_scope: Option<ConfigurationDigest>,
    ) -> Result<(), String> {
        let managed = self
            .managed_resources
            .lock()
            .await
            .values()
            .filter(|managed| {
                managed.zone == context.zone && managed.resource_ref == *context.resource_ref
            })
            .cloned()
            .collect::<Vec<_>>();
        for managed in managed {
            // U13/U6 restart adoption: this pass destroys a live process, so
            // only a *proven* identity change retires it. A per-pass resolved
            // input the current context cannot resolve is unknown, never a
            // change (the manager-served Guest's VMM was retired and relaunched
            // on every daemon restart when the recovery pass resolved the
            // owning Guest's uid after the adopt that seeded the entry).
            let mut mismatches = resource_identity_changes(&managed, context);
            if managed.provider != provider {
                mismatches.push(format!(
                    "provider(managed={:?},requested={provider:?})",
                    managed.provider
                ));
            }
            if managed.template != *template {
                mismatches.push(format!(
                    "template(managed={:?},requested={template:?})",
                    managed.template
                ));
            }
            if managed.execution_ref != *execution_ref {
                mismatches.push(format!(
                    "execution_ref(managed={:?},requested={execution_ref:?})",
                    managed.execution_ref
                ));
            }
            if let (Some(managed_scope), Some(requested_scope)) =
                (managed.runtime_scope.as_ref(), runtime_scope.as_ref())
                && managed_scope != requested_scope
            {
                mismatches.push(format!(
                    "runtime_scope(managed={managed_scope:?},requested={requested_scope:?})"
                ));
            }
            if mismatches.is_empty() {
                continue;
            }
            tracing::warn!(
                resource = %managed.resource_ref.to_canonical_string(),
                identity = %managed.identity.to_hex(),
                mismatches = ?mismatches,
                "retiring a managed process whose identity changed"
            );
            self.retire_managed_resource(&managed).await?;
            self.forget_resource_in_zone(
                &managed.zone,
                managed.zone_uid.as_ref(),
                &managed.resource_ref,
            );
        }
        Ok(())
    }

    fn ticket(&self, vm: &str, node: &ProcessNode) -> Result<LaunchTicket, String> {
        self.ticket_with_timeout(vm, node, Duration::from_secs(30))
    }

    fn ticket_with_timeout(
        &self,
        vm: &str,
        node: &ProcessNode,
        timeout: Duration,
    ) -> Result<LaunchTicket, String> {
        build_ticket(&self.bundle, vm, node, self.provider_for(node), timeout)
            .map_err(|error| format!("provider-ticket:{}", error.code()))
    }
/// Resolve the typed launch parameters of one declared Device-owned worker
/// row (`U17` gap closure).
///
/// The Device Providers declare their worker rows path-free and the
/// Process spec is argv-free by contract, so the inputs the device argv
/// generators need come from the three sources this seat owns:
///
/// - the owned `Device` row: its uid keys the controller-created state
///   Volume, and its declared Provider settings carry the GPU context
///   classes, displays, and EGL/Vulkan flags;
/// - the trusted declared template: the bundle's Device-worker intent for
///   this exact declared row pins the worker binary and the principal the
///   sockets belong to, and `device_worker_posture` pins the template's
///   closed role posture;
/// - the daemon's runtime paths: the swtpm state directory backing the
///   controller-created state Volume, and the per-VM socket roots under
///   the daemon runtime root (the same conventions the guest VMM's
///   `--tpm socket=` / `--gpu socket=` / `--vhost-user-media socket=`
///   arguments name) - plus the bundle's `site.json`, which projects the
///   host Wayland socket the GPU worker renders into.
///
/// Returns `None` for every row that is not one of the declared Device
/// worker templates. A declared template whose trusted inputs cannot be
/// resolved refuses the launch (the named code is the diagnosis) instead
/// of launching bare.
pub(crate) async fn resolve_device_worker_launch(
    &self,
    ctx: &mut ResourceContext,
    identity: &ProcessResourceIdentity,
    spec: &ProcessFamilySpec,
) -> Result<Option<DeviceWorkerLaunch>, &'static str> {
    let execution = spec.execution();
    let template = execution.template().as_str();
    let family = device_worker_family(template);
    let Some(family) = family else {
        return Ok(None);
    };
    let launch = &identity.launch;
    // Owner fence: these rows are Device-declared children of the Device
    // that owns the physical function, and every path below is derived
    // from that Device.
    let owner_key = ctx
        .owner_key()
        .cloned()
        .ok_or("device-worker-owner-unresolved")?;
    if owner_key.type_name != "Device" {
        return Err("device-worker-owner-not-device");
    }
    let device_ref = launch
        .owner_ref()
        .filter(|owner| owner.resource_type().as_str() == "Device")
        .cloned()
        .ok_or("device-worker-owner-not-device")?;
    let device_uid = ctx
        .owner()
        .and_then(resource_uid_from_bytes)
        .ok_or("device-worker-owner-uid-unresolved")?;
    // A Device-owned worker row declares `executionRef Host/host-system`
    // and no Guest target, so the row's own launch identity names no VM
    // by construction. The coherent VM scope is the owning Device's
    // Guest owner - the same derivation `tpm_device_targets_vm` requires
    // (`Device.metadata.ownerRef == Guest/<vm>`) and the TPM
    // shared-provider effects mint their `VmId` from. A Device with no
    // Guest owner is the genuinely unresolvable case.
    let vm_name = match launch.vm() {
        Some(vm) => vm.to_owned(),
        None => device_worker_vm(ctx, &owner_key).await?,
    };
    // Template fence: the trusted intent must exist for this exact
    // declared row name + template, and the template must belong to a
    // Device Provider's closed posture table.
    let execution_ref = execution.execution_ref().to_canonical_string();
    let user_ref = execution.user_ref().map(ResourceRef::to_canonical_string);
    let domain = match execution
        .domain()
        .unwrap_or(ExecutionDomain::System)
    {
        ExecutionDomain::System => ProcessExecutionDomain::System,
        ExecutionDomain::User => ProcessExecutionDomain::User,
    };
    let intent = self
        .bundle()
        .find_device_worker_intent(
            &identity.resource_ref,
            &execution_ref,
            domain,
            user_ref.as_deref(),
            template,
        )
        .ok_or("device-worker-intent-unresolved")?;
    let Some(posture) = d2b_core::bundle_resolver::device_worker_posture(
        intent.owner_ref.as_deref().unwrap_or_default(),
        template,
    ) else {
        return Err("device-worker-template-refused");
    };
    if !intent.accepts_launch_args {
        return Err("device-worker-template-refused");
    }
    // The socket owner ids the worker asks swtpm for, in the namespace
    // the launch actually runs in: a posture with the ADR 0021
    // single-entry user namespace names the in-namespace identity (`0`,
    // the only id the mapping declares), a posture without one keeps the
    // host principal. Naming the host principal inside its own namespace
    // made swtpm's socket chown fail with EINVAL and the worker exit 1
    // before it bound anything.
    let (socket_uid, socket_gid) = posture.launch_ids(intent.uid, intent.gid);
    let socket_runtime_dir = self.socket_runtime_dir().to_path_buf();
    let params = match family {
        DeviceWorkerFamily::Swtpm => {
            let state_dir = device_state_dir(
                self.bundle(),
                &identity.zone,
                &device_uid,
                &device_ref,
                &execution_ref,
                &vm_name,
            )?;
            DeviceWorkerLaunch::Swtpm(Box::new(SwtpmWorkerParams {
                binary_path: intent.binary_path.clone(),
                vm_name: vm_name.clone(),
                ctrl_socket_path: state_dir.join("ctrl.sock"),
                server_socket_path: device_runtime_socket(
                    &socket_runtime_dir,
                    &vm_name,
                    "tpm.sock",
                ),
                state_dir,
                uid: socket_uid,
                gid: socket_gid,
                log_level: d2b_provider_device_tpm::SwtpmSettings::default().log_level,
            }))
        }
        DeviceWorkerFamily::SwtpmFlush => {
            let state_dir = device_state_dir(
                self.bundle(),
                &identity.zone,
                &device_uid,
                &device_ref,
                &execution_ref,
                &vm_name,
            )?;
            DeviceWorkerLaunch::SwtpmFlush(Box::new(SwtpmFlushParams {
                ioctl_binary_path: intent.binary_path.clone(),
                vm_name: vm_name.clone(),
                ctrl_socket_path: state_dir.join("ctrl.sock"),
            }))
        }
        DeviceWorkerFamily::Gpu => {
            let settings = device_gpu_settings(ctx, &owner_key).await?;
            // The Wayland socket the sidecar renders into is projected by
            // the trusted bundle from the site's own Wayland session
            // (`d2b.site.waylandUser` / `waylandDisplay`, see
            // `nixos-modules/site-json.nix`). A bundle without the
            // artifact, or a headless site, leaves the slot unbound and
            // the launch refuses with its own code instead of naming a
            // path no trusted artifact names.
            let wayland_sock = gpu_worker_wayland_sock(self.bundle().site.as_ref())?;
            // The typed parameters travel as the canonical JSON of the
            // Provider's own `GpuParams`; the argv seat decodes them back.
            let params = serde_json::to_value(d2b_provider_device_gpu::GpuParams {
                context_types: settings
                    .context_types
                    .iter()
                    .map(|context| match context {
                        d2b_provider_device_gpu::ContextType::Virgl => {
                            d2b_provider_device_gpu::GpuContextType::Virgl
                        }
                        d2b_provider_device_gpu::ContextType::Virgl2 => {
                            d2b_provider_device_gpu::GpuContextType::Virgl2
                        }
                        d2b_provider_device_gpu::ContextType::CrossDomain => {
                            d2b_provider_device_gpu::GpuContextType::CrossDomain
                        }
                    })
                    .collect(),
                displays: settings
                    .displays
                    .iter()
                    .map(|display| d2b_provider_device_gpu::GpuDisplayConfig {
                        hidden: display.hidden,
                    })
                    .collect(),
                egl: settings.egl,
                vulkan: settings.vulkan,
            })
            .map_err(|_| "device-worker-gpu-settings-invalid")?;
            DeviceWorkerLaunch::Gpu(Box::new(GpuWorkerParams {
                binary_path: intent.binary_path.clone(),
                vm_name: vm_name.clone(),
                socket_path: device_runtime_socket(&socket_runtime_dir, &vm_name, "gpu.sock"),
                wayland_sock,
                params,
            }))
        }
        DeviceWorkerFamily::Video => {
            // The declared row's template and the owning Device's
            // `videoNvidiaDecode` setting are one decision (the posture
            // binds the NVIDIA nodes only through the
            // `video-worker-nvidia` template), so a disagreement is a
            // refusal rather than a launch where the setting is silently
            // ignored.
            let settings = device_gpu_settings(ctx, &owner_key).await?;
            video_nvidia_posture(template, &settings)?;
            DeviceWorkerLaunch::Video(Box::new(VideoWorkerParams {
                binary_path: intent.binary_path.clone(),
                vm_name: vm_name.clone(),
                socket_path: video_runtime_socket(&socket_runtime_dir, &vm_name)
                    .ok_or("device-worker-video-socket-unresolved")?,
            }))
        }
    };
    Ok(Some(params))
}

}

fn provider_error(error: ProcessConformanceError) -> String {
    error.code().to_owned()
}

fn caller_uid(caller: &BrokerCallerRole) -> u32 {
    match caller {
        BrokerCallerRole::AdminUid { uid }
        | BrokerCallerRole::LauncherUid { uid }
        | BrokerCallerRole::RootUid { uid }
        | BrokerCallerRole::HostShutdownUid { uid } => *uid,
        BrokerCallerRole::NotAuthorized => 0,
    }
}

fn managed_provider_from_ref(provider_ref: &ResourceRef) -> Result<ManagedProvider, String> {
    match provider_ref.name().as_str() {
        "system-minijail" => Ok(ManagedProvider::Minijail),
        "system-systemd" => Ok(ManagedProvider::Systemd),
        _ => Err("provider-ticket:unsupported-provider".to_owned()),
    }
}

fn is_credential_provider_ref(provider_ref: &ResourceRef) -> bool {
    provider_ref.resource_type().as_str() == "Provider"
        && matches!(
            provider_ref.name().as_str(),
            SECRET_SERVICE_BACKEND_REF | ENTRA_BACKEND_REF | MANAGED_IDENTITY_BACKEND_REF
        )
}

fn is_credential_agent_context(
    context: &ProcessResourceContext<'_>,
    execution: &d2b_contracts_resource::v3::process::ExecutionSpec,
) -> bool {
    context.provider_ref.name().as_str() == "system-minijail"
        && context
            .owner_ref
            .as_ref()
            .is_some_and(|owner| owner.resource_type().as_str() == "Credential")
        && context.resource_ref.resource_type().as_str() == "Process"
        && context.resource_ref.name().as_str().starts_with("mi-agent-")
        && execution.template().as_str() == d2b_provider_credential::CREDENTIAL_AGENT_BINARY
        && context.controller_provider_ref.as_ref().is_some_and(|provider| {
            provider.to_canonical_string() == MANAGED_IDENTITY_PROVIDER_REF
        })
}

/// Map the daemon's own mode onto the Process family's execution domain.
///
/// The family declares the Host/Guest vocabulary it admits and never reads
/// the daemon's mode; this is the seat where the daemon's mode crosses into
/// that vocabulary.
pub(crate) const fn execution_mode(mode: DaemonMode) -> ExecutionMode {
    match mode {
        DaemonMode::Host => ExecutionMode::Host,
        DaemonMode::Guest => ExecutionMode::Guest,
    }
}

fn validate_resource_execution_target(
    mode: DaemonMode,
    context: &ProcessResourceContext<'_>,
    execution: &d2b_contracts_resource::v3::process::ExecutionSpec,
) -> Result<(), String> {
    // A Host daemon has no authenticated cross-target Process session yet.
    // Reject Guest refs before ticket construction so they cannot fall
    // through to the Host broker's local systemd/minijail adapters.
    let execution_ref = execution.execution_ref();
    if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
        return Err("provider-ticket:invalid-execution-ref".to_owned());
    }
    if !execution_target_allowed(execution_mode(mode), execution_ref) {
        return Err(match mode {
            DaemonMode::Host => GUEST_EXECUTION_UNAVAILABLE,
            DaemonMode::Guest => "provider-ticket:host-execution-denied",
        }
        .to_owned());
    }
    if execution_ref.resource_type().as_str() == "Host"
        && let Some(target) = context.target_ref.as_ref()
        && target.resource_type().as_str() != "Guest"
    {
        return Err("provider-ticket:invalid-target".to_owned());
    }
    Ok(())
}

/// Compose one binding-owned serving worker's launch arguments.
///
/// The executable is never named here: the trusted `virtiofsd-worker`
/// template pins it, and the broker composes `argv[0]` from it. These
/// arguments carry the per-binding data the binding controller declared
/// (private socket path, served view root, thread pool, cache, flags).
async fn serving_worker_launch_args(
    bundle: &BundleResolver,
    socket_runtime_dir: &std::path::Path,
    zone: &ZoneId,
    launch: &ServingWorkerLaunch,
) -> Result<Vec<String>, String> {
    let zone_token = BoundedToken::parse(zone.as_str().to_owned())
        .map_err(|_| "provider-ticket:serving-zone-invalid".to_owned())?;
    let socket_path = crate::resource_plane_v3::serving_socket_path(
        socket_runtime_dir,
        &zone_token,
        &launch.volume_ref,
        &launch.guest_ref,
    )
    .ok_or_else(|| "provider-ticket:serving-socket-path-unresolved".to_owned())?;
    let root = launch
        .root
        .as_ref()
        .ok_or_else(|| "provider-ticket:serving-view-root-unsupported".to_owned())?;
    let shared_dir = match root {
        ServingWorkerRoot::StoragePath(storage_path_id) => bundle
            .resolve_volume_view_root(
                storage_path_id,
                launch.volume_ref.name().as_str(),
                &launch.view_path,
            )
            .ok_or_else(|| "provider-ticket:serving-view-root-unresolved".to_owned())?,
        // A closure-sourced Volume is served out of the broker-managed
        // per-Guest store-view farm (`store-view/live`; the preserved
        // `ro-store` redirect), never out of a bundle-declared storage path.
        ServingWorkerRoot::StoreViewFarm => {
            let intent = bundle
                .find_store_view_intent_for_zone(zone, launch.guest_ref.name().as_str())
                .ok_or_else(|| "provider-ticket:serving-store-view-intent-unresolved".to_owned())?;
            if launch.view_path.starts_with('/') {
                return Err("provider-ticket:serving-view-path-invalid".to_owned());
            }
            let mut farm = intent.hardlink_farm_path.clone();
            if !launch.view_path.is_empty() {
                farm.push(&launch.view_path);
            }
            farm
        }
    };
    if !shared_dir.is_absolute()
        || shared_dir
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("provider-ticket:serving-view-root-unresolved".to_owned());
    }
    // The worker binds its private socket as the in-namespace principal; the
    // directory is realized before the launch (old-plane runtime-dir prep).
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| "provider-ticket:serving-socket-dir-create".to_owned())?;
        use std::os::unix::fs::PermissionsExt as _;
        if let Err(error) = tokio::fs::set_permissions(
            parent,
            std::fs::Permissions::from_mode(0o700),
        ).await {
            tracing::warn!(
                zone = %zone,
                socket_dir = %parent.display(),
                error = %error,
                "failed to enforce 0700 on the serving worker socket directory"
            );
        }
    }
    let cache = match launch.cache {
        AttachmentCache::Auto => "auto",
        AttachmentCache::Always => "always",
        AttachmentCache::Never => "never",
    };
    let mut args = Vec::with_capacity(11);
    args.push(format!("--socket-path={}", socket_path.display()));
    if let Some(group) = launch.socket_group.as_deref() {
        args.push(format!("--socket-group={group}"));
    }
    args.push(format!("--shared-dir={}", shared_dir.display()));
    args.push(format!(
        "--thread-pool-size={}",
        launch.thread_pool_size.max(1)
    ));
    if launch.posix_acl {
        args.push("--posix-acl".to_owned());
    }
    if launch.xattr {
        args.push("--xattr".to_owned());
    }
    args.push(format!("--cache={cache}"));
    args.push("--sandbox=chroot".to_owned());
    args.push("--inode-file-handles=never".to_owned());
    if launch.access == AttachmentAccess::ReadOnly {
        args.push("--readonly".to_owned());
    }
    Ok(args)
}

/// One derived device-worker path the daemon owns: absolute, free of `..`,
/// and - when an anchor is given - contained by it.
fn device_worker_path(
    path: &std::path::Path,
    anchor: Option<&std::path::Path>,
    which: &str,
) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!("provider-ticket:device-worker-path-invalid:{which}"));
    }
    if let Some(anchor) = anchor
        && !path.starts_with(anchor)
    {
        return Err(format!("provider-ticket:device-worker-path-unanchored:{which}"));
    }
    Ok(())
}

/// The state directory backing the Device's controller-created TPM state
/// Volume: the controller-created Volume's name under the trusted per-VM
/// `path:swtpm-state:<vm>` storage row
/// (`packages/d2b-provider-volume-local/nix/storage-json.nix`). Both the
/// name and the root come from trusted artifacts - the Volume body is the
/// TPM Provider's own builder, and the root is the bundle's storage row -
/// so a worker can never be pointed at a path no trusted artifact names.
fn device_state_dir(
    bundle: &BundleResolver,
    zone: &ZoneId,
    device_uid: &ResourceUid,
    device_ref: &ResourceRef,
    execution_ref: &str,
    vm_name: &str,
) -> Result<PathBuf, &'static str> {
    let execution_ref =
        ResourceRef::parse(execution_ref).map_err(|_| "device-worker-execution-ref-invalid")?;
    let document = d2b_provider_device_tpm::build_tpm_state_volume_resource(
        device_uid,
        device_ref,
        zone.as_str(),
        &execution_ref,
    )
    .map_err(|_| "device-worker-state-volume-unresolved")?;
    let volume_name = document
        .pointer("/metadata/name")
        .and_then(serde_json::Value::as_str)
        .ok_or("device-worker-state-volume-unresolved")?;
    let storage_path_id = format!(
        "{}{vm_name}",
        d2b_provider_device_tpm::vocabulary::TPM_STATE_STORAGE_ROW_PREFIX
    );
    bundle
        .resolve_volume_view_root(&storage_path_id, volume_name, "")
        .ok_or("device-worker-state-dir-unresolved")
}

/// Decode the GPU settings declared by one Device row's stored spec.
///
/// Only an absent Provider extension decodes to the Provider's bounded
/// default. A present settings payload that does not decode as the closed
/// `device-gpu.d2bus.org` extension refuses with its own code instead: folding
/// it into the default made an undecodable declaration indistinguishable from
/// a Device that declares nothing, and the default's context classes
/// (including `CrossDomain`) are wider than anything the Device declared.
fn decode_device_gpu_settings(
    stored_spec: &[u8],
) -> Result<d2b_provider_device_gpu::GpuSettings, &'static str> {
    let envelope = serde_json::from_slice::<ResourceSpec>(stored_spec)
        .map_err(|_| "device-worker-device-row-unreadable")?;
    let Some(provider) = envelope.provider() else {
        return Ok(d2b_provider_device_gpu::GpuSettings::default());
    };
    let settings = provider.settings().to_canonical_bytes();
    serde_json::from_slice::<d2b_provider_device_gpu::GpuSettings>(&settings)
        .map_err(|_| "device-worker-gpu-settings-invalid")
}

/// The owning Device's declared GPU settings (the closed
/// `device-gpu.d2bus.org` Device extension); a Device that declares none
/// keeps the Provider's own bounded default.
async fn device_gpu_settings(
    ctx: &mut ResourceContext,
    owner_key: &ResourceKey,
) -> Result<d2b_provider_device_gpu::GpuSettings, &'static str> {
    let Some(row) = ctx
        .get(owner_key)
        .await
        .map_err(|_| "device-worker-device-row-unreadable")?
    else {
        return Err("device-worker-device-row-missing");
    };
    decode_device_gpu_settings(&row.spec)
}

/// The video sidecar posture fence: the declared row's template and the
/// owning Device's `videoNvidiaDecode` setting are one decision, so they must
/// agree. The NVIDIA template with the setting off (a stale or hand-authored
/// row) and the plain template with the setting on (the setting silently
/// dropped - the regression this fence exists for) both refuse by name. The
/// posture itself - the bound device nodes - comes from the row's template
/// through the broker's posture table, never from the setting.
fn video_nvidia_posture(
    template: &str,
    settings: &d2b_provider_device_gpu::GpuSettings,
) -> Result<(), &'static str> {
    if settings.video_nvidia_decode != (template == "video-worker-nvidia") {
        return Err("device-worker-nvidia-posture-mismatch");
    }
    Ok(())
}

/// The host Wayland socket the GPU sidecar renders into.
///
/// The trusted bundle projects it (`site.json`, emitted from the site's own
/// `d2b.site.waylandUser` / `waylandDisplay`), so the daemon never derives
/// `/run/user/<uid>/...` itself: the daemon's own `/run/user` is its runtime
/// directory, not the session user's. `None` - a bundle that predates the
/// artifact, or a site without a Wayland session - keeps the slot unbound so
/// the GPU launch refuses with its own closed code instead of running
/// against a path no trusted artifact names.
fn device_worker_wayland_sock(site: Option<&SiteJson>) -> Option<PathBuf> {
    site.and_then(|site| site.wayland_socket()).map(PathBuf::from)
}

/// The GPU worker's Wayland input, refused by name when the bundle does not
/// project one.
fn gpu_worker_wayland_sock(site: Option<&SiteJson>) -> Result<PathBuf, &'static str> {
    device_worker_wayland_sock(site).ok_or("device-worker-wayland-sock-unbound")
}

/// One per-VM device socket under the daemon runtime root
/// (`/run/d2b/vms/<vm>/<name>`), the convention the guest VMM's
/// `--tpm socket=` / `--gpu socket=` / `--vhost-user-media socket=`
/// arguments name (see `nixos-modules/vm-options.nix` and
/// `packages/d2b-provider-device-tpm/nix/guest.nix`).
fn device_runtime_socket(
    socket_runtime_dir: &std::path::Path,
    vm_name: &str,
    file_name: &str,
) -> PathBuf {
    socket_runtime_dir.join("vms").join(vm_name).join(file_name)
}

/// The per-VM video-decoder socket (`/run/d2b-video/<vm>/video.sock`): the
/// video module's own `RuntimeDirectory` and the guest's
/// `--vhost-user-media socket=` argument name it, so the video runtime root is
/// a sibling of the daemon's runtime root.
fn video_runtime_socket(
    socket_runtime_dir: &std::path::Path,
    vm_name: &str,
) -> Option<PathBuf> {
    let root = socket_runtime_dir.parent()?.join("d2b-video");
    Some(root.join(vm_name).join("video.sock"))
}

/// Compose one Device-owned worker row's launch arguments.
///
/// This is the seat where the daemon's runtime paths (state directory, socket
/// roots) become the launch's arguments: the owning Device Provider's own argv
/// generator renders and validates them (absolute paths, bounded log level,
/// closed context classes), and the executable slot is dropped because the
/// trusted template pins the binary and the broker composes `argv[0]`.
///
/// The fence: every path this seat renders is absolute and free of `..`
/// components, and the paths that must live under the daemon's shared socket
/// runtime root (the swtpm server socket, the GPU socket) are additionally
/// anchored under the root this call receives. The remaining paths are fenced
/// at derivation instead of here: the swtpm state directory and its ctrl
/// socket resolve through the bundle's trusted storage row
/// ([`device_state_dir`], the daemon-side derivation this module owns), the
/// one-shot flush's ctrl socket is that same state directory, and the video
/// and Wayland sockets come from the daemon's own runtime roots and the
/// bundle's projected site artifacts - so a declared row launches with
/// exactly the paths those trusted sources named.
fn device_worker_launch_args(
    socket_runtime_dir: &std::path::Path,
    launch: &DeviceWorkerLaunch,
) -> Result<Vec<String>, String> {
    let argv = match launch {
        DeviceWorkerLaunch::Swtpm(params) => {
            device_worker_path(&params.state_dir, None, "swtpm-state-dir")?;
            device_worker_path(&params.ctrl_socket_path, None, "swtpm-ctrl-socket")?;
            device_worker_path(
                &params.server_socket_path,
                Some(socket_runtime_dir),
                "swtpm-server-socket",
            )?;
            d2b_provider_device_tpm::generate_swtpm_argv(&d2b_provider_device_tpm::SwtpmArgvInput {
                swtpm_binary_path: params.binary_path.to_string_lossy().into_owned(),
                vm_name: params.vm_name.clone(),
                state_dir: params.state_dir.to_string_lossy().into_owned(),
                ctrl_socket_path: params.ctrl_socket_path.to_string_lossy().into_owned(),
                server_socket_path: params.server_socket_path.to_string_lossy().into_owned(),
                uid: params.uid,
                gid: params.gid,
                log_path: params
                    .state_dir
                    .join("swtpm.log")
                    .to_string_lossy()
                    .into_owned(),
                log_level: params.log_level,
                pid_path: params
                    .state_dir
                    .join("swtpm.pid")
                    .to_string_lossy()
                    .into_owned(),
                startup_clear: true,
                extra_args: Vec::new(),
            })
            .map_err(|_| "provider-ticket:device-worker-argv-invalid:swtpm".to_owned())?
        }
        DeviceWorkerLaunch::SwtpmFlush(params) => {
            device_worker_path(&params.ctrl_socket_path, None, "swtpm-flush-ctrl-socket")?;
            d2b_provider_device_tpm::generate_swtpm_ioctl_flush_argv(
                &d2b_provider_device_tpm::SwtpmIoctlFlushInput {
                    swtpm_ioctl_binary_path: params.ioctl_binary_path.to_string_lossy().into_owned(),
                    vm_name: params.vm_name.clone(),
                    ctrl_socket_path: params.ctrl_socket_path.to_string_lossy().into_owned(),
                },
            )
            .map_err(|_| "provider-ticket:device-worker-argv-invalid:swtpm-flush".to_owned())?
        }
        DeviceWorkerLaunch::Gpu(params) => {
            device_worker_path(
                &params.socket_path,
                Some(socket_runtime_dir),
                "gpu-worker-socket",
            )?;
            device_worker_path(&params.wayland_sock, None, "gpu-worker-wayland-sock")?;
            // The typed parameters travel as the canonical JSON of the
            // Provider's own `GpuParams` (the family crate cannot depend on
            // the realizer provider crate); a payload that no longer decodes
            // is a refusal, never a launch with silently dropped settings.
            let gpu_params =
                serde_json::from_value::<d2b_provider_device_gpu::GpuParams>(params.params.clone())
                    .map_err(|_| "provider-ticket:device-worker-gpu-params-invalid".to_owned())?;
            d2b_provider_device_gpu::generate_gpu_argv(&d2b_provider_device_gpu::GpuArgvInput {
                crosvm_binary_path: params.binary_path.to_string_lossy().into_owned(),
                vm_name: params.vm_name.clone(),
                socket_path: params.socket_path.to_string_lossy().into_owned(),
                wayland_sock: params.wayland_sock.to_string_lossy().into_owned(),
                params: gpu_params,
                extra_args: Vec::new(),
            })
            .map_err(|_| "provider-ticket:device-worker-argv-invalid:gpu".to_owned())?
        }
        DeviceWorkerLaunch::Video(params) => {
            device_worker_path(
                &params.socket_path,
                None,
                "video-worker-socket",
            )?;
            d2b_provider_device_gpu::generate_video_argv(&d2b_provider_device_gpu::VideoArgvInput {
                crosvm_binary_path: params.binary_path.to_string_lossy().into_owned(),
                vm_name: params.vm_name.clone(),
                socket_path: params.socket_path.to_string_lossy().into_owned(),
                backend: d2b_provider_device_gpu::VideoBackend::Vaapi,
            })
            .map_err(|_| "provider-ticket:device-worker-argv-invalid:video".to_owned())?
        }
    };
    // `argv[0]` is the trusted binary the template pins: the broker composes
    // it, so the ticket carries only the argument tail.
    let mut argv = argv;
    if argv.is_empty() {
        return Err("provider-ticket:device-worker-argv-empty".to_owned());
    }
    Ok(argv.split_off(1))
}

/// The launch inputs shared by every resource ticket assembly: the resolved
/// execution spec, the runner activation inputs (when declared), the verbatim
/// resource-row bytes the compiled-ticket digest commits, and the readiness
/// class to admit.
struct ExecutionIntent<'a> {
    execution: &'a d2b_contracts_resource::v3::process::ExecutionSpec,
    activation_input: Option<&'a d2b_contracts_resource::v3::ActivationRunnerInput>,
    spec_bytes: &'a [u8],
    readiness: Option<ReadinessClass>,
}

/// Compose the launch ticket of one one-shot (`EphemeralProcess`) resource
/// row.
///
/// A declared Device-owned one-shot worker row (the TPM pre-start flush) is
/// still a Device worker: its ticket carries the typed parameters the Process
/// controller derived from its owning Device row, its trusted template, and
/// the daemon's own runtime paths - the same attach `launch_resource`
/// performs for the durable rows. Without it the ticket carries no launch
/// arguments, the broker composes the bare template argv
/// (`mint_template_intent` renders `[binary_ref]`), and the one-shot worker
/// can neither reach its ctrl socket nor pass the typed `w1-swtpm` fence that
/// fences that socket.
fn ephemeral_launch_ticket(
    bundle: &BundleResolver,
    socket_runtime_dir: &std::path::Path,
    context: &ProcessResourceContext<'_>,
    intent: ExecutionIntent<'_>,
    provider: ManagedProvider,
    mode: DaemonMode,
    timeout: Duration,
) -> Result<LaunchTicket, String> {
    let ExecutionIntent {
        execution,
        activation_input,
        spec_bytes,
        readiness,
    } = intent;
    let ticket = resource_ticket(
        bundle,
        context,
        ExecutionIntent {
            execution,
            activation_input,
            spec_bytes,
            readiness,
        },
        provider,
        mode,
        timeout,
    )?;
    match context.device_worker_launch.as_ref() {
        Some(launch) => ticket
            .with_launch_args(device_worker_launch_args(socket_runtime_dir, launch)?)
            .map_err(|_| "provider-ticket:device-worker-args-invalid".to_owned()),
        None => Ok(ticket),
    }
}

fn resource_ticket(
    bundle: &BundleResolver,
    context: &ProcessResourceContext<'_>,
    intent: ExecutionIntent<'_>,
    provider: ManagedProvider,
    mode: DaemonMode,
    timeout: Duration,
) -> Result<LaunchTicket, String> {
    let ExecutionIntent {
        execution,
        activation_input,
        spec_bytes,
        readiness,
    } = intent;
    validate_resource_execution_target(mode, context, execution)?;
    let execution_domain = match execution.domain().unwrap_or(ExecutionDomain::System) {
        ExecutionDomain::System => d2b_core::processes::ProcessExecutionDomain::System,
        ExecutionDomain::User => d2b_core::processes::ProcessExecutionDomain::User,
    };
    let user_ref = execution.user_ref().map(ResourceRef::to_canonical_string);
    // One canonical identity: the row layers attach it; a context without one
    // (target-local probes and fixtures) resolves through the same resolver
    // instead of a second derivation.
    let launch = match context.launch.clone() {
        Some(launch) => launch,
        None => resolve_launch_identity(&LaunchRow {
            owner_ref: context.owner_ref.as_ref(),
            owner_uid: context.owner_uid.clone(),
            execution_ref: execution.execution_ref(),
            process_name: context.resource_ref.name().as_str(),
            template: execution.template().as_str(),
            declared_target: context.target_ref.as_ref().zip(context.owner_ref.as_ref()),
        })
        .map_err(|error| format!("provider-ticket:{}", error.code()))?,
    };
    let intent_vm = launch.vm();
    let execution_ref = execution.execution_ref().to_canonical_string();
    let owner_ref = launch
        .owner_ref()
        .map(ResourceRef::to_canonical_string);
    let exact_static_controller = execution.process_class() == ProcessClass::Controller
        && launch
            .owner_ref()
            .is_some_and(|owner| owner.resource_type().as_str() == "Provider");
    let credential_agent = is_credential_agent_context(context, execution);
    if exact_static_controller
        && launch
            .owner_ref()
            .is_some_and(is_credential_provider_ref)
        && execution.execution_ref().resource_type().as_str() != "Guest"
    {
        return Err("provider-ticket:credential-provider-guest-required".to_owned());
    }
    if execution.process_class() == ProcessClass::Controller && !exact_static_controller {
        return Err("provider-ticket:controller-owner-invalid".to_owned());
    }
    if credential_agent && execution.execution_ref().resource_type().as_str() != "Guest" {
        return Err("provider-ticket:credential-agent-guest-required".to_owned());
    }
    let static_intent = exact_static_controller.then(|| {
        bundle.find_provider_controller_intent(
            context.resource_ref,
            &execution_ref,
            execution_domain,
            user_ref.as_deref(),
            execution.template().as_str(),
            owner_ref.as_deref(),
        )
    });
    let binding_worker = launch.is_binding_worker();
    // A Device-declared worker row (`Process/swtpm-<device>`,
    // `EphemeralProcess/swtpm-flush-<device>`, `Process/gpu-<device>`, ...)
    // is owned by the Device it serves and executes on the Host: its trusted
    // template is the owning Device Provider's declared `processTemplates`
    // binding for that exact row, never the guest VMM chain.
    let device_worker = launch
        .owner_ref()
        .is_some_and(|owner| owner.resource_type().as_str() == "Device");
    let generic_intent = if exact_static_controller {
        None
    } else if credential_agent {
        bundle.find_provider_component_intent_for_template(
            &execution_ref,
            execution_domain,
            user_ref.as_deref(),
            execution.template().as_str(),
            Some(MANAGED_IDENTITY_PROVIDER_REF),
        )
    } else if binding_worker {
        // Binding-owned serving workers resolve through the owning
        // Provider's signed serving template, not the guest VMM chain.
        if execution.template().as_str()
            != d2b_provider_volume_virtiofs::WORKER_TEMPLATE
        {
            return Err("provider-ticket:template-not-found".to_owned());
        }
        bundle.find_provider_component_intent_for_template(
            &execution_ref,
            execution_domain,
            user_ref.as_deref(),
            execution.template().as_str(),
            Some("Provider/volume-virtiofs"),
        )
    } else if device_worker {
        // The declared row name is the launch identity: two Devices in one
        // Zone share the template but never the row, and the lookup pins the
        // template's owning Device Provider through the closed posture table.
        bundle.find_device_worker_intent(
            context.resource_ref,
            &execution_ref,
            execution_domain,
            user_ref.as_deref(),
            execution.template().as_str(),
        )
    } else if let Some(owner) = launch
        .owner_ref()
        .filter(|owner| owner.resource_type().as_str() == "Guest")
    {
        if context.resource_ref.resource_type().as_str() != "Process"
            || context.resource_ref.name().as_str() != format!("{}-vmm", owner.name().as_str())
        {
            return Err("provider-ticket:guest-process-not-vmm".to_owned());
        }
        let Some(descriptor_digest) = context.guest_descriptor_digest.as_ref() else {
            return Err("provider-ticket:guest-descriptor-unbound".to_owned());
        };
        bundle.find_guest_vmm_intent(
            context.zone.as_str(),
            owner,
            descriptor_digest,
            &execution_ref,
            execution_domain,
            execution.template().as_str(),
        )
    } else {
        bundle.find_runner_intent_for_process_in_vm(
            intent_vm,
            &execution_ref,
            execution_domain,
            user_ref.as_deref(),
            execution.template().as_str(),
        )
    };
    if static_intent.flatten().is_none() && generic_intent.is_none() {
        return Err("provider-ticket:template-not-found".to_owned());
    }
    let trusted_intent = static_intent
        .flatten()
        .or(generic_intent)
        .ok_or_else(|| "provider-ticket:template-not-found".to_owned())?;
    let ticket_template = if exact_static_controller
        || credential_agent
        || binding_worker
        || device_worker
    {
        // The resolved device-worker intent's role id is the declared row
        // name the broker keys the launch by; the ticket's template is the
        // row's declared template.
        execution.template().clone()
    } else {
        BoundedToken::parse(trusted_intent.role_id.clone())
            .map_err(|_| "provider-ticket:invalid-template".to_owned())?
    };
    let provider_name = context.provider_ref.name().as_str();
    let owner_provider =
        BoundedToken::parse(provider_name).map_err(|_| "provider-ticket:invalid-provider")?;
    let component = BoundedToken::parse("process-controller")
        .map_err(|_| "provider-ticket:invalid-component")?;
    let generation = context.resource_generation.get();
    let lifecycle_scope = format!(
        "{}:{}:{}",
        context
            .zone_uid
            .as_ref()
            .map(ResourceUid::as_str)
            .unwrap_or("unbound"),
        context
            .policy_revision
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unbound".to_owned()),
        context
            .provider_assignment_generation
            .map(|value| value.get().to_string())
            .unwrap_or_else(|| "unbound".to_owned()),
    );
    let operation_uid = stable_uid(
        "operation",
        &context.resource_ref.to_canonical_string(),
        &format!("{}:{lifecycle_scope}", context.resource_uid.as_str()),
        generation,
    );
    let deadline_ms = timeout.as_millis().clamp(1, 900_000) as u32;
    let mut ticket = LaunchTicket::new(
        context.resource_ref.clone(),
        context.resource_uid.clone(),
        context.resource_generation,
        context.controller_generation,
        owner_provider.clone(),
        component,
        ticket_template,
        execution.execution_ref().clone(),
        execution.domain().unwrap_or(ExecutionDomain::System),
        execution.user_ref().cloned(),
        owner_provider,
        compiled_resource_digests(bundle, context.resource_ref, provider, spec_bytes),
        OperationBinding::new(operation_uid, deadline_ms)
            .map_err(|_| "provider-ticket:invalid-operation")?,
        required_identity(provider),
    )
    .map_err(|error| format!("provider-ticket:{}", error.code()))?
    .with_launch_identity(launch.clone())
    .map_err(|error| format!("provider-ticket:{}", error.code()))?;
    ticket = ticket
        .with_inherited_fd_count(if exact_static_controller || credential_agent {
            if credential_agent
                || launch
                    .owner_ref()
                    .is_some_and(is_credential_provider_ref)
            {
                2
            } else {
                1
            }
        } else {
            0
        })
        .map_err(|error| format!("provider-ticket:{}", error.code()))?;
    let ticket = match context.guest_execution.as_ref() {
        Some(binding) if execution.execution_ref().resource_type().as_str() == "Guest" => ticket
            .with_guest_execution_binding(binding.clone())
            .map_err(|error| format!("provider-ticket:{}", error.code()))?,
        Some(_) => {
            return Err("provider-ticket:guest-binding-for-host".to_owned());
        }
        None if execution.execution_ref().resource_type().as_str() == "Guest" => {
            return Err("provider-ticket:guest-binding-missing".to_owned());
        }
        None => ticket,
    };
    let zone_uid = context
        .zone_uid
        .clone()
        .ok_or_else(|| "provider-ticket:zone-identity-missing".to_owned())?;
    let runtime_scope = runtime_scope_commitment(
        &zone_uid,
        context
            .guest_execution
            .as_ref()
            .map(GuestExecutionBinding::target_uid),
        context.resource_ref,
        context.resource_uid,
        trusted_intent.role_id.as_str(),
        context.resource_generation.get(),
    );
    let ticket = ticket
        .with_runtime_identity(zone_uid, launch.owner_ref().cloned(), runtime_scope)
        .map_err(|error| format!("provider-ticket:{}", error.code()))?;
    let ticket = match context.owner_uid.as_ref() {
        Some(owner_uid) if ticket.owner_uid().is_none() => ticket
            .with_owner_uid(owner_uid.clone())
            .map_err(|error| format!("provider-ticket:{}", error.code()))?,
        _ => ticket,
    };
    let ticket = match activation_input {
        Some(input) => ticket
            .with_activation_input(input.clone())
            .map_err(|error| format!("provider-ticket:{}", error.code()))?,
        None => ticket,
    };
    let commitment = execution_commitment(
        bundle.audit_bundle_hash(),
        ticket.execution_ref(),
        ticket.target_ref(),
        ticket.domain(),
        ticket.user_ref(),
        ticket.template(),
        ticket.selected_provider(),
    );
    let domain = execution.domain().unwrap_or(ExecutionDomain::System);
    let sandbox = if provider == ManagedProvider::Systemd {
        let spec = execution.sandbox();
        if !spec.namespace_classes().is_empty()
            || !spec.capability_classes().is_empty()
            || spec.seccomp_class().as_str() != "strict"
            || !spec.no_new_privileges()
            || spec.start_root()
            || !matches!(
                spec.environment_class(),
                d2b_contracts_resource::v3::process::EnvironmentClass::Minimal
            )
            || !spec.read_only_root()
            || spec.user_namespace().is_some()
        {
            return Err("provider-ticket:systemd-sandbox-unsupported".to_owned());
        }
        SandboxCompiler
            .compile_plan(spec, domain, false)
            .map_err(|error| format!("provider-ticket:{}", error.code()))?
    } else {
        SandboxCompiler
            .compile_plan(execution.sandbox(), domain, false)
            .map_err(|error| format!("provider-ticket:{}", error.code()))?
    };
    let readiness = resource_readiness_expectation(readiness, timeout)?;
    Ok(ticket
        .with_resource_revision(context.resource_revision)
        .map_err(|error| format!("provider-ticket:{}", error.code()))?
        .with_execution_commitment(commitment)
        .map_err(|error| format!("provider-ticket:{}", error.code()))?
        .with_sandbox_plan(sandbox)
        .with_readiness(readiness))
}

fn resource_readiness_expectation(
    readiness: Option<ReadinessClass>,
    timeout: Duration,
) -> Result<ReadinessExpectation, String> {
    match readiness {
        Some(ReadinessClass::ReadyCondition) => {
            let timeout_ms = timeout.as_millis().clamp(1, 900_000) as u32;
            ReadinessExpectation::condition(timeout_ms)
                .map_err(|_| "provider-ticket:invalid-readiness".to_owned())
        }
        Some(ReadinessClass::ProviderDefined) | None => Ok(ReadinessExpectation::None),
    }
}

fn compiled_resource_digests(
    bundle: &BundleResolver,
    resource_ref: &ResourceRef,
    provider: ManagedProvider,
    spec_bytes: &[u8],
) -> CompiledDigests {
    fn digest(label: &str, bytes: &[u8]) -> ConfigurationDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"d2bd-provider-resource-ticket-v1");
        hasher.update(label.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        ConfigurationDigest::from_bytes(hasher.finalize().into())
    }
    let context = format!(
        "{}:{}:{}",
        resource_ref.to_canonical_string(),
        match provider {
            ManagedProvider::Minijail => "system-minijail",
            ManagedProvider::Systemd => "system-systemd",
        },
        bundle.bundle.bundle_hash.as_deref().unwrap_or("bundle"),
    );
    CompiledDigests {
        sandbox: digest(&format!("{context}:sandbox"), spec_bytes),
        budget: digest(&format!("{context}:budget"), spec_bytes),
        mounts: digest(&format!("{context}:mounts"), spec_bytes),
        devices: digest(&format!("{context}:devices"), spec_bytes),
        network: digest(&format!("{context}:network"), spec_bytes),
        endpoints: digest(&format!("{context}:endpoints"), spec_bytes),
        fd_table: digest(&format!("{context}:fd-table"), spec_bytes),
    }
}

fn build_ticket(
    bundle: &BundleResolver,
    vm: &str,
    node: &ProcessNode,
    provider: ManagedProvider,
    timeout: Duration,
) -> Result<LaunchTicket, ProcessConformanceError> {
    let provider_name = match provider {
        ManagedProvider::Minijail => "system-minijail",
        ManagedProvider::Systemd => "system-systemd",
    };
    let process_type = if ProductionProcessProviders::is_long_lived(node) {
        "Process"
    } else {
        "EphemeralProcess"
    };
    let process_name = stable_token(&node.id.0);
    let process_ref = ResourceRef::parse(&format!("{process_type}/{process_name}"))
        .map_err(|_| ProcessConformanceError::InvalidTicket)?;
    let execution_ref = ResourceRef::parse(
        &node
            .execution_ref
            .clone()
            .unwrap_or_else(|| d2b_core::bundle_resolver::default_execution_ref(vm, &node.role)),
    )
    .map_err(|_| ProcessConformanceError::InvalidTicket)?;
    let owner_provider =
        BoundedToken::parse(provider_name).map_err(|_| ProcessConformanceError::InvalidTicket)?;
    let component =
        BoundedToken::parse("vm-process").map_err(|_| ProcessConformanceError::InvalidTicket)?;
    let template = BoundedToken::parse(stable_token(&node.id.0))
        .map_err(|_| ProcessConformanceError::InvalidTicket)?;
    let selected_provider = owner_provider.clone();
    let commitment = execution_commitment(
        bundle.audit_bundle_hash(),
        &execution_ref,
        None,
        ExecutionDomain::System,
        None,
        &template,
        &selected_provider,
    );
    let generation = stable_generation(bundle);
    let digests = compiled_digests(bundle, vm, node, provider);
    let operation_uid = stable_uid("operation", vm, &node.id.0, generation);
    let deadline_ms = timeout.as_millis().clamp(1, 900_000) as u32;
    let ticket = LaunchTicket::new(
        process_ref.clone(),
        stable_uid("process", vm, &node.id.0, generation),
        ResourceGeneration::new(generation).map_err(|_| ProcessConformanceError::InvalidTicket)?,
        ControllerGeneration::new(1).map_err(|_| ProcessConformanceError::InvalidTicket)?,
        owner_provider,
        component,
        template,
        execution_ref.clone(),
        ExecutionDomain::System,
        None,
        selected_provider,
        digests,
        OperationBinding::new(operation_uid, deadline_ms)?,
        required_identity(provider),
    )?;
    // A bundle process-DAG node names its own VM: the execution target alone
    // (a shared `Host/host-system`) cannot name the DAG's vm, and the broker's
    // identity fence resolves the node's runner intent under that name.
    let launch_identity = LaunchIdentity::new(
        None,
        None,
        execution_ref,
        None,
        process_ref.name().as_str(),
        false,
    )
    .and_then(|identity| identity.with_vm(vm))
    .map_err(|_| ProcessConformanceError::InvalidTicket)?;
    Ok(ticket
        .with_execution_commitment(commitment)
        .map_err(|_| ProcessConformanceError::InvalidTicket)?
        .with_launch_identity(launch_identity)
        .map_err(|_| ProcessConformanceError::InvalidTicket)?
        .with_readiness(ReadinessExpectation::None))
}

fn required_identity(provider: ManagedProvider) -> std::collections::BTreeSet<IdentityBinding> {
    match provider {
        ManagedProvider::Minijail => std::collections::BTreeSet::from([
            IdentityBinding::Pid,
            IdentityBinding::ProcessStartTime,
            IdentityBinding::Cgroup,
            IdentityBinding::Executable,
            IdentityBinding::Template,
            IdentityBinding::Generation,
        ]),
        ManagedProvider::Systemd => std::collections::BTreeSet::from([
            IdentityBinding::UnitInvocationId,
            IdentityBinding::Cgroup,
            IdentityBinding::UnitMainPid,
            IdentityBinding::ProcessStartTime,
            IdentityBinding::Template,
            IdentityBinding::Generation,
        ]),
    }
}

fn compiled_digests(
    bundle: &BundleResolver,
    vm: &str,
    node: &ProcessNode,
    provider: ManagedProvider,
) -> CompiledDigests {
    fn digest(label: &str, bytes: &[u8]) -> ConfigurationDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"d2bd-provider-ticket-v1");
        hasher.update(label.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        ConfigurationDigest::from_bytes(hasher.finalize().into())
    }
    let node_bytes = serde_json::to_vec(node).unwrap_or_default();
    let context = format!(
        "{vm}:{}:{}:{}",
        node.id.0,
        match provider {
            ManagedProvider::Minijail => "system-minijail",
            ManagedProvider::Systemd => "system-systemd",
        },
        bundle.bundle.bundle_hash.as_deref().unwrap_or("bundle")
    );
    CompiledDigests {
        sandbox: digest(&format!("{context}:sandbox"), &node_bytes),
        budget: digest(&format!("{context}:budget"), &node_bytes),
        mounts: digest(&format!("{context}:mounts"), &node_bytes),
        devices: digest(&format!("{context}:devices"), &node_bytes),
        network: digest(&format!("{context}:network"), &node_bytes),
        endpoints: digest(&format!("{context}:endpoints"), &node_bytes),
        fd_table: digest(&format!("{context}:fd-table"), &node_bytes),
    }
}

fn stable_generation(bundle: &BundleResolver) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(
        bundle
            .bundle
            .bundle_hash
            .as_deref()
            .unwrap_or(bundle.bundle.generation.generator.as_str()),
    );
    let bytes: [u8; 32] = hasher.finalize().into();
    let generation = u64::from_le_bytes(bytes[..8].try_into().expect("digest prefix"));
    if generation == 0 { 1 } else { generation }
}

fn stable_uid(label: &str, vm: &str, role: &str, generation: u64) -> ResourceUid {
    let mut hasher = Sha256::new();
    hasher.update(b"d2bd-provider-resource-v1");
    hasher.update(label.as_bytes());
    hasher.update([0]);
    hasher.update(vm.as_bytes());
    hasher.update([0]);
    hasher.update(role.as_bytes());
    hasher.update(generation.to_le_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    ResourceUid::from_bytes(&bytes).expect("stable provider uid")
}

fn stable_token(value: &str) -> String {
    let valid = !value.is_empty()
        && value.len() <= 63
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid {
        let digest = Sha256::digest(value.as_bytes());
        return format!(
            "process-{:02x}{:02x}{:02x}{:02x}",
            digest[0], digest[1], digest[2], digest[3]
        );
    }
    value.to_owned()
}

/// The composed fixed runtime as the family's declared facet (U1): every
/// method delegates to the same inherent surface the daemon's own callers
/// use, so the family's effects observe exactly the daemon's runtime. The
/// trait object is what crosses the provider boundary - never a daemon
/// state type.
#[async_trait::async_trait]
impl ProcessProviderRuntime for ProductionProcessProviders {
    fn bundle(&self) -> &BundleResolver {
        self.bundle()
    }

    fn socket_runtime_dir(&self) -> &std::path::Path {
        self.socket_runtime_dir()
    }

    fn guest_setup_descriptor_digest(
        &self,
        zone: &ZoneId,
        guest_ref: &ResourceRef,
    ) -> Option<SchemaFingerprint> {
        self.guest_setup_descriptor_digest(zone, guest_ref)
    }

    async fn resolve_device_worker_launch(
        &self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &ProcessFamilySpec,
    ) -> Result<Option<DeviceWorkerLaunch>, &'static str> {
        self.resolve_device_worker_launch(ctx, identity, spec).await
    }

    async fn launch_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
        timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        self.launch_resource(context, spec, timeout).await
    }

    async fn launch_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
        timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        self.launch_ephemeral_resource(context, spec, timeout).await
    }

    async fn adopt_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.adopt_resource(context, spec).await
    }

    async fn probe_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.probe_resource(context, spec).await
    }

    async fn adopt_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.adopt_ephemeral_resource(context, spec).await
    }

    async fn probe_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.probe_ephemeral_resource(context, spec).await
    }

    async fn stop_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.stop_resource(context, spec, term_timeout, kill_timeout).await
    }

    async fn stop_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.stop_ephemeral_resource(context, spec, term_timeout, kill_timeout)
            .await
    }

    async fn stop_stale_resource(
        &self,
        provider_ref: &ResourceRef,
        candidate: &AdoptionCandidate,
    ) -> Result<(), String> {
        self.stop_stale_resource(provider_ref, candidate).await
    }

    async fn finalize_resource(
        &self,
        context: ProcessResourceContext<'_>,
    ) -> Result<(), String> {
        self.finalize_resource(context).await
    }

    fn has_active_resource_in_zone(
        &self,
        zone: &ZoneId,
        zone_uid: Option<&ResourceUid>,
        resource_ref: &ResourceRef,
    ) -> bool {
        self.has_active_resource_in_zone(zone, zone_uid, resource_ref)
    }
}

/// The daemon's committed-`Provider` identity view exposed through the
/// provider-declared KTD7 facet (U1): the plane's registry publishes the
/// committed rows, and the composition root wires this view as the family's
/// committed identity source.
pub(crate) struct PlaneCommittedProviderIdentitySource {
    pub(crate) registry: Arc<crate::resource_plane_v3::PlaneResourceRegistry>,
}

impl CommittedProviderIdentitySource for PlaneCommittedProviderIdentitySource {
    fn committed_provider_identity(
        &self,
        provider: &ResourceRef,
    ) -> Option<(ResourceUid, d2b_contracts_resource::v3::ResourceGeneration)> {
        self.registry.committed_provider_identity(provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_provider_process::{GpuWorkerParams, SwtpmFlushParams, SwtpmWorkerParams, VideoWorkerParams};
    use d2b_contracts_resource::v3::{
        CanonicalJsonObject, ControllerGeneration, ResourceGeneration, ResourceName, ResourceRef,
        ResourceTypeName, Timestamp, ZoneId, ZoneRevision,
        execution_policy::{BoundedToken, ExecutionDomain},
    };
    use d2b_contracts_zone_session::v3::resource_bundle::{
        BundleResource, BundleResourceMetadata, ResourceBundle,
    };
    use d2b_core::{
        bundle::{Bundle, BundleGeneration},
        processes::ProcessesJson,
    };

    /// The daemon's own minijail `PlatformGate` converts into the Host
    /// family's gate type field for field, including the negative posture:
    /// a swapped or mis-copied field would change the gate the family's
    /// probe reports through the facet.
    #[test]
    fn minijail_platform_gate_converts_field_for_field() {
        let positive = PlatformGate::from_observed(6, 9, true);
        let converted = convert_platform_gate(positive);
        assert_eq!(converted.kernel_major, 6);
        assert_eq!(converted.kernel_minor, 9);
        assert!(converted.cgroup_kill_available);

        let negative = PlatformGate::from_observed(5, 2, false);
        let converted = convert_platform_gate(negative);
        assert_eq!(converted.kernel_major, 5);
        assert_eq!(converted.kernel_minor, 2);
        assert!(!converted.cgroup_kill_available);
    }

    fn controller_bootstrap_context_for_fence(
        generation: u64,
        process_uid: &str,
    ) -> ControllerBootstrapContext {
        ControllerBootstrapContext {
            zone: ZoneId::parse("work").expect("zone"),
            zone_uid: Some(
                ResourceUid::parse("323e4567-e89b-42d3-a456-426614174001")
                    .expect("zone uid"),
            ),
            process_ref: ResourceRef::parse("Process/provider-controller")
                .expect("process ref"),
            process_uid: ResourceUid::parse(process_uid).expect("process uid"),
            generation: ResourceGeneration::new(generation).expect("process generation"),
            process_identity: ProcessIdentityDigest::from_bytes([generation as u8; 32]),
            process_provider_ref: ResourceRef::parse("Provider/system-minijail")
                .expect("process provider"),
            provider_owner_ref: ResourceRef::parse("Provider/runtime-cloud-hypervisor")
                .expect("provider owner"),
            provider_uid: ResourceUid::parse("423e4567-e89b-42d3-a456-426614174001")
                .expect("provider uid"),
            provider_generation: ResourceGeneration::new(1).expect("provider generation"),
            execution_ref: ResourceRef::parse("Host/host-system").expect("execution ref"),
            user_ref: None,
            controller_generation: ControllerGeneration::new(1).expect("controller generation"),
        }
    }

    #[test]
    fn newer_bootstrap_context_fences_older_live_session() {
        let live = controller_bootstrap_context_for_fence(
            1,
            "123e4567-e89b-42d3-a456-426614174000",
        );
        let newer = controller_bootstrap_context_for_fence(
            2,
            "223e4567-e89b-42d3-a456-426614174000",
        );

        assert!(controller_session_needs_fence(Some(&newer), &live, false));
        assert!(!controller_session_needs_fence(Some(&live), &live, false));
        assert!(controller_session_needs_fence(None, &live, false));
        assert!(controller_session_needs_fence(Some(&live), &live, true));
    }

    // -- Device-owned worker launch arguments (U17 gap closure) --------------
    //
    // The declared Device worker rows carry no paths (the Process spec is
    // argv-free and the rows are declared path-free), so the Process
    // controller derives typed parameters and this composition seat renders
    // them through the owning Device Provider's own argv generator. These
    // tests pin the rendered tail (`argv[0]` is the broker-pinned executable)
    // and the daemon-side path fence.

    fn swtpm_params() -> DeviceWorkerLaunch {
        DeviceWorkerLaunch::Swtpm(Box::new(SwtpmWorkerParams {
            binary_path: PathBuf::from("/nix/store/swtpm/bin/swtpm"),
            vm_name: "corp-vm".to_owned(),
            state_dir: PathBuf::from("/var/lib/d2b/vms/corp-vm/swtpm/device-abc-tpm-state"),
            ctrl_socket_path: PathBuf::from(
                "/var/lib/d2b/vms/corp-vm/swtpm/device-abc-tpm-state/ctrl.sock",
            ),
            server_socket_path: PathBuf::from("/run/d2b/vms/corp-vm/tpm.sock"),
            uid: 60_100,
            gid: 60_100,
            log_level: 20,
        }))
    }

    /// The socket owner ids the rendered argv names follow the posture's
    /// user namespace, not the host principal: the swtpm worker runs under
    /// the ADR 0021 single-entry mapping, whose only id is in-namespace `0`
    /// (naming the host principal there made swtpm exit 1 on a socket-chown
    /// `EINVAL` before it ever bound a socket).
    #[test]
    fn namespaced_swtpm_worker_argv_names_the_in_namespace_socket_owner() {
        use d2b_core::bundle_resolver::{DEVICE_TPM_PROVIDER_REF, device_worker_posture};

        let posture = device_worker_posture(DEVICE_TPM_PROVIDER_REF, "swtpm-socket")
            .expect("swtpm-socket posture");
        assert!(posture.user_namespace(), "the long-lived worker is namespaced");
        let (uid, gid) = posture.launch_ids(60_100, 60_100);
        assert_eq!(
            (uid, gid),
            (0, 0),
            "the host principal is unmapped inside its own namespace"
        );

        let DeviceWorkerLaunch::Swtpm(mut params) = swtpm_params() else {
            unreachable!("fixture is the long-lived swtpm family");
        };
        params.uid = uid;
        params.gid = gid;
        let args = device_worker_launch_args(
            std::path::Path::new("/run/d2b"),
            &DeviceWorkerLaunch::Swtpm(params),
        )
        .expect("the derived typed parameters render");
        for flag in ["--ctrl", "--server"] {
            let value = &args[args
                .iter()
                .position(|arg| arg == flag)
                .expect("socket flag is rendered")
                + 1];
            assert!(
                value.ends_with(",mode=0660,uid=0,gid=0"),
                "{flag} must name the in-namespace owner: {value}"
            );
        }

        let flush = device_worker_posture(DEVICE_TPM_PROVIDER_REF, "swtpm-init-flush")
            .expect("flush posture");
        assert!(!flush.user_namespace(), "the one-shot flush runs unnamespaced");
        assert_eq!(
            flush.launch_ids(60_100, 60_100),
            (60_100, 60_100),
            "a posture without a user namespace keeps the host ids"
        );
    }

    #[test]
    fn device_worker_launch_args_render_the_owning_provider_argv() {
        let args = device_worker_launch_args(std::path::Path::new("/run/d2b"), &swtpm_params())
            .expect("the derived typed parameters render");
        assert_eq!(
            args,
            vec![
                "socket",
                "--tpm2",
                "--tpmstate",
                "dir=/var/lib/d2b/vms/corp-vm/swtpm/device-abc-tpm-state",
                "--ctrl",
                "type=unixio,path=/var/lib/d2b/vms/corp-vm/swtpm/device-abc-tpm-state/ctrl.sock,mode=0660,uid=60100,gid=60100",
                "--server",
                "type=unixio,path=/run/d2b/vms/corp-vm/tpm.sock,mode=0660,uid=60100,gid=60100",
                "--flags",
                "startup-clear",
                "--log",
                "file=/var/lib/d2b/vms/corp-vm/swtpm/device-abc-tpm-state/swtpm.log,level=20",
                "--pid",
                "file=/var/lib/d2b/vms/corp-vm/swtpm/device-abc-tpm-state/swtpm.pid",
            ],
            "the executable slot is the broker-pinned argv[0], never a ticket argument"
        );

        let flush = DeviceWorkerLaunch::SwtpmFlush(Box::new(SwtpmFlushParams {
            ioctl_binary_path: PathBuf::from("/nix/store/swtpm/bin/swtpm_ioctl"),
            vm_name: "corp-vm".to_owned(),
            ctrl_socket_path: PathBuf::from(
                "/var/lib/d2b/vms/corp-vm/swtpm/device-abc-tpm-state/ctrl.sock",
            ),
        }));
        assert_eq!(
            device_worker_launch_args(std::path::Path::new("/run/d2b"), &flush)
                .expect("the one-shot flush parameters render"),
            vec![
                "-i",
                "--unix",
                "/var/lib/d2b/vms/corp-vm/swtpm/device-abc-tpm-state/ctrl.sock",
            ]
        );

        let video = DeviceWorkerLaunch::Video(Box::new(VideoWorkerParams {
            binary_path: PathBuf::from("/nix/store/crosvm/bin/crosvm"),
            vm_name: "corp-vm".to_owned(),
            socket_path: PathBuf::from("/run/d2b-video/corp-vm/video.sock"),
        }));
        assert_eq!(
            device_worker_launch_args(std::path::Path::new("/run/d2b"), &video)
                .expect("the video sidecar parameters render"),
            vec![
                "device",
                "video-decoder",
                "--socket-path",
                "/run/d2b-video/corp-vm/video.sock",
                "--backend",
                "vaapi",
            ]
        );
    }

    /// The fence: a socket the daemon did not derive under its own runtime
    /// root never reaches a launch, even when every typed field is otherwise
    /// well formed.
    #[test]
    fn device_worker_launch_args_refuse_a_socket_outside_the_runtime_root() {
        let mut params = swtpm_params();
        let DeviceWorkerLaunch::Swtpm(swtpm) = &mut params else {
            unreachable!("fixture is the long-lived swtpm family");
        };
        swtpm.server_socket_path = PathBuf::from("/run/foreign/tpm.sock");
        let error = device_worker_launch_args(std::path::Path::new("/run/d2b"), &params)
            .expect_err("a foreign socket is refused");
        assert_eq!(
            error,
            "provider-ticket:device-worker-path-unanchored:swtpm-server-socket"
        );

        let gpu = DeviceWorkerLaunch::Gpu(Box::new(GpuWorkerParams {
            binary_path: PathBuf::from("/nix/store/crosvm/bin/crosvm"),
            vm_name: "corp-vm".to_owned(),
            socket_path: PathBuf::from("vms/corp-vm/gpu.sock"),
            wayland_sock: PathBuf::from("/run/user/1000/wayland-0"),
            params: serde_json::to_value(d2b_provider_device_gpu::GpuParams {
                context_types: vec![d2b_provider_device_gpu::GpuContextType::Virgl],
                displays: vec![d2b_provider_device_gpu::GpuDisplayConfig { hidden: true }],
                egl: true,
                vulkan: true,
            })
            .expect("the declared settings serialize"),
        }));
        assert_eq!(
            device_worker_launch_args(std::path::Path::new("/run/d2b"), &gpu)
                .expect_err("a relative socket is refused"),
            "provider-ticket:device-worker-path-invalid:gpu-worker-socket"
        );
    }

    /// The GPU shape renders from the owning Device's declared settings once
    /// the Wayland socket is bound, so the family is one bound input away from
    /// the same composition seat as the others.
    #[test]
    fn device_worker_launch_args_render_the_gpu_shape_from_device_settings() {
        let gpu = DeviceWorkerLaunch::Gpu(Box::new(GpuWorkerParams {
            binary_path: PathBuf::from("/nix/store/crosvm/bin/crosvm"),
            vm_name: "corp-vm".to_owned(),
            socket_path: PathBuf::from("/run/d2b/vms/corp-vm/gpu.sock"),
            wayland_sock: PathBuf::from("/run/user/1000/wayland-0"),
            params: serde_json::to_value(d2b_provider_device_gpu::GpuParams {
                context_types: vec![
                    d2b_provider_device_gpu::GpuContextType::Virgl,
                    d2b_provider_device_gpu::GpuContextType::Virgl2,
                    d2b_provider_device_gpu::GpuContextType::CrossDomain,
                ],
                displays: vec![d2b_provider_device_gpu::GpuDisplayConfig { hidden: true }],
                egl: true,
                vulkan: true,
            })
            .expect("the declared settings serialize"),
        }));
        assert_eq!(
            device_worker_launch_args(std::path::Path::new("/run/d2b"), &gpu)
                .expect("the GPU parameters render"),
            vec![
                "device",
                "gpu",
                "--socket",
                "/run/d2b/vms/corp-vm/gpu.sock",
                "--wayland-sock",
                "/run/user/1000/wayland-0",
                "--params",
                "{\"context-types\":\"virgl:virgl2:cross-domain\",\"displays\":[{\"hidden\":true}],\"egl\":true,\"vulkan\":true}",
            ]
        );

        // The socket is the bundle-projected site value, not a hardcoded
        // `wayland-0`: a site whose compositor is not first on the seat
        // renders the projected display name.
        let DeviceWorkerLaunch::Gpu(params) = &gpu else {
            unreachable!("fixture is the GPU family");
        };
        let mut projected = (**params).clone();
        projected.wayland_sock = PathBuf::from("/run/user/1001/wayland-7");
        let rendered = device_worker_launch_args(
            std::path::Path::new("/run/d2b"),
            &DeviceWorkerLaunch::Gpu(Box::new(projected)),
        )
        .expect("the projected Wayland socket renders");
        let wayland = rendered
            .iter()
            .position(|arg| arg == "--wayland-sock")
            .expect("the GPU shape carries --wayland-sock");
        assert_eq!(rendered[wayland + 1], "/run/user/1001/wayland-7");
    }

    #[test]
    fn device_worker_launch_args_pin_the_gpu_settings_wire_shape() {
        let gpu = DeviceWorkerLaunch::Gpu(Box::new(GpuWorkerParams {
            binary_path: PathBuf::from("/nix/store/crosvm/bin/crosvm"),
            vm_name: "corp-vm".to_owned(),
            socket_path: PathBuf::from("/run/d2b/vms/corp-vm/gpu.sock"),
            wayland_sock: PathBuf::from("/run/user/1000/wayland-0"),
            params: serde_json::from_str(
                r#"{"context-types":["virgl"],"displays":[{"hidden":true}],"egl":false,"vulkan":true}"#,
            )
            .expect("the hand-written payload is the declared wire shape"),
        }));
        assert_eq!(
            device_worker_launch_args(std::path::Path::new("/run/d2b"), &gpu)
                .expect("the pinned payload renders"),
            vec![
                "device",
                "gpu",
                "--socket",
                "/run/d2b/vms/corp-vm/gpu.sock",
                "--wayland-sock",
                "/run/user/1000/wayland-0",
                "--params",
                "{\"context-types\":\"virgl\",\"displays\":[{\"hidden\":true}],\"egl\":false,\"vulkan\":true}",
            ]
        );

        let malformed = DeviceWorkerLaunch::Gpu(Box::new(GpuWorkerParams {
            binary_path: PathBuf::from("/nix/store/crosvm/bin/crosvm"),
            vm_name: "corp-vm".to_owned(),
            socket_path: PathBuf::from("/run/d2b/vms/corp-vm/gpu.sock"),
            wayland_sock: PathBuf::from("/run/user/1000/wayland-0"),
            params: serde_json::json!({ "contextTypes": ["virgl"] }),
        }));
        assert_eq!(
            device_worker_launch_args(std::path::Path::new("/run/d2b"), &malformed)
                .expect_err("a payload the declared shape refuses fails closed"),
            "provider-ticket:device-worker-gpu-params-invalid"
        );
    }

    #[test]
    fn stable_uids_are_uuid_v4_shaped_and_repeatable() {
        let first = stable_uid("process", "corp-vm", "ch-runner", 7);
        let second = stable_uid("process", "corp-vm", "ch-runner", 7);
        let other = stable_uid("process", "corp-vm", "audio", 7);
        assert_eq!(first, second);
        assert_ne!(first, other);
    }

    #[test]
    fn stable_tokens_close_invalid_bundle_names_without_paths() {
        assert_eq!(stable_token("audio-sidecar"), "audio-sidecar");
        assert!(stable_token("/var/lib/d2b/audio").starts_with("process-"));
        assert!(stable_token("UpperCase").starts_with("process-"));
    }

    #[test]
    fn durable_process_readiness_is_not_reduced_to_liveness() {
        assert_eq!(
            resource_readiness_expectation(
                Some(ReadinessClass::ReadyCondition),
                Duration::from_secs(7),
            )
            .expect("bounded readiness"),
            ReadinessExpectation::Condition { timeout_ms: 7_000 }
        );
        assert_eq!(
            resource_readiness_expectation(None, Duration::from_secs(7))
                .expect("ephemeral readiness"),
            ReadinessExpectation::None
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn controller_bootstrap_wait_rearms_on_endpoint_readiness() {
        use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};

        let (sender, receiver) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("bootstrap socketpair");
        // One short frame on a socketpair never blocks; the writer is a plain
        // task (R13: no blocking-pool thread, timing unchanged).
        let writer = tokio::spawn(async move {
            rustix::net::send(&sender, b"ready", rustix::net::SendFlags::empty())
                .expect("bootstrap readiness frame");
        });
        let endpoint = wait_for_controller_bootstrap_endpoint(receiver, Duration::from_secs(1))
            .await
            .expect("bootstrap endpoint should become readable");
        writer.await.expect("bootstrap writer");
        drop(endpoint);
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn controller_bootstrap_wait_fails_without_endpoint_readiness() {
        use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};

        let (_sender, receiver) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("bootstrap socketpair");
        assert_eq!(
            wait_for_controller_bootstrap_endpoint(receiver, Duration::from_millis(1))
                .await
                .expect_err("unreadable bootstrap endpoint must fail"),
            "provider-controller-bootstrap-timeout"
        );
    }

    #[test]
    fn host_process_providers_reject_guest_execution_before_ticket_creation() {
        let resource_ref = ResourceRef::parse("Process/guest-worker").expect("resource ref");
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid");
        let provider_ref = ResourceRef::parse("Provider/system-systemd").expect("provider ref");
        let context = ProcessResourceContext::new(
            ZoneId::parse("work").expect("zone"),
            (
                &resource_ref,
                &uid,
                ResourceGeneration::new(1).expect("generation"),
                ZoneRevision::new(1)
            ),
            &provider_ref,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        );
        let execution = d2b_contracts_resource::v3::process::ExecutionSpec::minimal(
            ResourceRef::parse("Guest/workload").expect("guest ref"),
            d2b_contracts_resource::v3::process::ProcessClass::Worker,
            BoundedToken::parse("guest-worker").expect("template"),
        )
        .expect("execution");

        assert_eq!(
            validate_resource_execution_target(DaemonMode::Host, &context, &execution),
            Err(GUEST_EXECUTION_UNAVAILABLE.to_owned())
        );
    }

    #[test]
    fn guest_process_providers_reject_host_execution() {
        let resource_ref = ResourceRef::parse("Process/host-worker").expect("resource ref");
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid");
        let provider_ref = ResourceRef::parse("Provider/system-systemd").expect("provider ref");
        let context = ProcessResourceContext::new(
            ZoneId::parse("work").expect("zone"),
            (
                &resource_ref,
                &uid,
                ResourceGeneration::new(1).expect("generation"),
                ZoneRevision::new(1)
            ),
            &provider_ref,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        );
        let execution = d2b_contracts_resource::v3::process::ExecutionSpec::minimal(
            ResourceRef::parse("Host/host-system").expect("host ref"),
            d2b_contracts_resource::v3::process::ProcessClass::Worker,
            BoundedToken::parse("host-worker").expect("template"),
        )
        .expect("execution");

        assert_eq!(
            validate_resource_execution_target(DaemonMode::Guest, &context, &execution),
            Err("provider-ticket:host-execution-denied".to_owned())
        );
    }

    #[test]
    fn production_composition_registers_only_fixed_process_providers() {
        assert_eq!(
            ProductionProcessProviders::provider_names(),
            &["system-minijail", "system-systemd"]
        );
    }

    /// A per-pass resolved identity input the current context cannot resolve
    /// is unknown, never a change: the manager-served Guest's VMM was retired
    /// and relaunched on every daemon restart because the recovery pass that
    /// adopted the live process could not resolve the owning Guest's uid and
    /// the next pass could (U13/U6, 2026-09-11).
    #[test]
    fn identity_change_is_proven_only_when_both_sides_resolve() {
        let resource_ref =
            ResourceRef::parse("Process/acceptance-guest-vmm").expect("resource ref");
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid");
        let zone_uid =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").expect("zone uid");
        let guest_ref = ResourceRef::parse("Guest/acceptance-guest").expect("guest ref");
        let guest_uid =
            ResourceUid::parse("30e0427b-496f-4190-ba7c-879ccdec7964").expect("guest uid");
        let other_guest_uid =
            ResourceUid::parse("30e0427b-496f-4190-ba7c-879ccdec7965").expect("guest uid");
        let provider_ref = ResourceRef::parse("Provider/system-minijail").expect("provider ref");
        let managed = ManagedResource {
            zone: ZoneId::parse("work").expect("zone"),
            zone_uid: Some(zone_uid.clone()),
            resource_ref: resource_ref.clone(),
            provider: ManagedProvider::Minijail,
            provider_ref: provider_ref.clone(),
            provider_uid: None,
            provider_generation: None,
            owner_ref: Some(guest_ref.clone()),
            owner_uid: None,
            template: BoundedToken::parse("cloud-hypervisor-runner").expect("template"),
            identity: ProcessIdentityDigest::from_bytes([7; 32]),
            uid: uid.clone(),
            generation: ResourceGeneration::new(4).expect("generation"),
            controller_generation: ControllerGeneration::new(1).expect("controller generation"),
            execution_ref: ResourceRef::parse("Host/host-system").expect("execution ref"),
            target_ref: None,
            runtime_scope: None,
        };
        let context = ProcessResourceContext::new(
            ZoneId::parse("work").expect("zone"),
            (
                &resource_ref,
                &uid,
                ResourceGeneration::new(4).expect("generation"),
                ZoneRevision::new(4)
            ),
            &provider_ref,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        )
        .with_lifecycle_identity(Some(zone_uid.clone()), Some(1), None)
        .with_owner_ref(Some(guest_ref))
        .with_owner_uid(Some(guest_uid.clone()));
        // The stored entry could not resolve the owner; the request can. That
        // asymmetry is not a change, so the retire pre-flight keeps the live
        // process - while the strict comparison still reports the difference
        // the destructive gates refuse on.
        assert!(resource_identity_changes(&managed, &context).is_empty());
        assert_eq!(
            resource_identity_mismatches(&managed, &context),
            [format!(
                "owner_uid(managed=none,requested={})",
                guest_uid.to_canonical_string()
            )]
        );
        // A proven change - both sides known and different - still retires.
        let mut resolved = managed.clone();
        resolved.owner_uid = Some(guest_uid.clone());
        assert!(resource_identity_changes(&resolved, &context).is_empty());
        let changed = context.clone().with_owner_uid(Some(other_guest_uid.clone()));
        assert_eq!(
            resource_identity_changes(&resolved, &changed),
            [format!(
                "owner_uid(managed={},requested={})",
                guest_uid.to_canonical_string(),
                other_guest_uid.to_canonical_string()
            )]
        );
    }

    #[test]
    fn managed_resource_finalization_requires_the_current_resource_identity() {
        let resource_ref = ResourceRef::parse("Process/worker").expect("resource ref");
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid");
        let zone_uid =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").expect("zone uid");
        let provider_ref = ResourceRef::parse("Provider/system-minijail").expect("provider ref");
        let managed = ManagedResource {
            zone: ZoneId::parse("work").expect("zone"),
            zone_uid: Some(zone_uid.clone()),
            resource_ref: resource_ref.clone(),
            provider: ManagedProvider::Minijail,
            provider_ref: provider_ref.clone(),
            provider_uid: None,
            provider_generation: None,
            owner_ref: None,
            owner_uid: None,
            template: BoundedToken::parse("reaction").expect("template"),
            identity: ProcessIdentityDigest::from_bytes([7; 32]),
            uid: uid.clone(),
            generation: ResourceGeneration::new(4).expect("generation"),
            controller_generation: ControllerGeneration::new(1).expect("controller generation"),
            execution_ref: ResourceRef::parse("Host/host-system").expect("execution ref"),
            target_ref: None,
            runtime_scope: None,
        };
        let context = ProcessResourceContext::new(
            ZoneId::parse("work").expect("zone"),
            (
                &resource_ref,
                &uid,
                ResourceGeneration::new(4).expect("generation"),
                ZoneRevision::new(4)
            ),
            &provider_ref,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        )
        .with_lifecycle_identity(Some(zone_uid.clone()), Some(1), None);
        assert!(resource_identity_matches(&managed, &context));
        let stale_context = ProcessResourceContext::new(
            ZoneId::parse("work").expect("zone"),
            (
                &resource_ref,
                &uid,
                ResourceGeneration::new(3).expect("generation"),
                ZoneRevision::new(3)
            ),
            &provider_ref,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        )
        .with_lifecycle_identity(Some(zone_uid.clone()), Some(1), None);
        assert!(!resource_identity_matches(&managed, &stale_context));
        assert_eq!(
            resource_identity_mismatches(&managed, &stale_context),
            ["resource_generation(managed=4,requested=3)"]
        );
        let newer_revision = ProcessResourceContext::new(
            ZoneId::parse("work").expect("zone"),
            (
                &resource_ref,
                &uid,
                ResourceGeneration::new(4).expect("generation"),
                ZoneRevision::new(5)
            ),
            &provider_ref,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        )
        .with_lifecycle_identity(Some(zone_uid.clone()), Some(1), None);
        assert!(resource_identity_matches(&managed, &newer_revision));
        let stale_controller = ProcessResourceContext::new(
            ZoneId::parse("work").expect("zone"),
            (
                &resource_ref,
                &uid,
                ResourceGeneration::new(4).expect("generation"),
                ZoneRevision::new(4)
            ),
            &provider_ref,
            ControllerGeneration::new(2).expect("controller generation"),
            None,
        )
        .with_lifecycle_identity(Some(zone_uid.clone()), Some(1), None);
        assert!(!resource_identity_matches(&managed, &stale_controller));
        assert_eq!(
            resource_identity_mismatches(&managed, &stale_controller),
            ["controller_generation(managed=1,requested=2)"]
        );
        let different_zone = ProcessResourceContext::new(
            ZoneId::parse("work").expect("zone"),
            (
                &resource_ref,
                &uid,
                ResourceGeneration::new(4).expect("generation"),
                ZoneRevision::new(4)
            ),
            &provider_ref,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        )
        .with_lifecycle_identity(
            Some(ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").expect("zone uid")),
            Some(1),
            None,
        );
        assert!(!resource_identity_matches(&managed, &different_zone));
        assert_eq!(
            resource_identity_mismatches(&managed, &different_zone),
            [format!(
                "zone_uid(managed={},requested={})",
                zone_uid.to_canonical_string(),
                "323e4567-e89b-42d3-a456-426614174002"
            )]
        );
    }

    #[test]
    fn drain_retry_policy_distinguishes_transient_and_permanent_failures() {
        assert!(retryable_stop_error("stop-failed"));
        assert!(retryable_stop_error("process-fate-unknown"));
        assert!(!retryable_stop_error("identity-mismatch"));
        assert!(!retryable_stop_error("permission-denied"));
    }

    /// The controller bootstrap context (what the signed controller ticket
    /// carries) forms only with the owning Provider's committed identity
    /// bound; an unbound context refuses with the exact provider reason the
    /// launch path surfaces (KTD7: the effects bind it, and a missing row
    /// stays refused).
    #[test]
    fn controller_bootstrap_context_requires_the_bound_provider_identity() {
        let zone = ZoneId::parse("work").expect("zone");
        let process_ref = ResourceRef::parse("Process/controller").expect("process ref");
        let provider_ref = ResourceRef::parse("Provider/system-minijail").expect("provider");
        let owner = ResourceRef::parse("Provider/network-local").expect("owner");
        let execution_ref = ResourceRef::parse("Host/host-system").expect("execution");
        let process_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("process uid");
        let provider_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001").expect("provider uid");
        let provider_generation = ResourceGeneration::new(4).expect("generation");
        let identity = ProcessIdentityDigest::from_bytes([0x11; 32]);
        let context = ProcessResourceContext::new(
            zone,
            (
                &process_ref,
                &process_uid,
                ResourceGeneration::new(1).expect("generation"),
                ZoneRevision::new(1)
            ),
            &provider_ref,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        )
        .with_owner_ref(Some(owner));
        assert_eq!(
            ControllerBootstrapContext::from_resource_context(&context, &execution_ref, identity)
                .expect_err("an unbound controller identity refuses closed"),
            "provider-controller-provider-identity-missing"
        );
        let bound = context.with_provider_identity(Some(&provider_uid), Some(provider_generation));
        let bootstrap =
            ControllerBootstrapContext::from_resource_context(&bound, &execution_ref, identity)
                .expect("a bound controller identity forms the bootstrap context");
        assert_eq!(bootstrap.provider_uid(), &provider_uid);
        assert_eq!(bootstrap.provider_generation(), provider_generation);
    }

    #[test]
    fn static_controller_resource_ticket_resolves_private_intent_and_one_fd() {
        let zone = ZoneId::parse("dev").expect("zone");
        let owner = ResourceRef::parse("Provider/runtime-cloud-hypervisor").expect("owner");
        let provider = ResourceRef::parse("Provider/system-minijail").expect("provider");
        let target = ResourceRef::parse("Host/dev-host").expect("target");
        let process_ref = ResourceRef::parse("Process/controller-test").expect("process ref");
        let template = BoundedToken::parse("controller-test").expect("template");
        let execution = d2b_contracts_resource::v3::process::ExecutionSpec::new(
            target.clone(),
            Some(ExecutionDomain::System),
            None,
            ProcessClass::Controller,
            template.clone(),
            None,
            Vec::new(),
            Vec::new(),
            d2b_contracts_resource::v3::process::SandboxSpec::default(),
            d2b_contracts_resource::v3::execution_policy::BudgetSpec::default(),
            None,
            Vec::new(),
            d2b_contracts_resource::v3::process::TelemetrySpec::default(),
        )
        .expect("execution");
        let process = BundleResource::new(
            ResourceTypeName::parse("Process").expect("process type"),
            BundleResourceMetadata::new(
                ResourceName::parse("controller-test").expect("process name"),
                zone.clone(),
                Some(owner.clone()),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(
                br#"{"domain":"system","executionRef":"Host/dev-host","processClass":"controller","providerRef":"Provider/system-minijail","template":"controller-test"}"#,
            )
            .expect("process spec"),
        )
        .expect("process resource");
        let provider_resource = BundleResource::new(
            ResourceTypeName::parse("Provider").expect("provider type"),
            BundleResourceMetadata::new(
                ResourceName::parse("runtime-cloud-hypervisor").expect("provider name"),
                zone.clone(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(
                br#"{"artifactId":"runtime-cloud-hypervisor","config":{"controllerExecutionRef":"Host/dev-host"}}"#,
            )
            .expect("provider spec"),
        )
        .expect("provider resource");
        let resource_bundle = ResourceBundle::new(
            zone.clone(),
            vec![process, provider_resource],
            format!("sha256:{}", "b".repeat(64)),
            BTreeMap::new(),
            BTreeMap::new(),
            Timestamp::parse("1970-01-01T00:00:00.000Z").expect("timestamp"),
        )
        .expect("resource bundle")
        .with_process_templates(vec![
            d2b_contracts_zone_session::v3::resource_bundle::ProcessTemplateBinding::new(
                process_ref.clone(),
                owner.clone(),
                target.clone(),
                template.clone(),
                d2b_contracts_resource::v3::ArtifactId::parse("runtime-cloud-hypervisor")
                    .expect("artifact"),
                d2b_contracts_provider::v3::BinaryRef::parse("d2b-cloud-hypervisor-controller")
                    .expect("binary"),
                d2b_contracts_provider::v3::ArtifactDigest::parse(format!(
                    "sha256:{}",
                    "a".repeat(64)
                ))
                .expect("digest"),
                "/nix/store/runtime-cloud-hypervisor/bin/d2b-cloud-hypervisor-controller",
            )
            .expect("template binding"),
        ])
        .expect("process templates");
        let host = serde_json::from_str::<d2b_core::host::HostJson>(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host fixture");
        let manifest = d2b_core::manifest_v04::ManifestV04::from_slice(
            include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .expect("manifest fixture");
        let resolver = BundleResolver::from_artifacts_with_zone_resource_bundles(
            Bundle {
                bundle_version: 1,
                schema_version: "v3".to_owned(),
                privileges_path: "privileges.json".to_owned(),
                storage_path: None,
                realm_workloads_launcher_v2_path: None,
                generation: BundleGeneration {
                    generator: "test".to_owned(),
                    source_revision: None,
                    generated_at: None,
                },
                bundle_hash: Some("sha256:bundle".to_owned()),
                artifact_hashes: None,
            },
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            manifest,
            BTreeMap::from([(
                "dev".to_owned(),
                serde_json::to_vec(&resource_bundle).expect("resource bundle bytes"),
            )]),
        );
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid");
        let context = ProcessResourceContext::new(
            zone,
            (
                &process_ref,
                &uid,
                ResourceGeneration::new(1).expect("generation"),
                ZoneRevision::new(1)
            ),
            &provider,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        )
        .with_owner_ref(Some(owner.clone()))
        .with_lifecycle_identity(
            Some(ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").expect("zone uid")),
            Some(1),
            None,
        );
        let ticket = resource_ticket(
            &resolver,
            &context,
            ExecutionIntent {
                execution: &execution,
                activation_input: None,
                spec_bytes: b"process-spec",
                readiness: Some(ReadinessClass::ReadyCondition),
            },
            ManagedProvider::Minijail,
            DaemonMode::Host,
            Duration::from_secs(5),
        )
        .expect("static controller ticket");
        assert_eq!(ticket.inherited_fd_table().count(), 1);
        assert_eq!(ticket.process_ref(), &process_ref);
        assert_eq!(ticket.template(), &template);
        assert!(ticket.zone_uid().is_some());
        assert_eq!(ticket.owner_ref(), Some(&owner));
        assert!(ticket.runtime_scope().is_some());
        let wrong_owner = ResourceRef::parse("Provider/wrong-owner").expect("wrong owner");
        let wrong_owner_context = context.clone().with_owner_ref(Some(wrong_owner));
        assert_eq!(
            resource_ticket(
                &resolver,
                &wrong_owner_context,
                ExecutionIntent {
                    execution: &execution,
                    activation_input: None,
                    spec_bytes: b"process-spec",
                    readiness: Some(ReadinessClass::ReadyCondition),
                },
                ManagedProvider::Minijail,
                DaemonMode::Host,
                Duration::from_secs(5),
            ),
            Err("provider-ticket:template-not-found".to_owned())
        );
    }

    /// The bundle process-DAG ticket path names its own VM in the launch
    /// identity: every node shares the `Host/host-system` execution target, so
    /// only the DAG's `vm` can select the node's trusted runner intent - and
    /// the broker's identity fence reads that value instead of re-deriving the
    /// VM from the execution target.
    #[test]
    fn process_dag_ticket_names_its_vm_in_the_launch_identity() {
        use d2b_core::processes::{NodeId, ProcessNode, ProcessRole};

        let host = serde_json::from_str::<d2b_core::host::HostJson>(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host fixture");
        let manifest = d2b_core::manifest_v04::ManifestV04::from_slice(
            include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .expect("manifest fixture");
        let resolver = BundleResolver::from_artifacts_with_zone_resource_bundles(
            Bundle {
                bundle_version: 1,
                schema_version: "v3".to_owned(),
                privileges_path: "privileges.json".to_owned(),
                storage_path: None,
                realm_workloads_launcher_v2_path: None,
                generation: BundleGeneration {
                    generator: "test".to_owned(),
                    source_revision: None,
                    generated_at: None,
                },
                bundle_hash: Some("sha256:bundle".to_owned()),
                artifact_hashes: None,
            },
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            manifest,
            BTreeMap::new(),
        );
        let node = ProcessNode {
            execution_ref: None,
            execution_domain: None,
            user_ref: None,
            id: NodeId("virtiofsd-worker".to_owned()),
            role: ProcessRole::Virtiofsd,
            unit: None,
            binary_path: None,
            argv: vec![],
            env: vec![],
            plan_ops: vec![],
            network_interfaces: Vec::new(),
            profile: d2b_core::test_support::RoleProfileBuilder::new()
                .with_profile_id("virtiofsd-worker")
                .with_uid(0)
                .with_gid(0)
                .build(),
            readiness: vec![],
        };

        let ticket = build_ticket(
            &resolver,
            "corp-vm",
            &node,
            ManagedProvider::Minijail,
            Duration::from_secs(5),
        )
        .expect("bundle node ticket");
        let identity = ticket.launch_identity();
        assert_eq!(identity.vm(), Some("corp-vm"));
        assert_eq!(identity.launch_vm(), "corp-vm");
        assert_eq!(identity.role(), "virtiofsd-worker");
        assert_eq!(
            identity.execution_ref().resource_type().as_str(),
            "Host",
            "the DAG node executes on the shared host target"
        );
    }

    /// A declared Device-owned worker row's ticket resolves through the
    /// Device Provider's signed template binding for that exact declared row
    /// (never the guest VMM chain), and the ticket carries the row's declared
    /// template even though the resolved intent's role id is the row name.
    #[test]
    fn device_worker_resource_ticket_resolves_the_declared_row_intent() {
        let zone = ZoneId::parse("dev").expect("zone");
        let device_ref = ResourceRef::parse("Device/tpm-0").expect("device ref");
        let device_owner = ResourceRef::parse("Guest/dev").expect("guest ref");
        let provider = ResourceRef::parse("Provider/system-minijail").expect("provider");
        let binding_owner = ResourceRef::parse("Provider/device-tpm").expect("binding owner");
        let target = ResourceRef::parse("Host/dev-host").expect("target");
        let process_ref = ResourceRef::parse("Process/swtpm-tpm-0").expect("process ref");
        let template = BoundedToken::parse("swtpm-socket").expect("template");
        let execution = d2b_contracts_resource::v3::process::ExecutionSpec::new(
            target.clone(),
            Some(ExecutionDomain::System),
            None,
            ProcessClass::Worker,
            template.clone(),
            None,
            Vec::new(),
            Vec::new(),
            d2b_contracts_resource::v3::process::SandboxSpec::default(),
            d2b_contracts_resource::v3::execution_policy::BudgetSpec::default(),
            None,
            Vec::new(),
            d2b_contracts_resource::v3::process::TelemetrySpec::default(),
        )
        .expect("execution");
        let process = BundleResource::new(
            ResourceTypeName::parse("Process").expect("process type"),
            BundleResourceMetadata::new(
                ResourceName::parse("swtpm-tpm-0").expect("process name"),
                zone.clone(),
                Some(device_ref.clone()),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(
                br#"{"domain":"system","executionRef":"Host/dev-host","processClass":"worker","providerRef":"Provider/system-minijail","template":"swtpm-socket"}"#,
            )
            .expect("process spec"),
        )
        .expect("process resource");
        let device = BundleResource::new(
            ResourceTypeName::parse("Device").expect("device type"),
            BundleResourceMetadata::new(
                ResourceName::parse("tpm-0").expect("device name"),
                zone.clone(),
                Some(device_owner),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(br#"{"providerRef":"Provider/device-tpm"}"#)
                .expect("device spec"),
        )
        .expect("device resource");
        let provider_resource = BundleResource::new(
            ResourceTypeName::parse("Provider").expect("provider type"),
            BundleResourceMetadata::new(
                ResourceName::parse("device-tpm").expect("provider name"),
                zone.clone(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(
                br#"{"artifactId":"device-tpm","config":{"controllerExecutionRef":"Host/dev-host"}}"#,
            )
            .expect("provider spec"),
        )
        .expect("provider resource");
        let resource_bundle = ResourceBundle::new(
            zone.clone(),
            vec![process, device, provider_resource],
            format!("sha256:{}", "b".repeat(64)),
            BTreeMap::new(),
            BTreeMap::new(),
            Timestamp::parse("1970-01-01T00:00:00.000Z").expect("timestamp"),
        )
        .expect("resource bundle")
        .with_process_templates(vec![
            d2b_contracts_zone_session::v3::resource_bundle::ProcessTemplateBinding::new_with_launch_args(
                process_ref.clone(),
                binding_owner,
                target.clone(),
                template.clone(),
                d2b_contracts_resource::v3::ArtifactId::parse("device-tpm").expect("artifact"),
                d2b_contracts_provider::v3::BinaryRef::parse("swtpm").expect("binary"),
                d2b_contracts_provider::v3::ArtifactDigest::parse(format!(
                    "sha256:{}",
                    "a".repeat(64)
                ))
                .expect("digest"),
                "/nix/store/device-tpm/bin/swtpm",
            )
            .expect("template binding"),
        ])
        .expect("process templates");
        let host = serde_json::from_str::<d2b_core::host::HostJson>(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host fixture");
        let manifest = d2b_core::manifest_v04::ManifestV04::from_slice(
            include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .expect("manifest fixture");
        let resolver = BundleResolver::from_artifacts_with_zone_resource_bundles(
            Bundle {
                bundle_version: 1,
                schema_version: "v3".to_owned(),
                privileges_path: "privileges.json".to_owned(),
                storage_path: None,
                realm_workloads_launcher_v2_path: None,
                generation: BundleGeneration {
                    generator: "test".to_owned(),
                    source_revision: None,
                    generated_at: None,
                },
                bundle_hash: Some("sha256:bundle".to_owned()),
                artifact_hashes: None,
            },
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            manifest,
            BTreeMap::from([(
                "dev".to_owned(),
                serde_json::to_vec(&resource_bundle).expect("resource bundle bytes"),
            )]),
        );
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid");
        let zone_uid =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").expect("zone uid");
        let context = ProcessResourceContext::new(
            zone.clone(),
            (
                &process_ref,
                &uid,
                ResourceGeneration::new(1).expect("generation"),
                ZoneRevision::new(1)
            ),
            &provider,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        )
        .with_owner_ref(Some(device_ref.clone()))
        .with_lifecycle_identity(Some(zone_uid.clone()), Some(1), None);
        let ticket = resource_ticket(
            &resolver,
            &context,
            ExecutionIntent {
                execution: &execution,
                activation_input: None,
                spec_bytes: b"process-spec",
                readiness: Some(ReadinessClass::ReadyCondition),
            },
            ManagedProvider::Minijail,
            DaemonMode::Host,
            Duration::from_secs(5),
        )
        .expect("device worker ticket");
        assert_eq!(ticket.template(), &template);
        assert_eq!(ticket.process_ref(), &process_ref);
        assert_eq!(ticket.owner_ref(), Some(&device_ref));
        assert_eq!(ticket.execution_ref(), &target);
        assert_eq!(ticket.launch_identity().role(), "swtpm-tpm-0");
        // The declared row name is the launch identity: a Device-owned row
        // with no binding of its own still resolves through the
        // device-worker lookup (never the template's other row and never the
        // guest VMM chain), and refuses by name.
        let other_ref = ResourceRef::parse("Process/swtpm-other").expect("other ref");
        let other_context = ProcessResourceContext::new(
            zone,
            (
                &other_ref,
                &uid,
                ResourceGeneration::new(1).expect("generation"),
                ZoneRevision::new(1)
            ),
            &provider,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        )
        .with_owner_ref(Some(device_ref))
        .with_lifecycle_identity(Some(zone_uid), Some(1), None);
        assert_eq!(
            resource_ticket(
                &resolver,
                &other_context,
                ExecutionIntent {
                    execution: &execution,
                    activation_input: None,
                    spec_bytes: b"process-spec",
                    readiness: Some(ReadinessClass::ReadyCondition),
                },
                ManagedProvider::Minijail,
                DaemonMode::Host,
                Duration::from_secs(5),
            ),
            Err("provider-ticket:template-not-found".to_owned())
        );
    }

    /// A declared Device-owned one-shot worker row's ticket carries the typed
    /// launch arguments the daemon composed for it - the TPM pre-start flush
    /// (`swtpm_ioctl -i --unix <ctrl>`) - and a row with no typed launch keeps
    /// the bare template argv.
    ///
    /// This is the regression the ephemeral launch path must not reintroduce:
    /// `mint_template_intent` renders `[binary_ref]` for a Device-worker
    /// template, so a ticket built without the attach makes the one-shot
    /// worker run with no ctrl socket at all (and the broker's typed
    /// `w1-swtpm` fence then refuses the launch).
    #[test]
    fn ephemeral_device_worker_ticket_carries_the_composed_flush_argv() {
        let zone = ZoneId::parse("dev").expect("zone");
        let device_ref = ResourceRef::parse("Device/tpm-0").expect("device ref");
        let device_owner = ResourceRef::parse("Guest/dev").expect("guest ref");
        let provider = ResourceRef::parse("Provider/system-minijail").expect("provider");
        let binding_owner = ResourceRef::parse("Provider/device-tpm").expect("binding owner");
        let target = ResourceRef::parse("Host/dev-host").expect("target");
        let process_ref =
            ResourceRef::parse("EphemeralProcess/swtpm-flush-tpm-0").expect("process ref");
        let template = BoundedToken::parse("swtpm-init-flush").expect("template");
        let execution = d2b_contracts_resource::v3::process::ExecutionSpec::new(
            target.clone(),
            Some(ExecutionDomain::System),
            None,
            ProcessClass::Worker,
            template.clone(),
            None,
            Vec::new(),
            Vec::new(),
            d2b_contracts_resource::v3::process::SandboxSpec::default(),
            d2b_contracts_resource::v3::execution_policy::BudgetSpec::default(),
            None,
            Vec::new(),
            d2b_contracts_resource::v3::process::TelemetrySpec::default(),
        )
        .expect("execution");
        let state_dir = "/var/lib/d2b/tpm-state/device-123e4567e89b42d3a456426614174000-tpm-state";
        let ctrl_socket = format!("{state_dir}/ctrl.sock");
        let flush = BundleResource::new(
            ResourceTypeName::parse("EphemeralProcess").expect("process type"),
            BundleResourceMetadata::new(
                ResourceName::parse("swtpm-flush-tpm-0").expect("process name"),
                zone.clone(),
                Some(device_ref.clone()),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(
                br#"{"domain":"system","executionRef":"Host/dev-host","processClass":"worker","providerRef":"Provider/system-minijail","template":"swtpm-init-flush"}"#,
            )
            .expect("process spec"),
        )
        .expect("process resource");
        let device = BundleResource::new(
            ResourceTypeName::parse("Device").expect("device type"),
            BundleResourceMetadata::new(
                ResourceName::parse("tpm-0").expect("device name"),
                zone.clone(),
                Some(device_owner),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(br#"{"providerRef":"Provider/device-tpm"}"#)
                .expect("device spec"),
        )
        .expect("device resource");
        let provider_resource = BundleResource::new(
            ResourceTypeName::parse("Provider").expect("provider type"),
            BundleResourceMetadata::new(
                ResourceName::parse("device-tpm").expect("provider name"),
                zone.clone(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(
                br#"{"artifactId":"device-tpm","config":{"controllerExecutionRef":"Host/dev-host"}}"#,
            )
            .expect("provider spec"),
        )
        .expect("provider resource");
        let resource_bundle = ResourceBundle::new(
            zone.clone(),
            vec![flush, device, provider_resource],
            format!("sha256:{}", "b".repeat(64)),
            BTreeMap::new(),
            BTreeMap::new(),
            Timestamp::parse("1970-01-01T00:00:00.000Z").expect("timestamp"),
        )
        .expect("resource bundle")
        .with_process_templates(vec![
            d2b_contracts_zone_session::v3::resource_bundle::ProcessTemplateBinding::new_with_launch_args(
                process_ref.clone(),
                binding_owner,
                target.clone(),
                template.clone(),
                d2b_contracts_resource::v3::ArtifactId::parse("device-tpm").expect("artifact"),
                d2b_contracts_provider::v3::BinaryRef::parse("swtpm-ioctl").expect("binary"),
                d2b_contracts_provider::v3::ArtifactDigest::parse(format!(
                    "sha256:{}",
                    "a".repeat(64)
                ))
                .expect("digest"),
                "/nix/store/device-tpm/bin/swtpm-ioctl",
            )
            .expect("template binding"),
        ])
        .expect("process templates");
        let host = serde_json::from_str::<d2b_core::host::HostJson>(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host fixture");
        let manifest = d2b_core::manifest_v04::ManifestV04::from_slice(
            include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .expect("manifest fixture");
        let resolver = BundleResolver::from_artifacts_with_zone_resource_bundles(
            Bundle {
                bundle_version: 1,
                schema_version: "v3".to_owned(),
                privileges_path: "privileges.json".to_owned(),
                storage_path: None,
                realm_workloads_launcher_v2_path: None,
                generation: BundleGeneration {
                    generator: "test".to_owned(),
                    source_revision: None,
                    generated_at: None,
                },
                bundle_hash: Some("sha256:bundle".to_owned()),
                artifact_hashes: None,
            },
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            manifest,
            BTreeMap::from([(
                "dev".to_owned(),
                serde_json::to_vec(&resource_bundle).expect("resource bundle bytes"),
            )]),
        );
        let uid = ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").expect("uid");
        let zone_uid =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").expect("zone uid");
        let context = ProcessResourceContext::new(
            zone.clone(),
            (
                &process_ref,
                &uid,
                ResourceGeneration::new(1).expect("generation"),
                ZoneRevision::new(1)
            ),
            &provider,
            ControllerGeneration::new(1).expect("controller generation"),
            None,
        )
        .with_owner_ref(Some(device_ref.clone()))
        .with_lifecycle_identity(Some(zone_uid), Some(1), None)
        .with_device_worker_launch(Some(DeviceWorkerLaunch::SwtpmFlush(Box::new(
            SwtpmFlushParams {
                ioctl_binary_path: PathBuf::from("/nix/store/device-tpm/bin/swtpm_ioctl"),
                vm_name: "dev".to_owned(),
                ctrl_socket_path: PathBuf::from(&ctrl_socket),
            },
        ))));
        let spec_bytes = serde_json::to_vec(
            &d2b_provider_device_tpm::build_swtpm_flush_spec(&device_ref, "dev", &target)
                .expect("flush spec"),
        )
        .expect("spec bytes");
        let ticket = ephemeral_launch_ticket(
            &resolver,
            std::path::Path::new("/run/d2b"),
            &context,
            ExecutionIntent {
                execution: &execution,
                activation_input: None,
                spec_bytes: &spec_bytes,
                readiness: None,
            },
            ManagedProvider::Minijail,
            DaemonMode::Host,
            Duration::from_secs(5),
        )
        .expect("ephemeral device worker ticket");
        assert_eq!(ticket.template(), &template);
        assert_eq!(ticket.process_ref(), &process_ref);
        assert_eq!(ticket.owner_ref(), Some(&device_ref));
        assert_eq!(
            ticket.launch_args().to_vec(),
            vec!["-i".to_owned(), "--unix".to_owned(), ctrl_socket.clone()],
            "the one-shot flush ticket must carry the composed ctrl socket"
        );
        // No typed launch -> no controller-supplied arguments: the attach is
        // the declared Device worker's, never a blanket default.
        let bare = context.with_device_worker_launch(None);
        let bare_ticket = ephemeral_launch_ticket(
            &resolver,
            std::path::Path::new("/run/d2b"),
            &bare,
            ExecutionIntent {
                execution: &execution,
                activation_input: None,
                spec_bytes: &spec_bytes,
                readiness: None,
            },
            ManagedProvider::Minijail,
            DaemonMode::Host,
            Duration::from_secs(5),
        )
        .expect("ephemeral ticket without a typed launch");
        assert!(bare_ticket.launch_args().is_empty());
        assert_eq!(bare_ticket.template(), &template);
    }

    // -- Device-family resolution (U17 gap closure) --------------------------
    //
    // The Device-family-specific resolution (the owning Device's declared
    // settings, the controller-created state Volume, the projected Wayland
    // socket) is owned by this daemon seat and may name the device families;
    // the family crate receives the already-resolved typed parameters.

    /// A Device that declares GPU settings keeps them, a Device that declares
    /// none keeps the Provider's bounded default, and a present payload that
    /// does not decode refuses with its own code instead of silently becoming
    /// the default - whose `CrossDomain` context class the Device never
    /// declared.
    #[test]
    fn device_gpu_settings_refuse_an_undecodable_declaration() {
        let declared = br#"{"providerRef":"Provider/device-gpu","provider":{"schemaId":"device-gpu.d2bus.org/Device/spec","schemaVersion":"1.0","settings":{"contextTypes":["virgl"],"displays":[{"hidden":false}],"egl":false,"vulkan":false}}}"#;
        let settings =
            decode_device_gpu_settings(declared).expect("declared settings decode");
        assert_eq!(
            settings.context_types,
            vec![d2b_provider_device_gpu::ContextType::Virgl]
        );
        assert!(!settings.egl, "the declared setting wins over the default");
        assert!(
            decode_device_gpu_settings(br#"{"providerRef":"Provider/device-gpu"}"#)
                .expect("absent settings keep the default")
                == d2b_provider_device_gpu::GpuSettings::default(),
            "a Device that declares nothing keeps the Provider default"
        );
        let undecodable = br#"{"providerRef":"Provider/device-gpu","provider":{"schemaId":"device-gpu.d2bus.org/Device/spec","schemaVersion":"1.0","settings":{"contextTypes":["bogus"]}}}"#;
        assert_eq!(
            decode_device_gpu_settings(undecodable),
            Err("device-worker-gpu-settings-invalid"),
            "an undecodable declaration is never read as absent"
        );
        assert_eq!(
            decode_device_gpu_settings(b"{not-json"),
            Err("device-worker-device-row-unreadable")
        );
    }

    /// The owning Device's `videoNvidiaDecode` setting and the declared
    /// video row's template are one decision: the NVIDIA posture binds its
    /// device nodes only through the `video-worker-nvidia` template, so each
    /// disagreement refuses by name instead of launching a sidecar where the
    /// setting (on the plain template) or the template (on a Device that
    /// turned the setting off) is silently ignored.
    #[test]
    fn video_nvidia_posture_refuses_a_setting_template_mismatch() {
        let mut settings = d2b_provider_device_gpu::GpuSettings::default();
        assert!(!settings.video_nvidia_decode, "the default posture is plain");
        assert_eq!(video_nvidia_posture("video-worker", &settings), Ok(()));
        assert_eq!(
            video_nvidia_posture("video-worker-nvidia", &settings),
            Err("device-worker-nvidia-posture-mismatch"),
            "the NVIDIA template without its setting is a refusal"
        );

        settings.video_nvidia_decode = true;
        assert_eq!(
            video_nvidia_posture("video-worker-nvidia", &settings),
            Ok(())
        );
        assert_eq!(
            video_nvidia_posture("video-worker", &settings),
            Err("device-worker-nvidia-posture-mismatch"),
            "the setting on the plain template is never a silent no-op"
        );
    }

    /// The host Wayland socket is trusted bundle data (`site.json`), never a
    /// daemon-derived path: the reader resolves the projected value and refuses
    /// by name when the bundle carries none, so a bundle that predates the
    /// artifact - or a site without a Wayland session - cannot launch the GPU
    /// worker against an invented socket.
    #[test]
    fn gpu_worker_wayland_sock_reads_the_projected_site_and_refuses_without_it() {
        let site = SiteJson {
            schema_version: "v1".to_owned(),
            wayland_socket: Some("/run/user/1001/wayland-7".to_owned()),
        };
        assert_eq!(
            gpu_worker_wayland_sock(Some(&site)),
            Ok(std::path::PathBuf::from("/run/user/1001/wayland-7")),
            "the socket is exactly the bundle-projected value"
        );

        let headless = SiteJson {
            schema_version: "v1".to_owned(),
            wayland_socket: None,
        };
        assert_eq!(
            gpu_worker_wayland_sock(Some(&headless)),
            Err("device-worker-wayland-sock-unbound")
        );
        assert_eq!(
            gpu_worker_wayland_sock(None),
            Err("device-worker-wayland-sock-unbound"),
            "a bundle that predates site.json keeps the GPU launch refused by name"
        );
    }

    // -- Launched-runner pidfd-table registration -----------------------------
    //
    // A failed launch (e.g. a readiness-probe envelope timeout) stops the
    // spawned child but never clears the daemon's pidfd-table slot for its
    // (vm, role). The next launch's observer registration would hit the
    // duplicate guard and be swallowed, leaving the probe and the stop path
    // reading the dead pid. The observer must replace the stale entry.

    #[test]
    fn launched_observer_replaces_stale_pidfd_table_entry_on_relaunch() {
        use d2b_provider_supervisor::LaunchedObserver;
        use d2bd_runtime::supervisor::pidfd_table::PidfdTable;

        let state_path = std::env::temp_dir().join(format!(
            "d2b-test-pidfd-observer-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos()
        ));
        let table = Arc::new(PidfdTable::new(state_path.clone()));
        // The stale slot: a (vm, role) whose recorded process can never be
        // alive (a pid above pid_max).
        table
            .register(
                "host-system".to_owned(),
                "controller-stale".to_owned(),
                d2bd_runtime::supervisor::pidfd_table::PidfdEntry {
                    pidfd: std::fs::File::open("/dev/null").expect("null").into(),
                    pid: i32::MAX,
                    start_time_ticks: 1,
                },
            )
            .expect("stale entry registers");
        let observer = PidfdTableLaunchedObserver {
            pidfd_table: Arc::clone(&table),
        };
        // The relaunch: a fresh broker-confirmed spawn for the same slot.
        let live_pid = std::process::id() as i32;
        observer.launched(
            "host-system",
            "controller-stale",
            live_pid,
            2,
            std::fs::File::open("/dev/null").expect("null").into(),
        );
        let registration = table
            .list_for_vm("host-system")
            .into_iter()
            .find(|registration| registration.role == "controller-stale")
            .expect("the relaunched runner is registered");
        assert_eq!(
            registration.pid, live_pid,
            "the stale dead pid must be replaced by the relaunched runner"
        );
        let _ = std::fs::remove_file(&state_path);
    }

    #[test]
    fn launched_observer_keeps_a_live_duplicate_slot() {
        use d2b_provider_supervisor::LaunchedObserver;
        use d2bd_runtime::supervisor::pidfd_table::PidfdTable;

        let state_path = std::env::temp_dir().join(format!(
            "d2b-test-pidfd-observer-live-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos()
        ));
        let table = Arc::new(PidfdTable::new(state_path.clone()));
        // A slot whose recorded process is this test process itself: alive
        // with a matching start time.
        let live_pid = std::process::id() as i32;
        let start_time_ticks = d2bd_runtime::supervisor::pidfd_table::read_proc_start_time_pub(
            live_pid,
        )
        .expect("own proc start time")
        .expect("own proc is alive");
        table
            .register(
                "host-system".to_owned(),
                "controller-live".to_owned(),
                d2bd_runtime::supervisor::pidfd_table::PidfdEntry {
                    pidfd: std::fs::File::open("/dev/null").expect("null").into(),
                    pid: live_pid,
                    start_time_ticks,
                },
            )
            .expect("live entry registers");
        let observer = PidfdTableLaunchedObserver {
            pidfd_table: Arc::clone(&table),
        };
        observer.launched(
            "host-system",
            "controller-live",
            live_pid,
            start_time_ticks,
            std::fs::File::open("/dev/null").expect("null").into(),
        );
        let registration = table
            .list_for_vm("host-system")
            .into_iter()
            .find(|registration| registration.role == "controller-live")
            .expect("the live entry is kept");
        assert_eq!(
            registration.pid, live_pid,
            "a live duplicate slot is kept untouched (concurrent-spawn guard)"
        );
        let _ = std::fs::remove_file(&state_path);
    }
}
