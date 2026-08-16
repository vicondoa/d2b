//! Daemon-owned composition for authenticated interaction Providers.
//!
//! This is the only layer that may join a sealed ComponentSession admission
//! to process effects.  Provider crates receive authenticated sessions and
//! opaque evidence; they never construct a session, resolve a process, or
//! retain a persistent service unit.

use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    os::fd::AsFd,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::resource_runtime::{
    CommittedClipboardProviderConfiguration, CommittedInteractionProviderConfiguration,
    CommittedNotificationProviderConfiguration,
};
use d2b_bus::{
    BusAuthorizer, BusConfig, BusError, BusIngress, ComponentRequestReceiver,
    ComponentSessionAdmission, NoopBusObserver, OperationId, OperationSpec, RouteGenerations,
    RouteKey, RouteMember, RouteTarget, ZoneBus, ZoneRegistrar,
};
use d2b_contracts::v3::{
    ControllerGeneration, EvidenceClass, ResourceGeneration, ResourceRef, ResourceUid, ServiceName,
    ZoneId, ZoneRevision,
    component_session::{
        AttachmentKind, AttachmentPolicy, AttachmentPolicyKind, AttachmentPurpose, EndpointPolicy,
        EndpointPurpose, EndpointRole, IdentityEvidenceRequirement, LimitProfile,
        Locality as TransportLocality, NoiseProfile, PurposeClass, ServicePackage,
        TransportBinding, TransportClass,
    },
    execution_policy::{BoundedToken, ExecutionDomain},
    process::{CapabilityClass, EnvironmentClass, MappingClass, NamespaceClass, UserNamespaceSpec},
};
use d2b_contracts::{
    broker_wire::{BrokerCallerRole, RunnerRole, SandboxLaunchPlan},
    types::{BundleOpId, RoleId, VmId},
};
use d2b_core::{
    bundle_resolver::{BundleResolver, ResolvedRunnerIntent},
    processes::ProcessRole,
};
use d2b_process::{
    CompiledDigests, IdentityBinding, LaunchTicket as ProcessLaunchTicket, ProcessEffectError,
    ProcessIdentityDigest, ProcessLaunchEffectPort, ProcessRequest, StopClass,
};
use d2b_process_conformance::ReadinessExpectation;
use d2b_process_conformance::SandboxCompiler;
use d2b_provider_clipboard_wayland::{
    AttachmentClass, ClipboardProcessEffectPort, ClipboardServiceError,
};
use d2b_provider_display_wayland::{
    AuthenticatedDisplaySession, CleanupState, DependencyState, DisplayController,
    DisplayDependencyProof, DisplayLaunchBinding, DisplayProcessEffectPort, DisplayProcessRole,
    DisplayRuntime, DisplayRuntimeError, FilterInput, LaunchGrants, VolumeState,
    WaylandPolicySnapshot, WaylandSessionSpec, WorkerEffectError, WorkerLaunchReceipt,
    WorkerRestartEvidence, WorkerState,
};
#[cfg(test)]
use d2b_provider_notification_desktop::Category;
use d2b_provider_notification_desktop::{
    DesktopNotificationPort, NotificationProcessEffectPort, NotificationRequest,
    SourceProcessEffectPort, SourceProcessEffectReceipt, SourceReconcileResult,
};
use d2b_provider_supervisor::{
    BrokerLaunchIntent, BrokerLaunchResolver, BrokerObservedProcess, BrokerProcessBackend,
    NotificationHostSinkIdentity, NotificationLifecycleBackend, NotificationLifecycleObservation,
    NotificationLifecyclePlan, NotificationLifecycleSupervisor, NotificationSourceIdentity,
    ProviderSupervisor,
};
use d2b_resource_api::authz::{
    ApiCatalog, BindingScope, BootstrapPhase, BoundSubject, CompiledRole, CompiledRoleBinding,
    NativeAuthorizer, PolicyRule, PolicySet, SessionVerb,
};
use d2b_resource_store::PolicySnapshot;
use d2b_session::{
    AuthenticatedSessionRouteBinding, OwnedAttachment, OwnedTransport, SessionAcceptor,
    SessionEngine, TransportEvidence, operation_catalog_entry, ttrpc_stream_id,
};
use d2b_session_unix::{
    CreditPool, CreditScopeSet, PeerIdentityPolicy, SeqpacketSocket, UnixSeqpacketTransport,
    UnixSessionError, VerifiedPacket, VerifiedUnixPeer,
};
use nix::unistd::{Group, getgid};
use notify_rust::{Notification as DesktopNotification, Urgency};
use protobuf::Message;
use rustix::net::{SocketFlags, accept_with};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use socket2::{Domain, SockAddr, Socket, Type};
use tokio::sync::Mutex as AsyncMutex;
use ttrpc::proto::{
    Code as TtrpcCode, MessageHeader, Request as TtrpcRequest, Response as TtrpcResponse,
    Status as TtrpcStatus,
};

/// Errors at the daemon's authenticated Provider admission seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionAdmissionError {
    /// The session handshake or authentication failed.
    SessionAdmission,
    /// Zone registration rejected the authenticated candidate.
    Registration,
    /// The authenticated service could not install its daemon-owned runtime.
    ServiceUnavailable,
}

const INTERACTION_SERVICES: &[(&str, ServicePackage)] = &[
    (
        d2b_provider_display_wayland::SERVICE_PACKAGE,
        ServicePackage::DisplayV3,
    ),
    (
        d2b_provider_clipboard_wayland::MANAGEMENT_SERVICE,
        ServicePackage::ClipboardV3,
    ),
    (
        d2b_provider_clipboard_wayland::BRIDGE_SERVICE,
        ServicePackage::ClipboardBridgeV3,
    ),
    (
        d2b_provider_clipboard_wayland::PICKER_SERVICE,
        ServicePackage::ClipboardPickerCoordV3,
    ),
    (
        d2b_provider_notification_desktop::SERVICE_PACKAGE,
        ServicePackage::NotificationV3,
    ),
];

/// Return the exact ComponentSession policy for one daemon-owned interaction
/// listener.  Each service has a distinct Unix socket so the handshake offer
/// cannot select a different Provider after the listener policy is chosen.
pub fn interaction_endpoint_policy(service: &str, generation: u64) -> Option<EndpointPolicy> {
    let (_, package) = INTERACTION_SERVICES
        .iter()
        .find(|(candidate, _)| *candidate == service)?;
    let attachment_policy = AttachmentPolicy {
        kind: AttachmentPolicyKind::PacketAtomic,
        max_per_packet: 2,
        max_per_request: 2,
        max_per_operation: 2,
        max_per_session: 8,
        credentials_allowed: false,
    };
    Some(EndpointPolicy {
        purpose: EndpointPurpose::ProviderControl,
        purpose_class: PurposeClass::Local,
        initiator_role: EndpointRole::Provider,
        responder_role: EndpointRole::ZoneController,
        service: *package,
        schema_fingerprint: [0x11; 32],
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: TransportLocality::HostLocal,
            channel_binding: [0x22; 32],
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: generation,
        attachment_policy,
    })
}

fn binding_digest(policy: &EndpointPolicy) -> d2b_contracts::v3::BindingDigest {
    d2b_contracts::v3::BindingDigest::parse(format!(
        "sha256:{}",
        policy
            .transport_binding
            .channel_binding
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
    .expect("fixed interaction channel binding is a valid digest")
}

/// A registered interaction session after the daemon has consumed its sealed
/// registration capability.
///
/// The live authority remains owned by the Zone bus endpoint.  Provider
/// runtimes receive only this authenticated route projection for dispatch and
/// evidence checks; they never mint or retain a second session authority.
pub struct RegisteredInteractionSession {
    ingress: BusIngress,
    route: AuthenticatedSessionRouteBinding,
}

impl RegisteredInteractionSession {
    /// Borrow the registered bus ingress, whose drop closes the route.
    pub const fn ingress(&self) -> &BusIngress {
        &self.ingress
    }

    /// Borrow the authenticated route projection for Provider dispatch.
    pub const fn route(&self) -> &AuthenticatedSessionRouteBinding {
        &self.route
    }

    /// Return the exact service package admitted for this session.
    pub fn service(&self) -> &d2b_contracts::v3::ServiceName {
        self.route.service()
    }

    /// Return the stable daemon-local key for this authenticated session.
    pub fn session_key(&self) -> String {
        interaction_session_key(&self.route)
    }

    /// Clone the daemon-owned request receiver demultiplexed by the bus.
    pub fn request_receiver(&self) -> ComponentRequestReceiver {
        self.ingress.component_request_receiver()
    }
}

fn interaction_session_key(route: &AuthenticatedSessionRouteBinding) -> String {
    format!(
        "{}|{}|{}|{}",
        route.zone().as_str(),
        route.service().as_str(),
        route.subject_uid().as_str(),
        route.reconnect_generation().get()
    )
}

impl core::fmt::Display for InteractionAdmissionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::SessionAdmission => "interaction-session-admission-failed",
            Self::Registration => "interaction-session-registration-failed",
            Self::ServiceUnavailable => "interaction-service-unavailable",
        })
    }
}

impl std::error::Error for InteractionAdmissionError {}

impl From<BusError> for InteractionAdmissionError {
    fn from(_: BusError) -> Self {
        Self::Registration
    }
}

/// Authenticate one transport and register it in the Zone bus.
///
/// The registrar consumes the single-use admission capability.  No Provider
/// code can call this function with a fabricated subject or a caller-owned
/// session token.
pub async fn admit_and_register<T>(
    registrar: &mut ZoneRegistrar,
    acceptor: SessionAcceptor<ComponentSessionAdmission>,
    engine: SessionEngine<T>,
    evidence: TransportEvidence,
    now_tick: u64,
) -> Result<BusIngress, InteractionAdmissionError>
where
    T: OwnedTransport + 'static,
{
    Ok(
        admit_and_register_with_route(registrar, acceptor, engine, evidence, now_tick)
            .await?
            .ingress,
    )
}

/// Authenticate, register, and return the daemon-owned route projection for
/// Provider runtime dispatch.
pub async fn admit_and_register_with_route<T>(
    registrar: &mut ZoneRegistrar,
    acceptor: SessionAcceptor<ComponentSessionAdmission>,
    engine: SessionEngine<T>,
    evidence: TransportEvidence,
    now_tick: u64,
) -> Result<RegisteredInteractionSession, InteractionAdmissionError>
where
    T: OwnedTransport + 'static,
{
    let session = acceptor
        .admit(engine, evidence, now_tick)
        .await
        .map_err(|_| InteractionAdmissionError::SessionAdmission)?;
    let route = session.route_binding();
    let ingress = registrar
        .register_component_session(session)
        .await
        .map_err(InteractionAdmissionError::from)?;
    Ok(RegisteredInteractionSession { ingress, route })
}

/// The daemon-owned composition for one authenticated interaction session.
///
/// This object is intentionally the only place where a registered bus ingress,
/// Provider runtime state, and supervisor effect owner meet.  The Provider
/// crates never receive the registrar or supervisor directly.  Dropping the
/// composition without calling [`Self::finalize`] is safe but leaves the
/// ingress open until its normal owner is dropped; production shutdown calls
/// `finalize` before releasing the ingress.
pub struct InteractionComposition<S, G = UnavailableGuestFrontendEffects>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    registrar: ZoneRegistrar,
    supervisor: S,
    guest_frontend: G,
    sessions: BTreeMap<String, RegisteredInteractionSession>,
    display: Option<DisplayRuntime<DisplaySupervisorEffects<S, G>>>,
    clipboard: Option<d2b_provider_clipboard_wayland::ClipboardRuntime<InteractionDrainEffects>>,
    notification:
        Option<d2b_provider_notification_desktop::NotificationRuntime<InteractionDrainEffects>>,
    pending_picker_receipts: BTreeMap<String, d2b_provider_clipboard_wayland::PickerReceipt>,
    pending_guest_selection_events:
        BTreeMap<String, d2b_provider_clipboard_wayland::GuestSelectionEvent>,
    notification_port: Arc<Mutex<Box<dyn DesktopNotificationPort + Send>>>,
    display_resource_evidence: Option<CoreDisplayResourceEvidence>,
    clipboard_configuration: Option<CommittedClipboardProviderConfiguration>,
    notification_configuration: Option<CommittedNotificationProviderConfiguration>,
}

/// Daemon-owned collection of independently Zone-bound compositions.
pub struct InteractionRuntimeSet<S, G = UnavailableGuestFrontendEffects>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    runtimes: BTreeMap<String, InteractionComposition<S, G>>,
}

impl<S, G> core::fmt::Debug for InteractionRuntimeSet<S, G>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InteractionRuntimeSet")
            .field("zone_count", &self.runtimes.len())
            .finish()
    }
}

impl<S, G> InteractionRuntimeSet<S, G>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    /// Construct an empty Zone runtime set.
    pub fn new() -> Self {
        Self {
            runtimes: BTreeMap::new(),
        }
    }

    /// Insert one fully Zone-bound runtime.
    pub fn insert(&mut self, zone: ZoneId, runtime: InteractionComposition<S, G>) {
        self.runtimes.insert(zone.as_str().to_owned(), runtime);
    }

    /// Return whether any Zone runtime is installed.
    pub fn is_empty(&self) -> bool {
        self.runtimes.is_empty()
    }

    fn runtime_for(&self, zone: &ZoneId) -> Option<&InteractionComposition<S, G>> {
        self.runtimes.get(zone.as_str())
    }

    fn runtime_for_mut(&mut self, zone: &ZoneId) -> Option<&mut InteractionComposition<S, G>> {
        self.runtimes.get_mut(zone.as_str())
    }

    async fn remove_session(&mut self, zone: &ZoneId, session_key: &str) -> Result<(), String> {
        self.runtime_for_mut(zone)
            .ok_or_else(|| "interaction runtime unavailable".to_owned())?
            .remove_session(session_key)
            .await
    }

    /// Finalize every Zone composition, retaining failed state for retry.
    pub async fn finalize_async(
        &mut self,
        grace: d2b_provider_display_wayland::GraceState,
    ) -> Result<(), InteractionFinalizeError> {
        let zones = self.runtimes.keys().cloned().collect::<Vec<_>>();
        let mut failure = None;
        for zone in zones {
            if let Some(runtime) = self.runtimes.get_mut(&zone)
                && let Err(error) = runtime.finalize_async(grace).await
            {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl<S, G> Default for InteractionRuntimeSet<S, G>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionDispatchError {
    SessionUnavailable,
    MalformedRequest,
    ServiceMismatch,
    UnknownOperation,
    InvalidPayload,
    RuntimeFailure,
    ResponseFailed,
}

/// Daemon-owned guest frontend lifecycle port. The Provider only asks for
/// typed ensure/stop effects; the implementation owns guest-control
/// authentication, service identity, and restart/adoption evidence.
pub trait GuestFrontendEffectPort: Clone + Send {
    fn ensure(
        &mut self,
        guest: &ResourceRef,
        policy_generation: u64,
        teardown_generation: u64,
        session_digest: [u8; 32],
    ) -> Result<WorkerLaunchReceipt, WorkerEffectError>;

    fn stop(
        &mut self,
        guest: &ResourceRef,
        policy_generation: u64,
        teardown_generation: u64,
        session_digest: [u8; 32],
    ) -> Result<WorkerLaunchReceipt, WorkerEffectError>;
}

/// Closed default used by focused host-only composition tests. Production
/// composition installs [`AuthenticatedGuestFrontendEffects`] instead.
#[derive(Clone, Default)]
pub struct UnavailableGuestFrontendEffects;

impl GuestFrontendEffectPort for UnavailableGuestFrontendEffects {
    fn ensure(
        &mut self,
        _guest: &ResourceRef,
        _policy_generation: u64,
        _teardown_generation: u64,
        _session_digest: [u8; 32],
    ) -> Result<WorkerLaunchReceipt, WorkerEffectError> {
        Err(WorkerEffectError::WorkerUnavailable)
    }

    fn stop(
        &mut self,
        _guest: &ResourceRef,
        policy_generation: u64,
        teardown_generation: u64,
        session_digest: [u8; 32],
    ) -> Result<WorkerLaunchReceipt, WorkerEffectError> {
        Ok(WorkerLaunchReceipt::from_supervisor(
            DisplayProcessRole::GuestFrontend,
            WorkerState::Terminal { deleted: true },
            policy_generation,
            teardown_generation,
            session_digest,
        ))
    }
}

/// Production guest frontend effect owner. It resolves the guest's trusted
/// VM process DAG, authenticates over guest-control, and controls only the
/// closed `wayland-proxy.service` unit through the guest workload user.
#[derive(Clone)]
pub struct AuthenticatedGuestFrontendEffects {
    resolver: BundleResolver,
    broker_socket_path: PathBuf,
    caller_role: BrokerCallerRole,
    expected_state_root_uid: u32,
    expected_state_root_gid: u32,
}

impl AuthenticatedGuestFrontendEffects {
    pub fn new(
        resolver: BundleResolver,
        broker_socket_path: PathBuf,
        caller_role: BrokerCallerRole,
        expected_state_root_uid: u32,
        expected_state_root_gid: u32,
    ) -> Self {
        Self {
            resolver,
            broker_socket_path,
            caller_role,
            expected_state_root_uid,
            expected_state_root_gid,
        }
    }

    fn params(
        &self,
        guest: &ResourceRef,
    ) -> Result<crate::guest_control_bridge::ProbeParams, WorkerEffectError> {
        let guest_name = guest
            .to_canonical_string()
            .strip_prefix("Guest/")
            .map(str::to_owned)
            .ok_or(WorkerEffectError::WorkerUnavailable)?;
        let vm = if self.resolver.find_process_vm(&guest_name).is_some() {
            guest_name
        } else {
            let mut candidates = self
                .resolver
                .processes
                .vms
                .iter()
                .filter(|dag| {
                    dag.nodes
                        .iter()
                        .any(|node| node.role == ProcessRole::WaylandProxy)
                })
                .map(|dag| dag.vm.as_str());
            let Some(vm) = candidates.next() else {
                return Err(WorkerEffectError::WorkerUnavailable);
            };
            if candidates.next().is_some() {
                return Err(WorkerEffectError::WorkerUnavailable);
            }
            vm.to_owned()
        };
        let dag = self
            .resolver
            .find_process_vm(&vm)
            .ok_or(WorkerEffectError::WorkerUnavailable)?;
        let runner = dag
            .nodes
            .iter()
            .find(|node| node.role == ProcessRole::CloudHypervisorRunner)
            .ok_or(WorkerEffectError::WorkerUnavailable)?;
        let socket_path = runner
            .argv
            .windows(2)
            .find_map(|pair| {
                (pair[0] == "--vsock").then(|| {
                    pair[1]
                        .split(',')
                        .find_map(|field| field.strip_prefix("socket=").map(PathBuf::from))
                })
            })
            .flatten()
            .ok_or(WorkerEffectError::WorkerUnavailable)?;
        let state_root = socket_path
            .parent()
            .map(PathBuf::from)
            .ok_or(WorkerEffectError::WorkerUnavailable)?;
        Ok(crate::guest_control_bridge::ProbeParams {
            vm_id: vm,
            socket_path,
            state_root,
            expected_state_root_uid: self.expected_state_root_uid,
            expected_state_root_gid: self.expected_state_root_gid,
            expected_peer_uid: runner.profile.uid,
            expected_peer_gid: runner.profile.gid,
        })
    }
}

impl GuestFrontendEffectPort for AuthenticatedGuestFrontendEffects {
    fn ensure(
        &mut self,
        guest: &ResourceRef,
        policy_generation: u64,
        teardown_generation: u64,
        session_digest: [u8; 32],
    ) -> Result<WorkerLaunchReceipt, WorkerEffectError> {
        let params = self.params(guest)?;
        let probe = crate::guest_control_bridge::RealGuestControlProbe::with_caller_role(
            self.broker_socket_path.clone(),
            self.caller_role.clone(),
        );
        let observed = probe
            .wayland_service(
                &params,
                Duration::from_secs(10),
                crate::guest_control_bridge::GuestWaylandServiceAction::Observe,
            )
            .map_err(|_| WorkerEffectError::WorkerUnavailable)?;
        let evidence = if observed.active {
            observed
        } else {
            probe
                .wayland_service(
                    &params,
                    Duration::from_secs(10),
                    crate::guest_control_bridge::GuestWaylandServiceAction::Ensure,
                )
                .map_err(|_| WorkerEffectError::LaunchRejected)?
        };
        if !evidence.active {
            return Err(WorkerEffectError::LaunchRejected);
        }
        Ok(WorkerLaunchReceipt::from_supervisor(
            DisplayProcessRole::GuestFrontend,
            WorkerState::Ready {
                generation: evidence.state_generation.max(1),
            },
            policy_generation,
            teardown_generation,
            session_digest,
        ))
    }

    fn stop(
        &mut self,
        guest: &ResourceRef,
        policy_generation: u64,
        teardown_generation: u64,
        session_digest: [u8; 32],
    ) -> Result<WorkerLaunchReceipt, WorkerEffectError> {
        let params = self.params(guest)?;
        let probe = crate::guest_control_bridge::RealGuestControlProbe::with_caller_role(
            self.broker_socket_path.clone(),
            self.caller_role.clone(),
        );
        let evidence = probe
            .wayland_service(
                &params,
                Duration::from_secs(10),
                crate::guest_control_bridge::GuestWaylandServiceAction::Stop,
            )
            .map_err(|_| WorkerEffectError::CleanupIncomplete)?;
        if evidence.active {
            return Err(WorkerEffectError::CleanupIncomplete);
        }
        Ok(WorkerLaunchReceipt::from_supervisor(
            DisplayProcessRole::GuestFrontend,
            WorkerState::Terminal { deleted: true },
            policy_generation,
            teardown_generation,
            session_digest,
        ))
    }
}

impl InteractionDispatchError {
    const fn code(self) -> TtrpcCode {
        match self {
            Self::SessionUnavailable => TtrpcCode::UNAUTHENTICATED,
            Self::MalformedRequest | Self::ServiceMismatch | Self::InvalidPayload => {
                TtrpcCode::INVALID_ARGUMENT
            }
            Self::UnknownOperation => TtrpcCode::UNIMPLEMENTED,
            Self::RuntimeFailure => TtrpcCode::FAILED_PRECONDITION,
            Self::ResponseFailed => TtrpcCode::UNAVAILABLE,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ClipboardCaptureRequest {
    mime: String,
    #[serde(default)]
    bytes: Option<Vec<u8>>,
    #[serde(default)]
    source_entry_digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PickerCompletionRequest {
    entry_digest: String,
    mime_types: Vec<String>,
    selected_digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PickerMaterializeRequest {
    operation_id: String,
    entry_digest: String,
}

#[derive(Debug, Deserialize)]
struct NotificationDeliverRequest {
    request: NotificationRequest,
}

fn daemon_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn daemon_monotonic_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[derive(Debug, Deserialize)]
struct DisplayReconcileRequest {
    spec: WaylandSessionSpec,
}

/// Committed Core/resource-plane evidence used to compile display policy.
///
/// Requests carry only the desired session shape.  Policy identity,
/// generation, and dependency readiness come from this daemon-owned snapshot,
/// which is replaced atomically when the resource plane commits a new revision.
#[derive(Clone)]
pub struct CoreDisplayResourceEvidence {
    policy_ref: ResourceRef,
    policy_generation: u64,
    defaults: FilterInput,
    zone_policy: FilterInput,
    dependencies: DependencyState,
    committed_policy: PolicySnapshot,
    observer_user_ref: ResourceRef,
    resource_revision: ZoneRevision,
    resource_ready: bool,
}

impl CoreDisplayResourceEvidence {
    /// Bind a display policy to a committed resource snapshot.
    #[allow(clippy::too_many_arguments)]
    pub fn from_committed_policy(
        policy_ref: ResourceRef,
        committed_policy: PolicySnapshot,
        policy_generation: u64,
        defaults: FilterInput,
        zone_policy: FilterInput,
        dependencies: DependencyState,
        observer_user_ref: ResourceRef,
        resource_revision: ZoneRevision,
        resource_ready: bool,
    ) -> Result<Self, &'static str> {
        if policy_ref.resource_type().as_str() != "display-wayland.d2bus.org.WaylandPolicy"
            || policy_generation == 0
            || committed_policy.policy_revision == 0
            || committed_policy.active_configuration_revision.get() == 0
            || committed_policy
                .controller_generation
                .is_some_and(|generation| generation.get() == 0)
            || observer_user_ref.resource_type().as_str() != "User"
            || resource_revision.get() == 0
            || !resource_ready
        {
            return Err("display-resource-evidence-invalid");
        }
        Ok(Self {
            policy_ref,
            policy_generation,
            defaults,
            zone_policy,
            dependencies,
            committed_policy,
            observer_user_ref,
            resource_revision,
            resource_ready,
        })
    }
}

fn interaction_route_for_member(
    binding: &AuthenticatedSessionRouteBinding,
    member: &str,
) -> Result<RouteKey, InteractionDispatchError> {
    let service = ServiceName::parse(binding.service().as_str())
        .map_err(|_| InteractionDispatchError::ServiceMismatch)?;
    let member = RouteMember::method(member.to_owned())
        .map_err(|_| InteractionDispatchError::UnknownOperation)?;
    let target_ref = binding
        .provider_ref()
        .unwrap_or_else(|| binding.subject_ref())
        .clone();
    let target = if target_ref.resource_type().as_str() == "Provider" {
        RouteTarget::provider(target_ref)
    } else {
        RouteTarget::resource(target_ref)
    }
    .map_err(|_| InteractionDispatchError::ServiceMismatch)?;
    Ok(RouteKey::new(
        binding.zone().clone(),
        service,
        member,
        target,
        binding.schema().clone(),
        RouteGenerations::new(
            binding.provider_generation(),
            binding.controller_generation(),
            binding.reconnect_generation(),
        ),
    ))
}

fn encode_interaction_response(
    stream_id: u32,
    code: TtrpcCode,
    payload: Vec<u8>,
) -> Result<Vec<u8>, InteractionDispatchError> {
    let mut status = TtrpcStatus::new();
    status.set_code(code);
    status.set_message(
        match code {
            TtrpcCode::OK => "",
            TtrpcCode::UNIMPLEMENTED => "interaction-operation-unsupported",
            TtrpcCode::UNAUTHENTICATED => "interaction-session-unavailable",
            TtrpcCode::INVALID_ARGUMENT => "interaction-request-invalid",
            TtrpcCode::FAILED_PRECONDITION => "interaction-runtime-rejected",
            TtrpcCode::UNAVAILABLE => "interaction-response-unavailable",
            _ => "interaction-request-failed",
        }
        .to_owned(),
    );
    let response = TtrpcResponse {
        status: protobuf::MessageField::some(status),
        payload,
        ..TtrpcResponse::default()
    };
    let bytes = response
        .write_to_bytes()
        .map_err(|_| InteractionDispatchError::ResponseFailed)?;
    let length =
        u32::try_from(bytes.len()).map_err(|_| InteractionDispatchError::ResponseFailed)?;
    let mut frame = Vec::from(MessageHeader::new_response(stream_id, length));
    frame.extend_from_slice(&bytes);
    Ok(frame)
}

impl<S, G> core::fmt::Debug for InteractionComposition<S, G>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InteractionComposition")
            .field("session_count", &self.sessions.len())
            .field("display_ready", &self.display.is_some())
            .field("clipboard_ready", &self.clipboard.is_some())
            .field("notification_ready", &self.notification.is_some())
            .finish()
    }
}

impl<S> InteractionComposition<S, UnavailableGuestFrontendEffects>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
{
    /// Join one daemon-owned registrar to its supervisor effect owner.
    pub fn new(registrar: ZoneRegistrar, supervisor: S) -> Self {
        Self::new_with_notification_port(
            registrar,
            supervisor,
            UnavailableGuestFrontendEffects,
            Box::new(InteractionNotificationPort::default()),
        )
    }

    /// Join one daemon-owned registrar to its supervisor and presentation
    /// effect owner.
    pub fn new_with_notification_port(
        registrar: ZoneRegistrar,
        supervisor: S,
        guest_frontend: UnavailableGuestFrontendEffects,
        notification_port: Box<dyn DesktopNotificationPort + Send>,
    ) -> Self {
        Self {
            registrar,
            supervisor,
            guest_frontend,
            sessions: BTreeMap::new(),
            display: None,
            clipboard: None,
            notification: None,
            pending_picker_receipts: BTreeMap::new(),
            pending_guest_selection_events: BTreeMap::new(),
            notification_port: Arc::new(Mutex::new(notification_port)),
            display_resource_evidence: None,
            clipboard_configuration: None,
            notification_configuration: None,
        }
    }
}

impl<S, G> InteractionComposition<S, G>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    /// Join a daemon-owned registrar, host supervisor, and authenticated guest
    /// frontend effect owner.
    pub fn new_with_guest_frontend(
        registrar: ZoneRegistrar,
        supervisor: S,
        guest_frontend: G,
    ) -> Self {
        Self::new_with_guest_frontend_and_notification_port(
            registrar,
            supervisor,
            guest_frontend,
            Box::new(InteractionNotificationPort::default()),
        )
    }

    /// Join daemon-owned effects with an injected desktop presentation
    /// adapter.
    pub fn new_with_guest_frontend_and_notification_port(
        registrar: ZoneRegistrar,
        supervisor: S,
        guest_frontend: G,
        notification_port: Box<dyn DesktopNotificationPort + Send>,
    ) -> Self {
        Self {
            registrar,
            supervisor,
            guest_frontend,
            sessions: BTreeMap::new(),
            display: None,
            clipboard: None,
            notification: None,
            pending_picker_receipts: BTreeMap::new(),
            pending_guest_selection_events: BTreeMap::new(),
            notification_port: Arc::new(Mutex::new(notification_port)),
            display_resource_evidence: None,
            clipboard_configuration: None,
            notification_configuration: None,
        }
    }

    /// Borrow the registrar used for authenticated admission.
    pub const fn registrar(&self) -> &ZoneRegistrar {
        &self.registrar
    }

    /// Admit and register one ComponentSession, retaining only its route
    /// projection after the bus consumes the sealed authority.
    pub async fn admit_and_register<T>(
        &mut self,
        acceptor: SessionAcceptor<ComponentSessionAdmission>,
        engine: SessionEngine<T>,
        evidence: TransportEvidence,
        now_tick: u64,
    ) -> Result<&RegisteredInteractionSession, InteractionAdmissionError>
    where
        T: OwnedTransport + 'static,
    {
        let session = admit_and_register_with_route(
            &mut self.registrar,
            acceptor,
            engine,
            evidence,
            now_tick,
        )
        .await?;
        let service = session.route().service().as_str().to_owned();
        let session_key = session.session_key();
        if self.sessions.contains_key(&session_key) {
            let RegisteredInteractionSession { ingress, .. } = session;
            self.registrar
                .revoke(ingress)
                .await
                .map_err(|_| InteractionAdmissionError::Registration)?;
            return Err(InteractionAdmissionError::Registration);
        }
        if matches!(
            service.as_str(),
            d2b_provider_clipboard_wayland::MANAGEMENT_SERVICE
                | d2b_provider_clipboard_wayland::BRIDGE_SERVICE
                | d2b_provider_clipboard_wayland::PICKER_SERVICE
        ) && self.ensure_clipboard().is_err()
        {
            let RegisteredInteractionSession { ingress, .. } = session;
            let _ = self.registrar.revoke(ingress).await;
            return Err(InteractionAdmissionError::ServiceUnavailable);
        }
        self.sessions.insert(session_key.clone(), session);
        Ok(self
            .sessions
            .get(&session_key)
            .expect("session was just installed"))
    }

    /// Borrow the authenticated route retained after registration.
    pub fn route(&self) -> Option<&AuthenticatedSessionRouteBinding> {
        self.sessions
            .values()
            .next()
            .map(RegisteredInteractionSession::route)
    }

    /// Borrow the authenticated route for one exact service package.
    pub fn route_for_service(&self, service: &str) -> Option<&AuthenticatedSessionRouteBinding> {
        self.sessions
            .values()
            .find(|session| session.service().as_str() == service)
            .map(RegisteredInteractionSession::route)
    }

    /// Borrow every authenticated route for one service package.
    pub fn routes_for_service(&self, service: &str) -> Vec<AuthenticatedSessionRouteBinding> {
        self.sessions
            .values()
            .filter(|session| session.service().as_str() == service)
            .map(|session| session.route().clone())
            .collect()
    }

    fn route_for_session(&self, session_key: &str) -> Option<&AuthenticatedSessionRouteBinding> {
        self.sessions
            .get(session_key)
            .map(RegisteredInteractionSession::route)
    }

    /// Return the number of live authenticated interaction sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Whether one exact interaction service has an admitted session.
    pub fn has_service_session(&self, service: &str) -> bool {
        self.sessions
            .values()
            .any(|session| session.service().as_str() == service)
    }

    fn has_session(&self, session_key: &str) -> bool {
        self.sessions.contains_key(session_key)
    }

    /// Install the latest committed Core/resource-plane display evidence.
    pub fn bind_display_resource_evidence(&mut self, evidence: CoreDisplayResourceEvidence) {
        self.display_resource_evidence = Some(evidence);
    }

    /// Bind Core's sealed committed interaction Provider configuration.
    pub(crate) fn bind_interaction_provider_configuration(
        &mut self,
        configuration: &CommittedInteractionProviderConfiguration,
    ) -> Result<(), &'static str> {
        if !configuration.is_complete() {
            return Err("interaction-configuration-incomplete");
        }
        self.clipboard_configuration = configuration.clipboard().cloned();
        self.notification_configuration = configuration.notification().cloned();
        Ok(())
    }

    /// Receive and dispatch one request that was demultiplexed by the
    /// registrar-owned ComponentSession response task.
    ///
    /// The request's service/member are checked against the authenticated
    /// route before a local operation lease is minted. Runtime methods only
    /// receive route projections retained by this composition.
    pub async fn dispatch_component_request(
        &mut self,
        service: &str,
        frame: Vec<u8>,
    ) -> Result<(), String> {
        let session_key = self
            .sessions
            .iter()
            .find(|(_, session)| session.service().as_str() == service)
            .map(|(key, _)| key.clone())
            .ok_or("interaction-session-unavailable")?;
        self.dispatch_component_request_for_session(&session_key, frame, Vec::new())
            .await
    }

    /// Dispatch one authenticated request together with its separately
    /// demultiplexed attachment batch.
    pub async fn dispatch_component_request_with_attachments(
        &mut self,
        service: &str,
        frame: Vec<u8>,
        attachments: Vec<OwnedAttachment>,
    ) -> Result<(), String> {
        let session_key = self
            .sessions
            .iter()
            .find(|(_, session)| session.service().as_str() == service)
            .map(|(key, _)| key.clone())
            .ok_or("interaction-session-unavailable")?;
        self.dispatch_component_request_for_session(&session_key, frame, attachments)
            .await
    }

    async fn dispatch_component_request_for_session(
        &mut self,
        session_key: &str,
        frame: Vec<u8>,
        attachments: Vec<OwnedAttachment>,
    ) -> Result<(), String> {
        let service = self
            .route_for_session(session_key)
            .ok_or("interaction-session-unavailable")?
            .service()
            .as_str()
            .to_owned();
        let stream_id = ttrpc_stream_id(&frame).map_err(|_| "invalid-request-frame")?;
        let payload = frame
            .get(ttrpc::proto::MESSAGE_HEADER_LENGTH..)
            .ok_or("invalid-request-frame")?;
        let request = match TtrpcRequest::parse_from_bytes(payload) {
            Ok(request) => request,
            Err(_) => {
                self.send_component_response(
                    session_key,
                    encode_interaction_response(
                        stream_id,
                        InteractionDispatchError::MalformedRequest.code(),
                        Vec::new(),
                    )
                    .map_err(|_| "response-encode-failed")?,
                )
                .await?;
                return Ok(());
            }
        };
        if request.service != service {
            self.send_component_response(
                session_key,
                encode_interaction_response(
                    stream_id,
                    InteractionDispatchError::ServiceMismatch.code(),
                    Vec::new(),
                )
                .map_err(|_| "response-encode-failed")?,
            )
            .await?;
            return Ok(());
        }
        if operation_catalog_entry(
            &service,
            &request.method,
            d2b_session::OperationKind::Method,
        )
        .is_none()
        {
            self.send_component_response(
                session_key,
                encode_interaction_response(
                    stream_id,
                    InteractionDispatchError::UnknownOperation.code(),
                    Vec::new(),
                )
                .map_err(|_| "response-encode-failed")?,
            )
            .await?;
            return Ok(());
        };
        let route = {
            let registered = self
                .sessions
                .get(session_key)
                .ok_or("interaction-session-unavailable")?;
            interaction_route_for_member(registered.route(), &request.method)
                .map_err(|_| "interaction-route-invalid")?
        };
        let operation_id = OperationId::parse(
            format!(
                "interaction-{}-{stream_id}-{}",
                service.replace('.', "-"),
                request.method.replace('/', "-"),
            )
            .to_ascii_lowercase(),
        )
        .map_err(|_| "interaction-operation-invalid")?;
        let operation = OperationSpec::new(operation_id, 60_000)
            .map_err(|_| "interaction-operation-invalid")?;
        let ingress = self
            .sessions
            .get(session_key)
            .ok_or("interaction-session-unavailable")?
            .ingress();
        let lease = match ingress.begin_local_invoke(route, operation).await {
            Ok(lease) => lease,
            Err(_) => {
                self.send_component_response(
                    session_key,
                    encode_interaction_response(
                        stream_id,
                        InteractionDispatchError::SessionUnavailable.code(),
                        Vec::new(),
                    )
                    .map_err(|_| "response-encode-failed")?,
                )
                .await?;
                return Ok(());
            }
        };
        let (code, response_payload, finalize_after_response) = match self
            .dispatch_interaction_operation(
                session_key,
                &service,
                &request.method,
                &request.payload,
                attachments,
            ) {
            Ok((payload, finalize_after_response)) => {
                (TtrpcCode::OK, payload, finalize_after_response)
            }
            Err(error) => (error.code(), Vec::new(), false),
        };
        self.send_component_response(
            session_key,
            encode_interaction_response(stream_id, code, response_payload)
                .map_err(|_| "response-encode-failed")?,
        )
        .await?;
        lease.finish().map_err(|_| "interaction-operation-failed")?;
        if finalize_after_response {
            tokio::time::sleep(Duration::from_millis(1)).await;
            self.finalize_async(d2b_provider_display_wayland::GraceState::Expired)
                .await
                .map_err(|_| "interaction-finalization-failed")?;
        }
        Ok(())
    }

    async fn send_component_response(&self, service: &str, frame: Vec<u8>) -> Result<(), String> {
        self.sessions
            .get(service)
            .ok_or("interaction-session-unavailable")?
            .ingress()
            .send_component_response(frame)
            .await
            .map_err(|_| "interaction-response-failed".to_owned())
    }

    fn dispatch_interaction_operation(
        &mut self,
        session_key: &str,
        service: &str,
        method: &str,
        payload: &[u8],
        attachments: Vec<OwnedAttachment>,
    ) -> Result<(Vec<u8>, bool), InteractionDispatchError> {
        match (service, method) {
            (d2b_provider_display_wayland::SERVICE_PACKAGE, "DisplayService/Observe") => Ok((
                serde_json::to_vec(&serde_json::json!({
                    "runtime_installed": self.display.is_some(),
                    "ready": self
                        .display
                        .as_ref()
                        .is_some_and(|runtime| runtime.is_ready()),
                }))
                .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                false,
            )),
            (d2b_provider_display_wayland::SERVICE_PACKAGE, "DisplayService/Finalize") => {
                if !payload.is_empty() {
                    return Err(InteractionDispatchError::InvalidPayload);
                }
                Ok((
                    serde_json::to_vec(&serde_json::json!({"accepted": true}))
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    true,
                ))
            }
            (d2b_provider_display_wayland::SERVICE_PACKAGE, "DisplayService/Reconcile") => {
                let request: DisplayReconcileRequest = serde_json::from_slice(payload)
                    .map_err(|_| InteractionDispatchError::InvalidPayload)?;
                let result = self
                    .reconcile_display_request(request)
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                Ok((
                    serde_json::to_vec(&serde_json::json!({
                        "phase": format!("{:?}", result.status.phase),
                        "worker_actions": result.worker_actions.len(),
                    }))
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }
            (
                d2b_provider_clipboard_wayland::BRIDGE_SERVICE,
                "ClipboardBridgeService/CaptureGuest",
            ) => {
                let request: ClipboardCaptureRequest = serde_json::from_slice(payload)
                    .map_err(|_| InteractionDispatchError::InvalidPayload)?;
                let bridge_route = self
                    .route_for_session(session_key)
                    .filter(|route| {
                        route.service().as_str() == d2b_provider_clipboard_wayland::BRIDGE_SERVICE
                    })
                    .cloned()
                    .ok_or(InteractionDispatchError::SessionUnavailable)?;
                let bytes = self
                    .clipboard_payload(
                        request.bytes,
                        attachments,
                        bridge_route.clone(),
                        AttachmentClass::GuestTransfer,
                    )
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                let token = self
                    .capture_guest_clipboard_route(
                        bridge_route.clone(),
                        &request.mime,
                        &bytes,
                        daemon_now_secs(),
                    )
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                if let Some(clipboard) = self.clipboard.as_mut() {
                    let event = clipboard
                        .guest_selection_event_route(bridge_route, &token, daemon_now_secs())
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                    if self.pending_guest_selection_events.len() >= 128 {
                        self.pending_guest_selection_events.pop_first();
                    }
                    self.pending_guest_selection_events
                        .insert(token.clone(), event);
                }
                if let Some(clipboard) = self.clipboard.as_mut() {
                    clipboard
                        .flush_audit(16)
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                }
                Ok((
                    serde_json::to_vec(&serde_json::json!({"entry_digest": token}))
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }
            (
                d2b_provider_display_wayland::SERVICE_PACKAGE,
                "ClipboardBridgeService/CaptureHost",
            ) => {
                let request: ClipboardCaptureRequest = serde_json::from_slice(payload)
                    .map_err(|_| InteractionDispatchError::InvalidPayload)?;
                let display_route = self
                    .route_for_session(session_key)
                    .filter(|route| {
                        route.service().as_str() == d2b_provider_display_wayland::SERVICE_PACKAGE
                    })
                    .cloned()
                    .ok_or(InteractionDispatchError::SessionUnavailable)?;
                let source_event = request
                    .source_entry_digest
                    .as_deref()
                    .and_then(|digest| self.pending_guest_selection_events.remove(digest));
                let bytes = self
                    .clipboard_payload(
                        request.bytes,
                        attachments,
                        display_route.clone(),
                        AttachmentClass::HostSelectionWrite,
                    )
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                let token = self
                    .capture_host_clipboard_route(
                        display_route,
                        &request.mime,
                        &bytes,
                        source_event,
                        daemon_now_secs(),
                    )
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                if let Some(clipboard) = self.clipboard.as_mut() {
                    clipboard
                        .flush_audit(16)
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                }
                Ok((
                    serde_json::to_vec(&serde_json::json!({"entry_digest": token}))
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }

            (d2b_provider_clipboard_wayland::MANAGEMENT_SERVICE, "ClipboardService/Drain")
            | (d2b_provider_clipboard_wayland::BRIDGE_SERVICE, "ClipboardBridgeService/Drain") => {
                if !payload.is_empty() {
                    return Err(InteractionDispatchError::InvalidPayload);
                }
                let route = self
                    .route_for_session(session_key)
                    .cloned()
                    .ok_or(InteractionDispatchError::SessionUnavailable)?;
                self.ensure_clipboard()
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?
                    .admit_route(route)
                    .map_err(|_| InteractionDispatchError::SessionUnavailable)?;
                self.clipboard
                    .as_mut()
                    .expect("clipboard runtime was just admitted")
                    .drain()
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                Ok((
                    serde_json::to_vec(&serde_json::json!({"drained": true}))
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }
            (d2b_provider_clipboard_wayland::PICKER_SERVICE, "ClipboardPickerService/Complete") => {
                let request: PickerCompletionRequest = serde_json::from_slice(payload)
                    .map_err(|_| InteractionDispatchError::InvalidPayload)?;
                let source_route = self
                    .route_for_session(session_key)
                    .filter(|route| {
                        route.service().as_str() == d2b_provider_clipboard_wayland::PICKER_SERVICE
                    })
                    .cloned()
                    .ok_or(InteractionDispatchError::SessionUnavailable)?;
                let destination_route = self
                    .route_for_service(d2b_provider_clipboard_wayland::BRIDGE_SERVICE)
                    .cloned()
                    .ok_or(InteractionDispatchError::SessionUnavailable)?;
                let source = self
                    .ensure_clipboard()
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?
                    .admit_route(source_route)
                    .map_err(|_| InteractionDispatchError::SessionUnavailable)?;
                let destination = self
                    .clipboard
                    .as_ref()
                    .expect("clipboard runtime was just admitted")
                    .admit_route(destination_route)
                    .map_err(|_| InteractionDispatchError::SessionUnavailable)?;
                let picker_request = d2b_provider_clipboard_wayland::PickerRequest::from_sessions(
                    &source,
                    &destination,
                    request.mime_types,
                )
                .map_err(|_| InteractionDispatchError::InvalidPayload)?;
                let result = request.selected_digest.clone().map_or(
                    d2b_provider_clipboard_wayland::PickerResult::Cancelled,
                    d2b_provider_clipboard_wayland::PickerResult::Selected,
                );
                let receipt = self
                    .clipboard
                    .as_mut()
                    .expect("clipboard runtime was just admitted")
                    .complete_picker(
                        &source,
                        &destination,
                        &picker_request,
                        result,
                        request.entry_digest,
                        daemon_now_secs(),
                    )
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                let operation_id = receipt.operation_id().to_owned();
                self.pending_picker_receipts
                    .insert(operation_id.clone(), receipt);
                Ok((
                    serde_json::to_vec(&serde_json::json!({
                        "completed": true,
                        "operation_id": operation_id,
                    }))
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }
            (
                d2b_provider_clipboard_wayland::PICKER_SERVICE,
                "ClipboardPickerService/Materialize",
            ) => {
                let request: PickerMaterializeRequest = serde_json::from_slice(payload)
                    .map_err(|_| InteractionDispatchError::InvalidPayload)?;
                let receipt = self
                    .pending_picker_receipts
                    .remove(&request.operation_id)
                    .ok_or(InteractionDispatchError::SessionUnavailable)?;
                let source_route = self
                    .route_for_session(session_key)
                    .filter(|route| {
                        route.service().as_str() == d2b_provider_clipboard_wayland::PICKER_SERVICE
                    })
                    .cloned()
                    .ok_or(InteractionDispatchError::SessionUnavailable)?;
                let destination_route = self
                    .route_for_service(d2b_provider_clipboard_wayland::BRIDGE_SERVICE)
                    .cloned()
                    .ok_or(InteractionDispatchError::SessionUnavailable)?;
                let source = self
                    .ensure_clipboard()
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?
                    .admit_route(source_route)
                    .map_err(|_| InteractionDispatchError::SessionUnavailable)?;
                let destination = self
                    .clipboard
                    .as_ref()
                    .expect("clipboard runtime was just admitted")
                    .admit_route(destination_route)
                    .map_err(|_| InteractionDispatchError::SessionUnavailable)?;
                let paste_route =
                    d2b_provider_clipboard_wayland::AuthenticatedPasteRoute::from_sessions(
                        &source,
                        &destination,
                    )
                    .map_err(|_| InteractionDispatchError::SessionUnavailable)?;
                let bytes = self
                    .clipboard
                    .as_mut()
                    .expect("clipboard runtime was just admitted")
                    .materialize_after_picker(
                        &paste_route,
                        receipt,
                        &request.entry_digest,
                        daemon_now_secs(),
                    )
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                Ok((
                    serde_json::to_vec(&serde_json::json!({
                        "materialized": true,
                        "entry_digest": request.entry_digest,
                        "bytes": bytes,
                    }))
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }
            (d2b_provider_notification_desktop::SERVICE_PACKAGE, "NotificationService/Drain") => {
                if !payload.is_empty() {
                    return Err(InteractionDispatchError::InvalidPayload);
                }
                if self.route_for_session(session_key).is_none() {
                    return Err(InteractionDispatchError::SessionUnavailable);
                }
                self.ensure_notification()
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?
                    .drain()
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                Ok((
                    serde_json::to_vec(&serde_json::json!({"drained": true}))
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }
            (d2b_provider_notification_desktop::SERVICE_PACKAGE, "NotificationService/Deliver") => {
                let request: NotificationDeliverRequest = serde_json::from_slice(payload)
                    .map_err(|_| InteractionDispatchError::InvalidPayload)?;
                let source_route = self
                    .route_for_session(session_key)
                    .filter(|route| {
                        route.service().as_str()
                            == d2b_provider_notification_desktop::SERVICE_PACKAGE
                    })
                    .cloned()
                    .ok_or(InteractionDispatchError::SessionUnavailable)?;
                let observer_route = self
                    .route_for_service(d2b_provider_display_wayland::SERVICE_PACKAGE)
                    .cloned()
                    .ok_or(InteractionDispatchError::SessionUnavailable)?;
                if !self
                    .display
                    .as_ref()
                    .is_some_and(|display| display.is_ready())
                {
                    return Err(InteractionDispatchError::RuntimeFailure);
                }
                let source_evidence =
                    d2b_provider_notification_desktop::SessionEvidence::from_daemon_route(
                        source_route.clone(),
                    )
                    .map_err(|_| InteractionDispatchError::SessionUnavailable)?;
                let observer_user_ref = self
                    .display_resource_evidence
                    .as_ref()
                    .ok_or(InteractionDispatchError::SessionUnavailable)?
                    .observer_user_ref
                    .clone();
                let observer_evidence = d2b_provider_notification_desktop::SessionEvidence::
                    from_display_dependency_route(observer_route, observer_user_ref)
                    .map_err(|_| InteractionDispatchError::SessionUnavailable)?;
                self.ensure_notification()
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                let mut notification_port = self
                    .notification_port
                    .lock()
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                let result = self
                    .notification
                    .as_mut()
                    .expect("notification runtime was just installed")
                    .deliver_evidence(
                        &mut **notification_port,
                        &source_evidence,
                        &observer_evidence,
                        request.request,
                        daemon_now_secs(),
                    )
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                let response = match result {
                    d2b_provider_notification_desktop::NotificationResult::Accepted {
                        notification_id,
                        action_nonces,
                    } => serde_json::json!({
                        "accepted": true,
                        "notification_id": notification_id,
                        "action_count": action_nonces.len(),
                    }),
                    d2b_provider_notification_desktop::NotificationResult::CapacityExceeded => {
                        serde_json::json!({"accepted": false, "capacity_exceeded": true})
                    }
                    d2b_provider_notification_desktop::NotificationResult::SinkUnavailable => {
                        serde_json::json!({"accepted": false, "sink_unavailable": true})
                    }
                    d2b_provider_notification_desktop::NotificationResult::Rejected => {
                        serde_json::json!({"accepted": false, "rejected": true})
                    }
                };
                Ok((
                    serde_json::to_vec(&response)
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }
            (d2b_provider_clipboard_wayland::MANAGEMENT_SERVICE, "ClipboardService/Reconcile") => {
                if !payload.is_empty() {
                    return Err(InteractionDispatchError::InvalidPayload);
                }
                self.reconcile_dependents()
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                Ok((
                    serde_json::to_vec(&serde_json::json!({"reconciled": true}))
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }
            (
                d2b_provider_notification_desktop::SERVICE_PACKAGE,
                "NotificationService/Reconcile",
            ) => {
                if !payload.is_empty() {
                    return Err(InteractionDispatchError::InvalidPayload);
                }
                self.reconcile_dependents()
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                Ok((
                    serde_json::to_vec(&serde_json::json!({"reconciled": true}))
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }
            _ => Err(InteractionDispatchError::UnknownOperation),
        }
    }

    fn clipboard_payload(
        &mut self,
        inline: Option<Vec<u8>>,
        attachments: Vec<OwnedAttachment>,
        route: AuthenticatedSessionRouteBinding,
        attachment_class: AttachmentClass,
    ) -> Result<Vec<u8>, ClipboardServiceError> {
        if attachments.is_empty() {
            return inline.ok_or(ClipboardServiceError::AttachmentRejected);
        }
        if inline.is_some() {
            return Err(ClipboardServiceError::AttachmentRejected);
        }
        for attachment in &attachments {
            let descriptor = attachment
                .descriptor()
                .ok_or(ClipboardServiceError::AttachmentRejected)?;
            if descriptor.service != ServicePackage::ClipboardBridgeV3
                || descriptor.kind != AttachmentKind::FileDescriptor
                || descriptor.purpose != AttachmentPurpose::ClipboardTransfer
            {
                return Err(ClipboardServiceError::AttachmentRejected);
            }
        }
        let packet = VerifiedPacket::from_bound_attachments(attachments)
            .map_err(|_| ClipboardServiceError::AttachmentRejected)?;
        let clipboard = self.ensure_clipboard()?;
        let session = clipboard
            .admit_route(route)
            .map_err(|_| ClipboardServiceError::SessionUnauthenticated)?;
        let verified =
            clipboard
                .host()
                .accept_verified_packet(&session, packet, attachment_class)?;
        let payloads = verified
            .read_all()
            .map_err(|_| ClipboardServiceError::AttachmentRejected)?;
        if payloads.len() != 1 {
            return Err(ClipboardServiceError::AttachmentRejected);
        }
        payloads
            .into_iter()
            .next()
            .ok_or(ClipboardServiceError::AttachmentRejected)
    }

    /// Project the retained authenticated route into the clipboard service
    /// identity without reconstructing ComponentSession authority.
    pub fn clipboard_session(
        &self,
    ) -> Result<d2b_provider_clipboard_wayland::AuthenticatedClipboardSession, ClipboardServiceError>
    {
        let route = self
            .route_for_service(d2b_provider_clipboard_wayland::BRIDGE_SERVICE)
            .ok_or(ClipboardServiceError::SessionUnauthenticated)?;
        d2b_provider_clipboard_wayland::AuthenticatedClipboardSession::from_authenticated_route(
            route.clone(),
        )
    }

    /// Project the retained authenticated route into notification evidence
    /// for source reconciliation and bounded dispatch.
    pub fn notification_session(
        &self,
    ) -> Result<
        d2b_provider_notification_desktop::SessionEvidence,
        d2b_provider_notification_desktop::AdmissionError,
    > {
        let route = self
            .route_for_service(d2b_provider_notification_desktop::SERVICE_PACKAGE)
            .ok_or(d2b_provider_notification_desktop::AdmissionError::SessionUnauthenticated)?;
        d2b_provider_notification_desktop::SessionEvidence::from_authenticated_route(route.clone())
    }

    fn ensure_clipboard(
        &mut self,
    ) -> Result<
        &mut d2b_provider_clipboard_wayland::ClipboardRuntime<InteractionDrainEffects>,
        ClipboardServiceError,
    > {
        if self.clipboard.is_none() {
            #[cfg(not(test))]
            let configuration = self
                .clipboard_configuration
                .as_ref()
                .ok_or(ClipboardServiceError::SessionUnauthenticated)?;
            #[cfg(test)]
            let configuration = self.clipboard_configuration.as_ref();
            #[cfg(not(test))]
            let policy = configuration.policy();
            #[cfg(test)]
            let policy = configuration.map_or_else(
                d2b_provider_clipboard_wayland::Policy::default,
                CommittedClipboardProviderConfiguration::policy,
            );
            #[cfg(not(test))]
            let audit_capacity = configuration.audit_capacity();
            #[cfg(test)]
            let audit_capacity =
                configuration.map_or(128, |configuration| configuration.audit_capacity());
            self.clipboard = Some(
                d2b_provider_clipboard_wayland::ClipboardRuntime::new(
                    policy,
                    audit_capacity,
                    None,
                    InteractionDrainEffects::default(),
                )
                .map_err(|error| match error {
                    d2b_provider_clipboard_wayland::ClipboardRuntimeError::Service(error) => error,
                    _ => ClipboardServiceError::SessionUnauthenticated,
                })?,
            );
        }
        Ok(self
            .clipboard
            .as_mut()
            .expect("clipboard runtime was installed"))
    }

    fn ensure_notification(
        &mut self,
    ) -> Result<
        &mut d2b_provider_notification_desktop::NotificationRuntime<InteractionDrainEffects>,
        &'static str,
    > {
        if self.notification.is_none() {
            let source_route = self
                .route_for_service(d2b_provider_notification_desktop::SERVICE_PACKAGE)
                .cloned()
                .ok_or("notification-source-session-unavailable")?;
            self.ensure_notification_for_source(&source_route)?;
            self.reconcile_dependents()
                .map_err(|_| "notification-display-dependency-unavailable")?;
        }
        Ok(self
            .notification
            .as_mut()
            .expect("notification runtime was installed"))
    }

    fn ensure_notification_for_source(
        &mut self,
        _source_route: &AuthenticatedSessionRouteBinding,
    ) -> Result<
        &mut d2b_provider_notification_desktop::NotificationRuntime<InteractionDrainEffects>,
        &'static str,
    > {
        if self.notification.is_none() {
            #[cfg(not(test))]
            let config = self
                .notification_configuration
                .as_ref()
                .ok_or("notification-configuration-unavailable")?
                .config();
            #[cfg(test)]
            let config = match self.notification_configuration.as_ref() {
                Some(configuration) => configuration.config(),
                None => {
                    let source = d2b_provider_notification_desktop::GuestSourceConfig::new(
                        _source_route.subject_ref().clone(),
                        _source_route.zone().clone(),
                        Category::ALL,
                    )?;
                    let display_route = self
                        .route_for_service(d2b_provider_display_wayland::SERVICE_PACKAGE)
                        .ok_or("notification-display-session-unavailable")?;
                    let host_execution_ref = display_route
                        .context()
                        .execution_ref()
                        .cloned()
                        .ok_or("notification-host-binding-missing")?;
                    let observer_user_ref = self
                        .display_resource_evidence
                        .as_ref()
                        .ok_or("notification-display-evidence-unavailable")?
                        .observer_user_ref
                        .clone();
                    d2b_provider_notification_desktop::NotificationProviderConfig::new(vec![
                        source,
                    ])?
                    .with_host_binding(host_execution_ref, observer_user_ref)?
                    .with_display_wayland_ref(Some(
                        ResourceRef::parse("Provider/display-wayland")
                            .map_err(|_| "notification-display-provider-invalid")?,
                    ))?
                }
            };
            self.notification = Some(
                d2b_provider_notification_desktop::NotificationRuntime::new(
                    config,
                    InteractionDrainEffects::new(Arc::clone(&self.notification_port)),
                )
                .map_err(|_| "notification-runtime-unavailable")?,
            );
        }
        Ok(self
            .notification
            .as_mut()
            .expect("notification runtime was installed"))
    }

    /// Reconcile the dependent clipboard and notification runtimes after the
    /// display route has supplied a current authenticated dependency.
    pub fn reconcile_dependents(&mut self) -> Result<(), InteractionDependencyError> {
        if !self
            .display
            .as_ref()
            .is_some_and(|display| display.is_ready())
        {
            if let Some(clipboard) = self.clipboard.as_mut() {
                clipboard
                    .reconcile_display(None)
                    .map_err(InteractionDependencyError::Clipboard)?;
            }
            if let Some(notification) = self.notification.as_mut() {
                notification
                    .reconcile_daemon_routes(None, &[])
                    .map_err(InteractionDependencyError::Notification)?;
            }
            return Err(InteractionDependencyError::DisplayUnavailable);
        }
        let route = self
            .route_for_service(d2b_provider_display_wayland::SERVICE_PACKAGE)
            .ok_or(InteractionDependencyError::SessionUnauthenticated)?
            .clone();
        let observer_user_ref = self
            .display_resource_evidence
            .as_ref()
            .ok_or(InteractionDependencyError::DisplayUnavailable)?
            .observer_user_ref
            .clone();
        let clipboard_dependency =
            d2b_provider_clipboard_wayland::DisplayDependencyEvidence::from_committed_display_route(
                route.clone(),
                observer_user_ref,
            )
            .map_err(|_| InteractionDependencyError::DisplayUnavailable)?;
        if self
            .clipboard_configuration
            .as_ref()
            .is_some_and(|configuration| !configuration.matches_display(&clipboard_dependency))
        {
            return Err(InteractionDependencyError::DisplayUnavailable);
        }
        if let Some(clipboard) = self.clipboard.as_mut() {
            clipboard
                .reconcile_display(Some(clipboard_dependency))
                .map_err(InteractionDependencyError::Clipboard)?;
        }
        let source_routes =
            self.routes_for_service(d2b_provider_notification_desktop::SERVICE_PACKAGE);
        if let Some(notification) = self.notification.as_mut() {
            notification
                .reconcile_daemon_routes(Some(route), &source_routes)
                .map_err(InteractionDependencyError::Notification)?;
        }
        Ok(())
    }

    /// Dispatch a bounded Guest clipboard capture through the authenticated
    /// route retained by the daemon.
    pub fn capture_guest_clipboard(
        &mut self,
        mime: &str,
        bytes: &[u8],
        now_secs: u64,
    ) -> Result<String, ClipboardServiceError> {
        let route = self
            .route_for_service(d2b_provider_clipboard_wayland::BRIDGE_SERVICE)
            .ok_or(ClipboardServiceError::SessionUnauthenticated)?
            .clone();
        self.capture_guest_clipboard_route(route, mime, bytes, now_secs)
    }

    /// Dispatch a bounded Guest clipboard capture through one exact route.
    pub fn capture_guest_clipboard_route(
        &mut self,
        route: AuthenticatedSessionRouteBinding,
        mime: &str,
        bytes: &[u8],
        now_secs: u64,
    ) -> Result<String, ClipboardServiceError> {
        if self
            .clipboard_configuration
            .as_ref()
            .is_some_and(|configuration| !configuration.allows_guest_source(route.subject_ref()))
        {
            return Err(ClipboardServiceError::SessionUnauthenticated);
        }
        self.ensure_clipboard()?
            .capture_guest_route(route, mime, bytes, now_secs)
            .map_err(|error| match error {
                d2b_provider_clipboard_wayland::ClipboardRuntimeError::Service(error) => error,
                _ => ClipboardServiceError::SessionUnauthenticated,
            })
    }

    /// Dispatch a bounded host clipboard capture through the authenticated
    /// route retained by the daemon.
    pub fn capture_host_clipboard(
        &mut self,
        mime: &str,
        bytes: &[u8],
        source_event: Option<d2b_provider_clipboard_wayland::GuestSelectionEvent>,
        now_secs: u64,
    ) -> Result<String, ClipboardServiceError> {
        let route = self
            .route_for_service(d2b_provider_display_wayland::SERVICE_PACKAGE)
            .ok_or(ClipboardServiceError::SessionUnauthenticated)?
            .clone();
        self.capture_host_clipboard_route(route, mime, bytes, source_event, now_secs)
    }

    /// Dispatch a bounded host clipboard capture through an authenticated
    /// desktop User route.
    pub fn capture_host_clipboard_route(
        &mut self,
        route: AuthenticatedSessionRouteBinding,
        mime: &str,
        bytes: &[u8],
        source_event: Option<d2b_provider_clipboard_wayland::GuestSelectionEvent>,
        now_secs: u64,
    ) -> Result<String, ClipboardServiceError> {
        self.ensure_clipboard()?
            .capture_host_route(route, mime, bytes, source_event, now_secs)
            .map_err(|error| match error {
                d2b_provider_clipboard_wayland::ClipboardRuntimeError::Service(error) => error,
                _ => ClipboardServiceError::SessionUnauthenticated,
            })
    }

    /// Reconcile the display runtime through the registered route and the
    /// daemon-owned supervisor effects.
    pub fn reconcile_display(
        &mut self,
        controller: d2b_provider_display_wayland::DisplayController,
        spec: &WaylandSessionSpec,
        dependencies: d2b_provider_display_wayland::DependencyState,
        supervision: d2b_provider_display_wayland::WorkerRestartEvidence,
        policy: &WaylandPolicySnapshot,
    ) -> Result<d2b_provider_display_wayland::ReconcileResult, DisplayRuntimeError> {
        let route = self
            .route_for_service(d2b_provider_display_wayland::SERVICE_PACKAGE)
            .ok_or(DisplayRuntimeError::SessionUnauthenticated)?
            .clone();
        let supervisor = self.supervisor.clone();
        let guest_frontend = self.guest_frontend.clone();
        let runtime = self.display.get_or_insert_with(|| {
            DisplayRuntime::new(
                controller,
                DisplaySupervisorEffects::new_with_guest_frontend(supervisor, guest_frontend),
            )
        });
        let result =
            runtime.reconcile_registered(&route, spec, dependencies, supervision, policy)?;
        if result.status.phase == d2b_provider_display_wayland::Phase::Ready {
            self.reconcile_dependents()
                .map_err(|_| DisplayRuntimeError::ObservationUnavailable)?;
        }
        Ok(result)
    }

    fn reconcile_display_request(
        &mut self,
        request: DisplayReconcileRequest,
    ) -> Result<d2b_provider_display_wayland::ReconcileResult, DisplayRuntimeError> {
        let route = self
            .route_for_service(d2b_provider_display_wayland::SERVICE_PACKAGE)
            .ok_or(DisplayRuntimeError::SessionUnauthenticated)?
            .clone();
        if request.spec.guest_ref() != route.subject_ref()
            || request.spec.host_ref().resource_type().as_str() != "Host"
        {
            return Err(DisplayRuntimeError::SessionMismatch);
        }
        let policy_generation = route
            .provider_generation()
            .map(|generation| generation.get())
            .ok_or(DisplayRuntimeError::InvalidPolicy)?;
        let evidence = self
            .display_resource_evidence
            .as_ref()
            .ok_or(DisplayRuntimeError::InvalidPolicy)?;
        if request.spec.policy_ref() != &evidence.policy_ref {
            return Err(DisplayRuntimeError::InvalidPolicy);
        }
        if !evidence.resource_ready
            || evidence.resource_revision.get() == 0
            || policy_generation == 0
        {
            return Err(DisplayRuntimeError::InvalidPolicy);
        }
        if route
            .controller_generation()
            .map(|generation| generation.get())
            .is_some_and(|generation| {
                evidence
                    .committed_policy
                    .controller_generation
                    .is_some_and(|committed| committed.get() != generation)
            })
        {
            return Err(DisplayRuntimeError::InvalidPolicy);
        }
        let policy = WaylandPolicySnapshot::from_authenticated_route(
            &route,
            evidence.policy_ref.clone(),
            evidence.policy_generation,
            evidence.defaults.clone(),
            evidence.zone_policy.clone(),
        )
        .map_err(|_| DisplayRuntimeError::InvalidPolicy)?;
        let supervision = if let Some(display) = self.display.as_mut() {
            display.refresh_supervision();
            display.supervision()
        } else {
            WorkerRestartEvidence::from_supervisor(daemon_monotonic_ms(), None, None, 1)
        };
        self.reconcile_display(
            DisplayController::new(8),
            &request.spec,
            evidence.dependencies.clone(),
            supervision,
            &policy,
        )
    }

    /// Finalize display first, then drain clipboard/notification effects, and
    /// only then release the bus ingress.
    pub fn finalize(
        &mut self,
        grace: d2b_provider_display_wayland::GraceState,
    ) -> Result<d2b_provider_display_wayland::FinalizationReport, InteractionFinalizeError> {
        let (report, mut failure) = match self.display.as_mut() {
            Some(display) => match display.finalize(grace) {
                Ok(report) => (report, None),
                Err(error) => (
                    d2b_provider_display_wayland::FinalizationReport::empty(),
                    Some(InteractionFinalizeError::Display(error)),
                ),
            },
            None => (
                d2b_provider_display_wayland::FinalizationReport::empty(),
                None,
            ),
        };
        if let Some(clipboard) = self.clipboard.as_mut()
            && let Err(error) = clipboard.finalize(std::iter::empty())
        {
            failure.get_or_insert(InteractionFinalizeError::Clipboard(error));
        }
        if let Some(notification) = self.notification.as_mut()
            && let Err(error) = notification.finalize()
        {
            failure.get_or_insert(InteractionFinalizeError::Notification(error));
        }
        if failure.is_none() {
            self.pending_picker_receipts.clear();
            self.pending_guest_selection_events.clear();
            self.sessions.clear();
        }
        failure.map_or(Ok(report), Err)
    }

    /// Finalize all runtimes and explicitly revoke every registered ingress.
    ///
    /// The synchronous [`Self::finalize`] method remains available for
    /// bounded unit callers.  Production shutdown uses this async form so
    /// bus cancellation and response tasks are joined before authority is
    /// released.
    pub async fn finalize_async(
        &mut self,
        grace: d2b_provider_display_wayland::GraceState,
    ) -> Result<d2b_provider_display_wayland::FinalizationReport, InteractionFinalizeError> {
        let (report, mut failure) = match self.display.as_mut() {
            Some(display) => match display.finalize(grace) {
                Ok(report) => (report, None),
                Err(error) => (
                    d2b_provider_display_wayland::FinalizationReport::empty(),
                    Some(InteractionFinalizeError::Display(error)),
                ),
            },
            None => (
                d2b_provider_display_wayland::FinalizationReport::empty(),
                None,
            ),
        };
        if let Some(clipboard) = self.clipboard.as_mut()
            && let Err(error) = clipboard.finalize(std::iter::empty())
        {
            failure.get_or_insert(InteractionFinalizeError::Clipboard(error));
        }
        if let Some(notification) = self.notification.as_mut()
            && let Err(error) = notification.finalize()
        {
            failure.get_or_insert(InteractionFinalizeError::Notification(error));
        }
        if failure.is_none() {
            let services = self.sessions.keys().cloned().collect::<Vec<_>>();
            for service in services {
                let revoked = if let Some(session) = self.sessions.get_mut(&service) {
                    self.registrar
                        .revoke_in_place(&mut session.ingress)
                        .await
                        .is_ok()
                } else {
                    true
                };
                if !revoked {
                    failure.get_or_insert(InteractionFinalizeError::Registration);
                    break;
                }
                self.sessions.remove(&service);
            }
        }
        if failure.is_none() {
            self.pending_picker_receipts.clear();
            self.pending_guest_selection_events.clear();
        }
        failure.map_or(Ok(report), Err)
    }

    async fn remove_session(&mut self, session_key: &str) -> Result<(), String> {
        let Some(session) = self.sessions.get(session_key) else {
            return Ok(());
        };
        let service = session.service().as_str().to_owned();
        let last_for_service = self
            .sessions
            .values()
            .filter(|candidate| candidate.service().as_str() == service)
            .count()
            == 1;
        let session = self
            .sessions
            .get_mut(session_key)
            .ok_or_else(|| "interaction-session-unavailable".to_owned())?;
        self.registrar
            .revoke_in_place(&mut session.ingress)
            .await
            .map_err(|_| "interaction-session-revocation-failed".to_owned())?;
        self.sessions.remove(session_key);
        let service_cleanup = match service.as_str() {
            d2b_provider_display_wayland::SERVICE_PACKAGE if last_for_service => {
                if let Some(display) = self.display.as_mut() {
                    display
                        .finalize(d2b_provider_display_wayland::GraceState::Expired)
                        .map_err(|_| "display-finalization-failed".to_owned())?;
                } else {
                    self.display_resource_evidence = None;
                }
                self.display_resource_evidence = None;
                self.clipboard.as_mut().map_or(Ok(()), |clipboard| {
                    clipboard
                        .reconcile_display(None)
                        .map_err(|_| "clipboard-disconnect-reconcile-failed".to_owned())
                })?;
                self.notification.as_mut().map_or(Ok(()), |notification| {
                    notification
                        .reconcile_daemon_routes(None, &[])
                        .map(|_| ())
                        .map_err(|_| "notification-disconnect-reconcile-failed".to_owned())
                })
            }
            d2b_provider_display_wayland::SERVICE_PACKAGE => self
                .reconcile_dependents()
                .map_err(|_| "display-disconnect-reconcile-failed".to_owned()),
            d2b_provider_clipboard_wayland::MANAGEMENT_SERVICE
            | d2b_provider_clipboard_wayland::BRIDGE_SERVICE
            | d2b_provider_clipboard_wayland::PICKER_SERVICE
                if last_for_service =>
            {
                self.clipboard.as_mut().map_or(Ok(()), |clipboard| {
                    clipboard
                        .finalize(std::iter::empty())
                        .map(|_| ())
                        .map_err(|_| "clipboard-finalization-failed".to_owned())
                })
            }
            d2b_provider_clipboard_wayland::MANAGEMENT_SERVICE
            | d2b_provider_clipboard_wayland::BRIDGE_SERVICE
            | d2b_provider_clipboard_wayland::PICKER_SERVICE => self
                .reconcile_dependents()
                .map_err(|_| "clipboard-disconnect-reconcile-failed".to_owned()),
            d2b_provider_notification_desktop::SERVICE_PACKAGE if last_for_service => {
                self.notification.as_mut().map_or(Ok(()), |notification| {
                    notification
                        .finalize()
                        .map(|_| ())
                        .map_err(|_| "notification-finalization-failed".to_owned())
                })
            }
            d2b_provider_notification_desktop::SERVICE_PACKAGE => self
                .reconcile_dependents()
                .map_err(|_| "notification-disconnect-reconcile-failed".to_owned()),
            _ => Ok(()),
        };
        service_cleanup.map_err(|_| "interaction-provider-cleanup-failed".to_owned())?;
        if service.starts_with("d2b.clipboard.") {
            self.pending_picker_receipts.clear();
            self.pending_guest_selection_events.clear();
        }
        Ok(())
    }
}

/// Errors while propagating an authenticated display dependency to U22/U24.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionDependencyError {
    /// No authenticated ComponentSession route was retained.
    SessionUnauthenticated,
    /// The display route did not satisfy the dependent Provider contract.
    DisplayUnavailable,
    /// Clipboard runtime reconciliation failed.
    Clipboard(d2b_provider_clipboard_wayland::ClipboardRuntimeError),
    /// Clipboard runtime could not be constructed.
    ClipboardUnavailable,
    /// Notification runtime admission or reconciliation failed.
    Notification(d2b_provider_notification_desktop::NotificationRuntimeError),
    /// Notification runtime could not be constructed.
    NotificationUnavailable,
}

/// Closed cleanup errors for the daemon composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionFinalizeError {
    /// Display runtime cleanup failed.
    Display(DisplayRuntimeError),
    /// Clipboard drain or authority release failed.
    Clipboard(d2b_provider_clipboard_wayland::ClipboardRuntimeError),
    /// Notification source/authority cleanup failed.
    Notification(d2b_provider_notification_desktop::NotificationRuntimeError),
    /// Bus ingress revocation failed.
    Registration,
    /// Finalization was requested before a display runtime was installed.
    NoDisplayRuntime,
}

impl core::fmt::Display for InteractionFinalizeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Display(_) => "interaction-display-finalization-failed",
            Self::Clipboard(_) => "interaction-clipboard-finalization-failed",
            Self::Notification(_) => "interaction-notification-finalization-failed",
            Self::Registration => "interaction-session-revocation-failed",
            Self::NoDisplayRuntime => "interaction-display-runtime-missing",
        })
    }
}

impl std::error::Error for InteractionFinalizeError {}

/// One daemon-owned process effect adapter for display workers.
///
/// The adapter consumes display tickets into neutral Process tickets, then
/// routes observe/adopt, launch, and exact stop through the shared
/// [`ProcessLaunchEffectPort`].  The Provider never sees the neutral ticket
/// or the retained process identity.
pub struct DisplaySupervisorEffects<S, G = UnavailableGuestFrontendEffects> {
    supervisor: S,
    guest_frontend: G,
    guest_subject: Option<ResourceRef>,
    identities: BTreeMap<DisplayProcessRole, LiveWorker>,
    guest_worker: Option<GuestWorker>,
    consumed_grants: BTreeMap<[u8; 32], u64>,
    tickets: BTreeMap<DisplayProcessRole, ProcessLaunchTicket>,
    last_failures: BTreeMap<DisplayProcessRole, u64>,
    session_digest: [u8; 32],
    reconnect_generation: u64,
    policy_generation: u64,
    teardown_generation: u64,
}

#[derive(Clone, Copy)]
struct LiveWorker {
    identity: ProcessIdentityDigest,
    policy_generation: u64,
    teardown_generation: u64,
    session_digest: [u8; 32],
}

#[derive(Clone, Copy)]
struct GuestWorker {
    policy_generation: u64,
    teardown_generation: u64,
    session_digest: [u8; 32],
}

impl<S> DisplaySupervisorEffects<S, UnavailableGuestFrontendEffects>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
{
    /// Construct a display effect adapter over the daemon supervisor.
    pub fn new(supervisor: S) -> Self {
        Self {
            supervisor,
            guest_frontend: UnavailableGuestFrontendEffects,
            guest_subject: None,
            identities: BTreeMap::new(),
            guest_worker: None,
            consumed_grants: BTreeMap::new(),
            tickets: BTreeMap::new(),
            last_failures: BTreeMap::new(),
            session_digest: [0; 32],
            reconnect_generation: 0,
            policy_generation: 0,
            teardown_generation: 0,
        }
    }

    /// Borrow the supervisor-owned process identities for diagnostics.
    pub fn live_worker_count(&self) -> usize {
        self.identities.len() + usize::from(self.guest_worker.is_some())
    }
}

impl<S, G> DisplaySupervisorEffects<S, G>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    /// Construct a display effect adapter with an independent guest
    /// frontend lifecycle owner.
    pub fn new_with_guest_frontend(supervisor: S, guest_frontend: G) -> Self {
        Self {
            supervisor,
            guest_frontend,
            guest_subject: None,
            identities: BTreeMap::new(),
            guest_worker: None,
            consumed_grants: BTreeMap::new(),
            tickets: BTreeMap::new(),
            last_failures: BTreeMap::new(),
            session_digest: [0; 32],
            reconnect_generation: 0,
            policy_generation: 0,
            teardown_generation: 0,
        }
    }
}

impl<S, G> DisplayProcessEffectPort for DisplaySupervisorEffects<S, G>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    fn current_supervision(&mut self) -> WorkerRestartEvidence {
        let observed_at_ms = daemon_monotonic_ms();
        for role in [
            DisplayProcessRole::HostProxy,
            DisplayProcessRole::GuestFrontend,
        ] {
            let Some(ticket) = self.tickets.get(&role).cloned() else {
                continue;
            };
            let supervisor = self.supervisor.clone();
            let alive = run_effect(move || async move {
                let Some(candidate) = supervisor
                    .observe(&ticket)
                    .await
                    .map_err(|_| WorkerEffectError::WorkerUnavailable)?
                else {
                    return Ok(false);
                };
                Ok(supervisor.open_pidfd(&candidate).await.is_ok())
            })
            .unwrap_or(false);
            if alive {
                self.last_failures.remove(&role);
            } else {
                self.last_failures.insert(role, observed_at_ms);
            }
        }
        WorkerRestartEvidence::from_supervisor(
            observed_at_ms,
            self.last_failures
                .get(&DisplayProcessRole::HostProxy)
                .copied(),
            self.last_failures
                .get(&DisplayProcessRole::GuestFrontend)
                .copied(),
            self.teardown_generation.max(1),
        )
    }

    fn issue_launch_grants(
        &mut self,
        session: &AuthenticatedDisplaySession,
        spec: &WaylandSessionSpec,
        policy: &WaylandPolicySnapshot,
        proof: Option<&DisplayDependencyProof>,
        teardown_generation: u64,
    ) -> Result<LaunchGrants, WorkerEffectError> {
        let session_digest = spec.session_digest(session.controller_generation());
        if let Some(proof) = proof
            && (proof.session_digest() != session_digest
                || proof.reconnect_generation() != session.reconnect_generation()
                || proof.controller_generation() != session.controller_generation()
                || proof.teardown_generation() != teardown_generation
                || proof.policy_generation() != policy.generation())
        {
            return Err(WorkerEffectError::GrantUnavailable);
        }
        self.session_digest = session_digest;
        self.guest_subject = Some(session.guest_ref().clone());
        self.reconnect_generation = session.reconnect_generation();
        self.policy_generation = policy.generation();
        self.teardown_generation = teardown_generation;
        LaunchGrants::issue_for_supervisor(
            session_digest,
            session.reconnect_generation(),
            teardown_generation,
        )
        .map_err(|_| WorkerEffectError::GrantUnavailable)
    }

    fn launch(
        &mut self,
        ticket: d2b_provider_display_wayland::LaunchTicket,
    ) -> Result<WorkerLaunchReceipt, WorkerEffectError> {
        if !ticket.is_current(self.teardown_generation)
            || ticket.policy_generation() != self.policy_generation
        {
            return Err(WorkerEffectError::LaunchRejected);
        }
        let binding = DisplayLaunchBinding::from_ticket(ticket);
        if self
            .consumed_grants
            .insert(binding.attachment_digest(), binding.teardown_generation())
            .is_some()
        {
            return Err(WorkerEffectError::GrantUnavailable);
        }
        if binding.role() == DisplayProcessRole::GuestFrontend {
            let guest = self
                .guest_subject
                .as_ref()
                .ok_or(WorkerEffectError::WorkerUnavailable)?
                .clone();
            if let Some(previous) = self.guest_worker {
                self.guest_frontend
                    .stop(
                        &guest,
                        previous.policy_generation,
                        previous.teardown_generation,
                        previous.session_digest,
                    )
                    .map_err(|_| WorkerEffectError::CleanupIncomplete)?;
                self.guest_worker = None;
            }
            let receipt = self.guest_frontend.ensure(
                &guest,
                binding.policy_generation(),
                binding.teardown_generation(),
                self.session_digest,
            )?;
            self.guest_worker = Some(GuestWorker {
                policy_generation: binding.policy_generation(),
                teardown_generation: binding.teardown_generation(),
                session_digest: self.session_digest,
            });
            return Ok(receipt);
        }
        let process_ticket = process_ticket(&binding)?;
        let role = binding.role();
        if let Some(previous) = self.identities.get(&role).copied() {
            let supervisor = self.supervisor.clone();
            run_effect(move || async move {
                supervisor
                    .stop(&previous.identity, StopClass::Terminate)
                    .await
                    .map_err(|_| WorkerEffectError::CleanupIncomplete)
            })?;
            self.identities.remove(&role);
        }
        let supervisor = self.supervisor.clone();
        let adoption_ticket = process_ticket.clone();
        let adopted = run_effect(move || {
            let supervisor = supervisor.clone();
            let process_ticket = adoption_ticket.clone();
            async move {
                if let Some(candidate) = supervisor
                    .observe(&process_ticket)
                    .await
                    .map_err(|_| WorkerEffectError::WorkerUnavailable)?
                {
                    match supervisor.open_pidfd(&candidate).await {
                        Ok(_) => Ok(candidate.identity),
                        Err(_) => Ok(supervisor
                            .launch(&process_ticket)
                            .await
                            .map_err(|_| WorkerEffectError::LaunchRejected)?
                            .identity),
                    }
                } else {
                    Ok(supervisor
                        .launch(&process_ticket)
                        .await
                        .map_err(|_| WorkerEffectError::LaunchRejected)?
                        .identity)
                }
            }
        })?;
        self.identities.insert(
            role,
            LiveWorker {
                identity: adopted,
                policy_generation: binding.policy_generation(),
                teardown_generation: binding.teardown_generation(),
                session_digest: self.session_digest,
            },
        );
        self.tickets.insert(role, process_ticket);
        Ok(WorkerLaunchReceipt::from_supervisor(
            role,
            WorkerState::Ready { generation: 1 },
            binding.policy_generation(),
            binding.teardown_generation(),
            self.session_digest,
        ))
    }

    fn stop(&mut self, role: DisplayProcessRole) -> Result<WorkerLaunchReceipt, WorkerEffectError> {
        if role == DisplayProcessRole::GuestFrontend {
            let Some(worker) = self.guest_worker else {
                return Ok(WorkerLaunchReceipt::from_supervisor(
                    role,
                    WorkerState::Terminal { deleted: true },
                    self.policy_generation,
                    self.teardown_generation,
                    self.session_digest,
                ));
            };
            let guest = self
                .guest_subject
                .as_ref()
                .ok_or(WorkerEffectError::WorkerUnavailable)?
                .clone();
            let receipt = self.guest_frontend.stop(
                &guest,
                worker.policy_generation,
                worker.teardown_generation,
                worker.session_digest,
            )?;
            self.guest_worker = None;
            return Ok(receipt);
        }
        let Some(worker) = self.identities.get(&role).copied() else {
            return Ok(WorkerLaunchReceipt::from_supervisor(
                role,
                WorkerState::Terminal { deleted: true },
                self.policy_generation,
                self.teardown_generation,
                self.session_digest,
            ));
        };
        let supervisor = self.supervisor.clone();
        run_effect(move || async move {
            supervisor
                .stop(&worker.identity, StopClass::Terminate)
                .await
                .map_err(|_| WorkerEffectError::CleanupIncomplete)
        })?;
        self.identities.remove(&role);
        self.tickets.remove(&role);
        self.last_failures.remove(&role);
        Ok(WorkerLaunchReceipt::from_supervisor(
            role,
            WorkerState::Terminal { deleted: true },
            worker.policy_generation,
            worker.teardown_generation,
            worker.session_digest,
        ))
    }

    fn delete_runtime_volume(&mut self) -> Result<VolumeState, WorkerEffectError> {
        Ok(VolumeState::Deleted)
    }

    fn revoke_portal(&mut self) -> Result<CleanupState, WorkerEffectError> {
        Ok(CleanupState::Complete)
    }

    fn release_principal(&mut self) -> Result<CleanupState, WorkerEffectError> {
        Ok(CleanupState::Complete)
    }

    fn release_authority(&mut self) -> Result<CleanupState, WorkerEffectError> {
        if self.identities.is_empty() && self.guest_worker.is_none() {
            Ok(CleanupState::Complete)
        } else {
            Err(WorkerEffectError::CleanupIncomplete)
        }
    }
}

/// Daemon-owned bounded drain state for clipboard and notification services.
#[derive(Default)]
pub struct InteractionDrainEffects {
    drained: bool,
    authority_released: bool,
    audit_events: usize,
    notification_lifecycle:
        Option<NotificationLifecycleSupervisor<InteractionNotificationLifecycleBackend>>,
    notification_recovered: bool,
}

/// Daemon-owned presentation adapter for the bounded desktop sink.  It
/// represents an already-admitted host presentation connection; no address,
/// bus name, or standalone service is stored here.
#[derive(Debug, Default)]
pub struct InteractionNotificationPort {
    next_id: u32,
    presented: VecDeque<d2b_provider_notification_desktop::SanitizedNotification>,
    active: bool,
}

impl DesktopNotificationPort for InteractionNotificationPort {
    fn activate(&mut self) -> Result<(), d2b_provider_notification_desktop::SinkError> {
        self.active = true;
        Ok(())
    }

    fn deactivate(&mut self) -> Result<(), d2b_provider_notification_desktop::SinkError> {
        self.active = false;
        self.presented.clear();
        Ok(())
    }

    fn notify(
        &mut self,
        notification: &d2b_provider_notification_desktop::SanitizedNotification,
    ) -> Result<u32, d2b_provider_notification_desktop::SinkError> {
        if !self.active || self.presented.len() >= 64 {
            return Err(d2b_provider_notification_desktop::SinkError::Unavailable);
        }
        self.presented.push_back(notification.clone());
        self.next_id = self.next_id.wrapping_add(1).max(1);
        Ok(self.next_id)
    }
}

/// Daemon-owned desktop presentation effect backed by the authenticated
/// session notification service.
#[derive(Debug, Default)]
pub struct NotifyRustNotificationPort {
    handles: VecDeque<notify_rust::NotificationHandle>,
    active: bool,
}

impl DesktopNotificationPort for NotifyRustNotificationPort {
    fn activate(&mut self) -> Result<(), d2b_provider_notification_desktop::SinkError> {
        self.active = true;
        Ok(())
    }

    fn deactivate(&mut self) -> Result<(), d2b_provider_notification_desktop::SinkError> {
        self.active = false;
        while let Some(handle) = self.handles.pop_front() {
            handle.close();
        }
        Ok(())
    }

    fn notify(
        &mut self,
        notification: &d2b_provider_notification_desktop::SanitizedNotification,
    ) -> Result<u32, d2b_provider_notification_desktop::SinkError> {
        if !self.active {
            return Err(d2b_provider_notification_desktop::SinkError::Unavailable);
        }
        let mut desktop = DesktopNotification::new();
        desktop
            .appname("d2bd")
            .summary(notification.summary())
            .body(notification.body())
            .urgency(match notification.urgency() {
                d2b_provider_notification_desktop::NotificationUrgency::Low => Urgency::Low,
                d2b_provider_notification_desktop::NotificationUrgency::Normal => Urgency::Normal,
                d2b_provider_notification_desktop::NotificationUrgency::Critical => {
                    Urgency::Critical
                }
            })
            .timeout(Duration::from_secs(u64::from(
                notification.expire_timeout_secs(),
            )));
        if let Some(icon) = notification.icon_ref() {
            desktop.icon(icon);
        }
        for (action_key, label) in notification.actions() {
            desktop.action(action_key, label);
        }
        let handle = desktop
            .show()
            .map_err(|_| d2b_provider_notification_desktop::SinkError::Unavailable)?;
        let id = handle.id();
        if self.handles.len() >= 64
            && let Some(old) = self.handles.pop_front()
        {
            old.close();
        }
        self.handles.push_back(handle);
        Ok(id)
    }
}

struct InteractionNotificationLifecycleState {
    sources: std::collections::BTreeSet<NotificationSourceIdentity>,
    host_sink: Option<NotificationHostSinkIdentity>,
}

struct InteractionNotificationLifecycleBackend {
    state: Mutex<InteractionNotificationLifecycleState>,
    port: Arc<Mutex<Box<dyn DesktopNotificationPort + Send>>>,
}

impl InteractionNotificationLifecycleBackend {
    fn new(port: Arc<Mutex<Box<dyn DesktopNotificationPort + Send>>>) -> Self {
        Self {
            state: Mutex::new(InteractionNotificationLifecycleState {
                sources: std::collections::BTreeSet::new(),
                host_sink: None,
            }),
            port,
        }
    }
}

impl NotificationLifecycleBackend for InteractionNotificationLifecycleBackend {
    fn start_source(&self, source: &NotificationSourceIdentity) -> Result<(), &'static str> {
        self.state
            .lock()
            .map_err(|_| "notification-source-lifecycle-unavailable")?
            .sources
            .insert(source.clone());
        Ok(())
    }

    fn stop_source(&self, source: &NotificationSourceIdentity) -> Result<(), &'static str> {
        if self
            .state
            .lock()
            .map_err(|_| "notification-source-lifecycle-unavailable")?
            .sources
            .remove(source)
        {
            Ok(())
        } else {
            Err("notification-source-lifecycle-mismatch")
        }
    }

    fn start_host_sink(&self, sink: &NotificationHostSinkIdentity) -> Result<(), &'static str> {
        self.port
            .lock()
            .map_err(|_| "notification-host-sink-unavailable")?
            .activate()
            .map_err(|_| "notification-host-sink-unavailable")?;
        self.state
            .lock()
            .map_err(|_| "notification-host-sink-unavailable")?
            .host_sink = Some(sink.clone());
        Ok(())
    }

    fn stop_host_sink(&self, sink: &NotificationHostSinkIdentity) -> Result<(), &'static str> {
        {
            let state = self
                .state
                .lock()
                .map_err(|_| "notification-host-sink-unavailable")?;
            if state.host_sink.as_ref() != Some(sink) {
                return Err("notification-host-sink-lifecycle-mismatch");
            }
        }
        self.port
            .lock()
            .map_err(|_| "notification-host-sink-unavailable")?
            .deactivate()
            .map_err(|_| "notification-host-sink-unavailable")?;
        self.state
            .lock()
            .map_err(|_| "notification-host-sink-unavailable")?
            .host_sink = None;
        Ok(())
    }

    fn observe(
        &self,
        _zone: &ZoneId,
        _provider_ref: &ResourceRef,
    ) -> Result<NotificationLifecycleObservation, &'static str> {
        let state = self
            .state
            .lock()
            .map_err(|_| "notification-source-lifecycle-unavailable")?;
        Ok(NotificationLifecycleObservation::new(
            state.sources.iter().cloned().collect(),
            state.host_sink.clone(),
        ))
    }
}

impl core::fmt::Debug for InteractionDrainEffects {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("InteractionDrainEffects(<redacted>)")
    }
}

impl InteractionDrainEffects {
    fn new(port: Arc<Mutex<Box<dyn DesktopNotificationPort + Send>>>) -> Self {
        Self {
            notification_lifecycle: Some(NotificationLifecycleSupervisor::new(
                InteractionNotificationLifecycleBackend::new(port),
            )),
            ..Self::default()
        }
    }

    /// Whether all daemon-owned workers have been drained.
    pub const fn drained(&self) -> bool {
        self.drained
    }

    /// Whether the final session authority release completed.
    pub const fn authority_released(&self) -> bool {
        self.authority_released
    }
}

impl ClipboardProcessEffectPort for InteractionDrainEffects {
    fn drain(&mut self) -> Result<(), ClipboardServiceError> {
        self.drained = true;
        Ok(())
    }

    fn release_authority(&mut self) -> Result<(), ClipboardServiceError> {
        if !self.drained {
            return Err(ClipboardServiceError::AuthorityReleaseIncomplete);
        }
        self.authority_released = true;
        Ok(())
    }
}

impl d2b_provider_clipboard_wayland::ClipboardAuditSink for InteractionDrainEffects {
    type Error = &'static str;

    fn publish(
        &mut self,
        event: &d2b_provider_clipboard_wayland::ClipboardAuditEvent,
    ) -> Result<(), Self::Error> {
        if event.to_wire().is_empty() {
            return Err("clipboard-audit-empty");
        }
        self.audit_events = self.audit_events.saturating_add(1);
        Ok(())
    }
}

impl SourceProcessEffectPort for InteractionDrainEffects {
    fn apply(
        &mut self,
        plan: &SourceReconcileResult,
        lifecycle: &NotificationLifecyclePlan,
    ) -> Result<SourceProcessEffectReceipt, &'static str> {
        let supervisor = self
            .notification_lifecycle
            .as_ref()
            .ok_or("notification-supervisor-unavailable")?;
        if !self.notification_recovered {
            supervisor.recover(lifecycle.zone(), lifecycle.provider_ref())?;
            self.notification_recovered = true;
        }
        let receipt = supervisor.apply(lifecycle)?;
        SourceProcessEffectReceipt::from_supervisor(plan, lifecycle, &receipt)
    }
}

impl NotificationProcessEffectPort for InteractionDrainEffects {
    fn release_authority(&mut self) -> Result<(), &'static str> {
        if self
            .notification_lifecycle
            .as_ref()
            .ok_or("notification-supervisor-unavailable")?
            .is_drained()?
        {
            self.authority_released = true;
            Ok(())
        } else {
            Err("notification-authority-release-incomplete")
        }
    }
}

fn process_ticket(
    binding: &DisplayLaunchBinding,
) -> Result<ProcessLaunchTicket, WorkerEffectError> {
    let role_name = match binding.role() {
        DisplayProcessRole::HostProxy => "host-proxy",
        DisplayProcessRole::GuestFrontend => "guest-frontend",
    };
    let process_ref = ResourceRef::parse(&format!("EphemeralProcess/display-{role_name}"))
        .map_err(|_| WorkerEffectError::LaunchRejected)?;
    let execution_ref =
        ResourceRef::parse("Host/host-system").map_err(|_| WorkerEffectError::LaunchRejected)?;
    let owner_provider =
        BoundedToken::parse("display-wayland").map_err(|_| WorkerEffectError::LaunchRejected)?;
    let component =
        BoundedToken::parse(role_name).map_err(|_| WorkerEffectError::LaunchRejected)?;
    let template =
        BoundedToken::parse("display-worker").map_err(|_| WorkerEffectError::LaunchRejected)?;
    let selected_provider =
        BoundedToken::parse("system-systemd").map_err(|_| WorkerEffectError::LaunchRejected)?;
    let process_uid = resource_uid(binding, b"process");
    let operation_uid = resource_uid(binding, b"operation");
    let generation = ResourceGeneration::new(binding.policy_generation())
        .map_err(|_| WorkerEffectError::LaunchRejected)?;
    let controller_generation = ControllerGeneration::new(binding.teardown_generation())
        .map_err(|_| WorkerEffectError::LaunchRejected)?;
    let digests = CompiledDigests {
        sandbox: configuration_digest(binding, b"sandbox"),
        budget: configuration_digest(binding, b"budget"),
        mounts: configuration_digest(binding, b"mounts"),
        devices: configuration_digest(binding, b"devices"),
        network: configuration_digest(binding, b"network"),
        endpoints: configuration_digest(binding, b"endpoints"),
        fd_table: configuration_digest(binding, b"fd-table"),
    };
    let operation = d2b_process::OperationBinding::new(operation_uid, 30_000)
        .map_err(|_| WorkerEffectError::LaunchRejected)?;
    let expected_identity = [
        IdentityBinding::Cgroup,
        IdentityBinding::Executable,
        IdentityBinding::Generation,
        IdentityBinding::Template,
    ]
    .into_iter()
    .collect();
    ProcessLaunchTicket::new(
        process_ref,
        process_uid,
        generation,
        controller_generation,
        owner_provider,
        component,
        template,
        execution_ref,
        ExecutionDomain::System,
        None,
        selected_provider,
        digests,
        operation,
        expected_identity,
    )
    .map(|ticket| {
        ticket
            .with_readiness(ReadinessExpectation::condition(1_000).expect("fixed readiness"))
            .with_inherited_fd_count(2)
            .expect("fixed inherited descriptor bound")
    })
    .map_err(|_| WorkerEffectError::LaunchRejected)
}

fn display_sandbox_plan(
    intent: &ResolvedRunnerIntent,
    domain: ExecutionDomain,
) -> Result<SandboxLaunchPlan, ProcessEffectError> {
    let namespace_classes = [
        (intent.namespaces.user, NamespaceClass::User),
        (intent.namespaces.pid, NamespaceClass::Pid),
        (intent.namespaces.mount, NamespaceClass::Mount),
        (intent.namespaces.ipc, NamespaceClass::Ipc),
        (intent.namespaces.uts, NamespaceClass::Uts),
        (intent.namespaces.net, NamespaceClass::Network),
    ]
    .into_iter()
    .filter_map(|(enabled, class)| enabled.then_some(class))
    .collect();
    let capability_classes = intent
        .capabilities
        .iter()
        .map(|capability| match capability.as_str() {
            "CAP_NET_BIND_SERVICE" => Ok(CapabilityClass::NetworkBind),
            "CAP_NET_RAW" => Ok(CapabilityClass::NetworkRaw),
            "CAP_NET_ADMIN" => Ok(CapabilityClass::NetworkAdmin),
            "CAP_SYS_TIME" => Ok(CapabilityClass::SysTime),
            "CAP_SYS_PTRACE" => Ok(CapabilityClass::SysPtrace),
            "CAP_SYS_ADMIN" => Ok(CapabilityClass::SysAdmin),
            "CAP_DAC_OVERRIDE" => Ok(CapabilityClass::DacOverride),
            "CAP_FOWNER" => Ok(CapabilityClass::Fowner),
            "CAP_CHOWN" => Ok(CapabilityClass::Chown),
            "CAP_SETUID" => Ok(CapabilityClass::Setuid),
            "CAP_SETGID" => Ok(CapabilityClass::Setgid),
            "CAP_AUDIT_WRITE" => Ok(CapabilityClass::AuditWrite),
            "CAP_KILL" => Ok(CapabilityClass::Kill),
            _ => Err(ProcessEffectError::ResolutionFailed),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let seccomp_class = intent
        .seccomp_policy_ref
        .as_deref()
        .ok_or(ProcessEffectError::ResolutionFailed)
        .and_then(|reference| {
            BoundedToken::parse(reference).map_err(|_| ProcessEffectError::ResolutionFailed)
        })?;
    let spec = d2b_contracts::v3::process::SandboxSpec::new(
        namespace_classes,
        capability_classes,
        seccomp_class,
        true,
        intent.root_carve_out,
        EnvironmentClass::Minimal,
        true,
        intent.umask.map(|umask| format!("{umask:o}")),
        0,
        intent.user_namespace.map(|_| UserNamespaceSpec {
            mapping_class: MappingClass::ProcessPrincipalRoot,
        }),
    )
    .map_err(|_| ProcessEffectError::ResolutionFailed)?;
    let compiled = SandboxCompiler
        .compile_plan(&spec, domain, false)
        .map_err(|_| ProcessEffectError::ResolutionFailed)?;
    Ok(SandboxLaunchPlan {
        digest: compiled.compiled().digest().to_hex(),
        domain,
        namespace_classes: spec.namespace_classes().to_vec(),
        capability_classes: spec.capability_classes().to_vec(),
        seccomp_class: spec.seccomp_class().clone(),
        no_new_privileges: spec.no_new_privileges(),
        start_root: spec.start_root(),
        environment_class: spec.environment_class(),
        read_only_root: spec.read_only_root(),
        umask: spec.umask().map(str::to_owned),
        oom_score_adj: spec.oom_score_adj(),
        user_namespace: spec.user_namespace().copied(),
    })
}

/// Daemon-owned resolver for display worker tickets.
///
/// The generic Process ticket intentionally contains no executable or broker
/// role. This resolver binds the display worker to exactly one trusted
/// Wayland-proxy row from the loaded bundle and retains only broker-safe
/// observations for same-daemon reconciliation.
#[derive(Clone)]
pub struct BundleDisplayLaunchResolver {
    bundle: Arc<BundleResolver>,
    observations: Arc<Mutex<BTreeMap<String, BrokerObservedProcess>>>,
    observation_path: Option<Arc<PathBuf>>,
}

impl BundleDisplayLaunchResolver {
    /// Bind display launch resolution to the daemon's verified bundle.
    pub fn new(bundle: BundleResolver) -> Self {
        Self::with_observation_path_inner(bundle, None)
    }

    /// Bind resolution to the verified bundle and retain broker observations
    /// across a daemon restart for pidfd adoption.
    pub fn with_observation_path(bundle: BundleResolver, path: PathBuf) -> Self {
        Self::with_observation_path_inner(bundle, Some(path))
    }

    fn with_observation_path_inner(bundle: BundleResolver, path: Option<PathBuf>) -> Self {
        let observation_path = path.map(Arc::new);
        let observations = observation_path
            .as_deref()
            .and_then(|path| load_observations(path).ok())
            .unwrap_or_default();
        Self {
            bundle: Arc::new(bundle),
            observations: Arc::new(Mutex::new(observations)),
            observation_path,
        }
    }

    fn resolve_intent(&self) -> Result<&ResolvedRunnerIntent, ProcessEffectError> {
        let mut candidates = self
            .bundle
            .runner_intent_ids()
            .filter_map(|id| self.bundle.find_runner_intent(id))
            .filter(|intent| intent.role == ProcessRole::WaylandProxy);
        let Some(intent) = candidates.next() else {
            return Err(ProcessEffectError::ResolutionFailed);
        };
        if candidates.next().is_some() {
            return Err(ProcessEffectError::ResolutionFailed);
        }
        Ok(intent)
    }

    fn observation_key(observed: &BrokerObservedProcess) -> String {
        format!(
            "{}:{}:{}",
            observed.intent.vm_id.as_str(),
            observed.intent.role_id.as_str(),
            observed.pid
        )
    }

    fn ticket_observation_key(request: &ProcessRequest) -> String {
        request.ticket().process_uid().as_str().to_owned()
    }

    fn persist(&self) {
        let Some(path) = self.observation_path.as_deref() else {
            return;
        };
        let Ok(observations) = self.observations.lock() else {
            return;
        };
        let records = observations
            .iter()
            .map(|(process_uid, observed)| PersistedObservation {
                process_uid: process_uid.clone(),
                vm_id: observed.intent.vm_id.as_str().to_owned(),
                role_id: observed.intent.role_id.as_str().to_owned(),
                role: observed.intent.role,
                bundle_runner_intent_ref: observed
                    .intent
                    .bundle_runner_intent_ref
                    .as_str()
                    .to_owned(),
                execution_ref: observed.intent.execution_ref.clone(),
                domain: observed.intent.domain,
                user_ref: observed.intent.user_ref.clone(),
                provider_identity: observed.intent.provider_identity,
                template_identity: observed.intent.template_identity,
                generation: observed.intent.generation,
                resource_ref: observed.intent.resource_ref.clone(),
                resource_uid: observed.intent.resource_uid.clone(),
                bundle_content_identity: observed.intent.bundle_content_identity.clone(),
                sandbox_plan: observed.intent.sandbox_plan.clone(),
                pid: observed.pid,
                start_time_ticks: observed.start_time_ticks,
                cgroup_verified: observed.cgroup_verified,
                executable_verified: observed.executable_verified,
            })
            .collect::<Vec<_>>();
        let encoded = match serde_json::to_vec(&records) {
            Ok(encoded) => encoded,
            Err(_) => return,
        };
        let temporary = path.with_extension("json.new");
        if std::fs::write(&temporary, encoded).is_ok() {
            let _ = std::fs::rename(temporary, path);
        }
    }
}

impl BrokerLaunchResolver for BundleDisplayLaunchResolver {
    fn resolve(&self, request: &ProcessRequest) -> Result<BrokerLaunchIntent, ProcessEffectError> {
        let ticket = request.ticket();
        if ticket.owner_provider().as_str() != "display-wayland"
            || ticket.component().as_str() != "host-proxy"
            || ticket.execution_ref().resource_type().as_str() != "Host"
        {
            return Err(ProcessEffectError::UnsupportedProvider);
        }
        let intent = self.resolve_intent()?;
        let provider_identity = digest_identity("provider", ticket.owner_provider().as_str());
        let template_identity = digest_identity("template", ticket.template().as_str());
        let sandbox_plan = display_sandbox_plan(intent, ticket.domain())?;
        Ok(BrokerLaunchIntent {
            vm_id: VmId::new(intent.vm_name.clone()),
            execution_ref: ticket.execution_ref().clone(),
            domain: ticket.domain(),
            user_ref: ticket.user_ref().cloned(),
            role_id: RoleId::new(intent.role_id.clone()),
            role: RunnerRole::WaylandProxy,
            bundle_runner_intent_ref: BundleOpId::new(intent.intent_id.clone()),
            provider_identity,
            template_identity,
            generation: ticket.resource_generation().get(),
            resource_ref: ticket.process_ref().clone(),
            resource_uid: ticket.process_uid().clone(),
            bundle_content_identity: self.bundle.audit_bundle_hash().to_owned(),
            sandbox_plan: Some(sandbox_plan),
        })
    }

    fn observe(
        &self,
        request: &ProcessRequest,
    ) -> Result<Option<BrokerObservedProcess>, ProcessEffectError> {
        let expected = self.resolve_intent()?;
        let key = Self::ticket_observation_key(request);
        let candidate = self
            .observations
            .lock()
            .map_err(|_| ProcessEffectError::ObserveFailed)
            .map(|observations| observations.get(&key).cloned())?;
        Ok(candidate.filter(|observed| {
            observed.intent.vm_id.as_str() == expected.vm_name
                && observed.intent.role_id.as_str() == expected.role_id
                && observed.intent.bundle_runner_intent_ref.as_str() == expected.intent_id
                && observed.intent.role == RunnerRole::WaylandProxy
                && observed.intent.generation == request.ticket().resource_generation().get()
        }))
    }

    fn record_launched(&self, request: &ProcessRequest, observed: &BrokerObservedProcess) {
        if let Ok(mut observations) = self.observations.lock() {
            observations.insert(Self::ticket_observation_key(request), observed.clone());
        }
        self.persist();
    }

    fn record_stopped(&self, observed: &BrokerObservedProcess) {
        if let Ok(mut observations) = self.observations.lock() {
            let key = Self::observation_key(observed);
            observations.retain(|_, candidate| Self::observation_key(candidate) != key);
        }
        self.persist();
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PersistedObservation {
    process_uid: String,
    vm_id: String,
    execution_ref: ResourceRef,
    domain: ExecutionDomain,
    user_ref: Option<ResourceRef>,
    role_id: String,
    role: RunnerRole,
    bundle_runner_intent_ref: String,
    provider_identity: [u8; 32],
    template_identity: [u8; 32],
    generation: u64,
    resource_ref: ResourceRef,
    resource_uid: ResourceUid,
    bundle_content_identity: String,
    sandbox_plan: Option<SandboxLaunchPlan>,
    pid: i32,
    start_time_ticks: u64,
    cgroup_verified: bool,
    executable_verified: bool,
}

fn load_observations(
    path: &std::path::Path,
) -> Result<BTreeMap<String, BrokerObservedProcess>, ProcessEffectError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(_) => return Err(ProcessEffectError::ObserveFailed),
    };
    let records = serde_json::from_slice::<Vec<PersistedObservation>>(&bytes)
        .map_err(|_| ProcessEffectError::ObserveFailed)?;
    Ok(records
        .into_iter()
        .map(|record| {
            let observed = BrokerObservedProcess {
                intent: BrokerLaunchIntent {
                    vm_id: VmId::new(record.vm_id),
                    execution_ref: record.execution_ref,
                    domain: record.domain,
                    user_ref: record.user_ref,
                    role_id: RoleId::new(record.role_id),
                    role: record.role,
                    bundle_runner_intent_ref: BundleOpId::new(record.bundle_runner_intent_ref),
                    provider_identity: record.provider_identity,
                    template_identity: record.template_identity,
                    generation: record.generation,
                    resource_ref: record.resource_ref,
                    resource_uid: record.resource_uid,
                    bundle_content_identity: record.bundle_content_identity,
                    sandbox_plan: record.sandbox_plan,
                },
                pid: record.pid,
                start_time_ticks: record.start_time_ticks,
                cgroup_verified: record.cgroup_verified,
                executable_verified: record.executable_verified,
            };
            (record.process_uid, observed)
        })
        .collect())
}

fn digest_identity(label: &str, value: &str) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2bd-display-broker-identity-v1");
    digest.update(label.as_bytes());
    digest.update([0]);
    digest.update(value.as_bytes());
    digest.finalize().into()
}

/// Construct the production broker-backed supervisor used by interaction
/// runtimes. The caller role is daemon-derived; Providers never receive it.
pub fn production_display_supervisor(
    bundle: BundleResolver,
    daemon_uid: u32,
    observation_path: PathBuf,
) -> ProviderSupervisor<BrokerProcessBackend<BundleDisplayLaunchResolver>> {
    let backend = BrokerProcessBackend::with_socket_and_role(
        BundleDisplayLaunchResolver::with_observation_path(bundle, observation_path),
        d2b_contracts::BROKER_SOCKET_PATH,
        std::time::Duration::from_secs(10),
        BrokerCallerRole::RootUid { uid: daemon_uid },
    );
    ProviderSupervisor::new(backend)
}

/// Committed resource-plane state required to construct one interaction runtime.
pub(crate) struct ProductionInteractionResourceState<'a> {
    zone: ZoneId,
    committed_policy: PolicySnapshot,
    resource_revision: ZoneRevision,
    resource_ready: bool,
    configuration: Option<&'a CommittedInteractionProviderConfiguration>,
}

impl<'a> ProductionInteractionResourceState<'a> {
    /// Bind the exact committed state for one ready Zone.
    pub(crate) const fn new(
        zone: ZoneId,
        committed_policy: PolicySnapshot,
        resource_revision: ZoneRevision,
        resource_ready: bool,
        configuration: Option<&'a CommittedInteractionProviderConfiguration>,
    ) -> Self {
        Self {
            zone,
            committed_policy,
            resource_revision,
            resource_ready,
            configuration,
        }
    }
}

/// Construct the daemon-owned authenticated interaction composition for one
/// trusted Zone.  The registrar is created here, rather than in Provider
/// code, and its production resolver derives the Provider identity from the
/// verified local peer.
pub(crate) fn production_interaction_composition(
    bundle: BundleResolver,
    daemon_uid: u32,
    observation_path: PathBuf,
    resource: ProductionInteractionResourceState<'_>,
) -> Result<
    InteractionComposition<
        ProviderSupervisor<BrokerProcessBackend<BundleDisplayLaunchResolver>>,
        AuthenticatedGuestFrontendEffects,
    >,
    BusError,
> {
    let catalog = ApiCatalog::standard();
    let subject_ref = ResourceRef::parse(&format!("Guest/uid-{daemon_uid}"))
        .map_err(|_| BusError::InvalidConfig)?;
    let subject_uid = unix_guest_subject_uid(daemon_uid);
    let rule = PolicyRule::new(
        &catalog,
        [],
        [],
        [
            SessionVerb::Connect,
            SessionVerb::Invoke,
            SessionVerb::OpenStream,
            SessionVerb::Cancel,
            SessionVerb::Observe,
            SessionVerb::AuditExport,
            SessionVerb::SupportBundle,
        ],
        [],
        [],
        [resource.zone.clone()],
        [],
    )
    .map_err(|_| BusError::InvalidConfig)?;
    let role = CompiledRole::new(
        ResourceRef::parse("Role/interaction-provider").expect("fixed role reference"),
        vec![rule],
    )
    .map_err(|_| BusError::InvalidConfig)?;
    let binding = CompiledRoleBinding::new(
        role.role_ref.clone(),
        [BoundSubject {
            subject_ref,
            subject_uid,
        }],
        BindingScope::default(),
        d2b_resource_api::authz::RelayGrantAuthority::None,
    )
    .map_err(|_| BusError::InvalidConfig)?;
    let policy = PolicySet::new(
        &catalog,
        resource.committed_policy.policy_revision,
        vec![role],
        vec![binding],
    )
    .map_err(|_| BusError::InvalidConfig)?;
    let native =
        NativeAuthorizer::new(catalog, Some(policy)).map_err(|_| BusError::InvalidConfig)?;
    let state = d2b_resource_api::authz::AuthorizationState {
        snapshot: resource.committed_policy,
        zone_policy_revision: resource.resource_revision,
        bootstrap_phase: BootstrapPhase::Disabled,
        now_tick: 1,
    };
    let committed_policy = state.snapshot;
    let authorizer = BusAuthorizer::new(native, state).map_err(|_| BusError::InvalidConfig)?;
    let (_bus, registrar) = ZoneBus::with_observer_and_metrics(
        resource.zone.clone(),
        authorizer,
        BusConfig::default(),
        std::sync::Arc::new(NoopBusObserver),
        std::sync::Arc::new(d2b_bus::metrics::NoopBusTelemetry),
    )?;
    let expected_state_root_gid = Group::from_name("users")
        .ok()
        .flatten()
        .map(|group| group.gid.as_raw())
        .unwrap_or_else(|| getgid().as_raw());
    let guest_frontend = AuthenticatedGuestFrontendEffects::new(
        bundle.clone(),
        PathBuf::from(d2b_contracts::BROKER_SOCKET_PATH),
        BrokerCallerRole::RootUid { uid: daemon_uid },
        daemon_uid,
        expected_state_root_gid,
    );
    let mut composition = InteractionComposition::new_with_guest_frontend_and_notification_port(
        registrar,
        production_display_supervisor(bundle, daemon_uid, observation_path),
        guest_frontend,
        Box::new(NotifyRustNotificationPort::default()),
    );
    if let Some(configuration) = resource.configuration {
        composition
            .bind_interaction_provider_configuration(configuration)
            .map_err(|_| BusError::InvalidConfig)?;
    }
    let observer_user_ref = resource
        .configuration
        .and_then(CommittedInteractionProviderConfiguration::notification)
        .map(|notification| notification.observer_user_ref().clone())
        .or_else(|| {
            resource
                .configuration
                .and_then(CommittedInteractionProviderConfiguration::clipboard)
                .map(|clipboard| clipboard.host_user_ref().clone())
        })
        .unwrap_or(
            ResourceRef::parse(&format!("User/uid-{daemon_uid}"))
                .map_err(|_| BusError::InvalidConfig)?,
        );
    let policy_ref = ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/display-wayland")
        .map_err(|_| BusError::InvalidConfig)?;
    let dependencies = DependencyState::ready().with_zone(resource.zone);
    let evidence = CoreDisplayResourceEvidence::from_committed_policy(
        policy_ref,
        committed_policy,
        committed_policy.policy_revision,
        FilterInput::default(),
        FilterInput::default(),
        dependencies,
        observer_user_ref,
        resource.resource_revision,
        resource.resource_ready,
    )
    .map_err(|_| BusError::InvalidConfig)?;
    composition.bind_display_resource_evidence(evidence);
    Ok(composition)
}

fn unix_guest_subject_uid(uid: u32) -> ResourceUid {
    let mut digest = Sha256::new();
    digest.update(b"d2b-unix-guest-subject-v1");
    digest.update(uid.to_be_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest.finalize()[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    ResourceUid::parse(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    ))
    .expect("digest-derived guest UID is valid")
}

/// Bind and retain the daemon-owned ComponentSession listeners for all
/// interaction Provider service packages.  Providers do not open these
/// sockets and no Provider-owned service unit is created.
pub fn spawn_interaction_listeners<S, G>(
    runtime: Arc<AsyncMutex<Option<InteractionRuntimeSet<S, G>>>>,
    state_dir: PathBuf,
    zone: ZoneId,
    expected_peer_uid: u32,
) -> Result<InteractionListenerSet, String>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    spawn_interaction_listeners_with_stop(
        runtime,
        state_dir,
        zone,
        expected_peer_uid,
        Arc::new(AtomicBool::new(false)),
    )
}

/// Bind listeners using an existing shutdown token so independently
/// Zone-bound listener sets can be stopped as one daemon-owned group.
pub fn spawn_interaction_listeners_with_stop<S, G>(
    runtime: Arc<AsyncMutex<Option<InteractionRuntimeSet<S, G>>>>,
    state_dir: PathBuf,
    zone: ZoneId,
    expected_peer_uid: u32,
    stop: Arc<AtomicBool>,
) -> Result<InteractionListenerSet, String>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    ensure_owned_state_dir(&state_dir, expected_peer_uid).map_err(|error| error.to_string())?;
    let state_metadata =
        std::fs::symlink_metadata(&state_dir).map_err(|error| error.to_string())?;
    if state_metadata.file_type().is_symlink()
        || !state_metadata.is_dir()
        || state_metadata.uid() != expected_peer_uid
        || state_metadata.mode() & 0o022 != 0
    {
        return Err("interaction-listener-state-directory-ownership".to_owned());
    }

    fn ensure_owned_state_dir(path: &std::path::Path, expected_uid: u32) -> std::io::Result<()> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || !metadata.is_dir()
                    || metadata.uid() != expected_uid
                    || metadata.mode() & 0o022 != 0
                {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "interaction listener state directory is not daemon-owned",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    let parent_metadata = std::fs::symlink_metadata(parent)?;
                    if parent_metadata.file_type().is_symlink()
                        || !parent_metadata.is_dir()
                        || parent_metadata.mode() & 0o002 != 0
                    {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "interaction listener parent directory is unsafe",
                        ));
                    }
                }
                std::fs::create_dir_all(path)?;
                let metadata = std::fs::symlink_metadata(path)?;
                if metadata.file_type().is_symlink()
                    || !metadata.is_dir()
                    || metadata.uid() != expected_uid
                    || metadata.mode() & 0o022 != 0
                {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "interaction listener state directory ownership changed",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }
    let mut paths = Vec::with_capacity(INTERACTION_SERVICES.len());
    let handlers = Arc::new(Mutex::new(Vec::new()));
    let active_handlers = Arc::new(AtomicUsize::new(0));
    let mut threads = Vec::with_capacity(INTERACTION_SERVICES.len());
    for (service, _) in INTERACTION_SERVICES {
        let slug = service.replace('.', "-");
        let path = state_dir.join(format!("interaction-{slug}.sock"));
        let listener = bind_interaction_listener(&path, expected_peer_uid)
            .map_err(|error| format!("bind interaction listener {}: {error}", path.display()))?;
        let runtime = Arc::clone(&runtime);
        let zone = zone.clone();
        let service = (*service).to_owned();
        let failure_stop = Arc::clone(&stop);
        let thread_name = format!("d2bd-interaction-{}", service.replace('.', "-"));
        let context = InteractionAcceptContext {
            runtime,
            zone,
            service,
            expected_peer_uid,
            stop: Arc::clone(&stop),
            handlers: Arc::clone(&handlers),
            active_handlers: Arc::clone(&active_handlers),
        };
        thread::Builder::new()
            .name(thread_name)
            .spawn(move || interaction_accept_loop(listener, context))
            .map(|thread| threads.push(thread))
            .map_err(|error| {
                failure_stop.store(true, Ordering::Release);
                for thread in threads.drain(..) {
                    let _ = thread.join();
                }
                error.to_string()
            })?;
        paths.push(path);
    }
    let socket_identities = paths
        .iter()
        .map(|path| {
            let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
            Ok((metadata.dev(), metadata.ino()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let parent_identities = paths
        .iter()
        .map(|path| {
            let parent = path
                .parent()
                .ok_or_else(|| "interaction-listener-parent-missing".to_owned())?;
            let metadata = std::fs::symlink_metadata(parent).map_err(|error| error.to_string())?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("interaction-listener-parent-invalid".to_owned());
            }
            Ok((metadata.dev(), metadata.ino()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(InteractionListenerSet {
        paths,
        socket_identities,
        parent_identities,
        stop,
        threads: Mutex::new(threads),
        handlers,
    })
}

/// Daemon-owned handles for the interaction listener set.
pub struct InteractionListenerSet {
    paths: Vec<PathBuf>,
    socket_identities: Vec<(u64, u64)>,
    parent_identities: Vec<(u64, u64)>,
    stop: Arc<AtomicBool>,
    threads: Mutex<Vec<thread::JoinHandle<()>>>,
    handlers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
}

impl core::fmt::Debug for InteractionListenerSet {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InteractionListenerSet")
            .field("listener_count", &self.paths.len())
            .field("stopping", &self.stop.load(Ordering::Acquire))
            .finish()
    }
}

impl InteractionListenerSet {
    /// Return the socket paths owned by this daemon listener set.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Borrow the shared daemon shutdown token.
    pub fn stop_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }

    /// Append another independently Zone-bound listener set.
    pub fn extend(&mut self, mut other: Self) {
        self.paths.append(&mut other.paths);
        self.socket_identities.append(&mut other.socket_identities);
        self.parent_identities.append(&mut other.parent_identities);
        let other_threads = std::mem::replace(&mut other.threads, Mutex::new(Vec::new()))
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.threads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(other_threads);
        self.handlers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .append(
                &mut other
                    .handlers
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            );
    }

    /// Stop accepting new sessions and join all listener loops.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        let mut threads = self
            .threads
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for thread in threads.drain(..) {
            let _ = thread.join();
        }
        let mut handlers = self
            .handlers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for handler in handlers.drain(..) {
            let _ = handler.join();
        }
        self.remove_socket_paths();
    }

    fn remove_socket_paths(&self) {
        for ((path, (device, inode)), (parent_device, parent_inode)) in self
            .paths
            .iter()
            .zip(&self.socket_identities)
            .zip(&self.parent_identities)
        {
            let parent_owned = path.parent().and_then(|parent| {
                std::fs::symlink_metadata(parent).ok().filter(|metadata| {
                    metadata.is_dir()
                        && !metadata.file_type().is_symlink()
                        && metadata.dev() == *parent_device
                        && metadata.ino() == *parent_inode
                })
            });
            if parent_owned.is_some()
                && std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                    metadata.file_type().is_socket()
                        && metadata.dev() == *device
                        && metadata.ino() == *inode
                })
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

impl Drop for InteractionListenerSet {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let threads = self
            .threads
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for thread in threads.drain(..) {
            let _ = thread.join();
        }
        let mut handlers = self
            .handlers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for handler in handlers.drain(..) {
            let _ = handler.join();
        }
        for ((path, (device, inode)), (parent_device, parent_inode)) in self
            .paths
            .iter()
            .zip(&self.socket_identities)
            .zip(&self.parent_identities)
        {
            let parent_owned = path.parent().and_then(|parent| {
                std::fs::symlink_metadata(parent).ok().filter(|metadata| {
                    metadata.is_dir()
                        && !metadata.file_type().is_symlink()
                        && metadata.dev() == *parent_device
                        && metadata.ino() == *parent_inode
                })
            });
            if parent_owned.is_some()
                && std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                    metadata.file_type().is_socket()
                        && metadata.dev() == *device
                        && metadata.ino() == *inode
                })
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn bind_interaction_listener(path: &std::path::Path, expected_uid: u32) -> std::io::Result<Socket> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() && metadata.uid() == expected_uid => {
            std::fs::remove_file(path)?
        }
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "interaction listener path is not a socket",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = Socket::new(Domain::UNIX, Type::from(libc::SOCK_SEQPACKET), None)?;
    listener.set_nonblocking(true)?;
    listener.bind(&SockAddr::unix(path)?)?;
    listener.listen(32)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
    Ok(listener)
}

#[derive(Clone)]
struct InteractionAcceptContext<S, G>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    runtime: Arc<AsyncMutex<Option<InteractionRuntimeSet<S, G>>>>,
    zone: ZoneId,
    service: String,
    expected_peer_uid: u32,
    stop: Arc<AtomicBool>,
    handlers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    active_handlers: Arc<AtomicUsize>,
}

fn interaction_accept_loop<S, G>(listener: Socket, context: InteractionAcceptContext<S, G>)
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    let InteractionAcceptContext {
        runtime,
        zone,
        service,
        expected_peer_uid,
        stop,
        handlers,
        active_handlers,
    } = context;
    while !stop.load(Ordering::Acquire) {
        reap_finished_handlers(&handlers);
        let socket = match accept_with(
            listener.as_fd(),
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        ) {
            Ok(accepted) => accepted,
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::AGAIN) => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(error) => {
                tracing::warn!(%error, service = %service, "interaction listener accept failed");
                continue;
            }
        };
        let runtime = Arc::clone(&runtime);
        let zone = zone.clone();
        let service = service.clone();
        if !reserve_interaction_handler(&active_handlers) {
            continue;
        }
        let handler_active = Arc::clone(&active_handlers);
        let handler_stop = Arc::clone(&stop);
        let handler = thread::Builder::new()
            .name("d2bd-interaction-session".to_owned())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())
                    .and_then(|runtime_handle| {
                        runtime_handle.block_on(admit_interaction_socket(
                            socket,
                            runtime,
                            zone,
                            service,
                            expected_peer_uid,
                            handler_stop,
                        ))
                    });
                if let Err(error) = result {
                    tracing::debug!(%error, "interaction ComponentSession refused");
                }
                handler_active.fetch_sub(1, Ordering::AcqRel);
            });
        if let Ok(handler) = handler {
            handlers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(handler);
        } else {
            active_handlers.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

const MAX_INTERACTION_HANDLERS: usize = 64;

fn reserve_interaction_handler(active_handlers: &AtomicUsize) -> bool {
    let mut active = active_handlers.load(Ordering::Acquire);
    loop {
        if active >= MAX_INTERACTION_HANDLERS {
            return false;
        }
        match active_handlers.compare_exchange_weak(
            active,
            active + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(current) => active = current,
        }
    }
}

fn reap_finished_handlers(handlers: &Mutex<Vec<thread::JoinHandle<()>>>) {
    let mut handlers = handlers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut index = 0;
    while index < handlers.len() {
        if handlers[index].is_finished() {
            let _ = handlers.swap_remove(index).join();
        } else {
            index += 1;
        }
    }
}

async fn admit_interaction_socket<S, G>(
    socket: std::os::fd::OwnedFd,
    runtime: Arc<AsyncMutex<Option<InteractionRuntimeSet<S, G>>>>,
    zone: ZoneId,
    service: String,
    expected_peer_uid: u32,
    stop: Arc<AtomicBool>,
) -> Result<(), String>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    let policy = interaction_endpoint_policy(&service, 1)
        .ok_or_else(|| "unknown interaction service".to_owned())?;
    let seqpacket = SeqpacketSocket::from_owned(socket).map_err(|error| error.to_string())?;
    let verified_peer =
        VerifiedUnixPeer::verify_seqpacket(&seqpacket).map_err(|error| error.to_string())?;
    if verified_peer.credentials().uid().as_raw() != expected_peer_uid {
        return Err("interaction-peer-uid-rejected".to_owned());
    }
    let expected_peer = verified_peer.credentials();
    let credits = CreditScopeSet::new(
        CreditPool::new(8).map_err(|error| format!("{error:?}"))?,
        CreditPool::new(8).map_err(|error| format!("{error:?}"))?,
        CreditPool::new(8).map_err(|error| format!("{error:?}"))?,
        CreditPool::new(8).map_err(|error| format!("{error:?}"))?,
        CreditPool::new(8).map_err(|error| format!("{error:?}"))?,
        CreditPool::new(8).map_err(|error| format!("{error:?}"))?,
    );
    let resolver: d2b_session_unix::DescriptorPolicyResolver = Arc::new(|descriptor| {
        let clipboard_service = matches!(
            descriptor.service,
            ServicePackage::ClipboardV3
                | ServicePackage::ClipboardBridgeV3
                | ServicePackage::ClipboardPickerCoordV3
        );
        if clipboard_service
            && descriptor.kind == AttachmentKind::FileDescriptor
            && descriptor.purpose == AttachmentPurpose::ClipboardTransfer
        {
            Ok(d2b_session_unix::DescriptorPolicy::ProviderValidatedFile)
        } else {
            Err(UnixSessionError::DescriptorMismatch)
        }
    });
    let transport = UnixSeqpacketTransport::new(
        seqpacket,
        TransportLocality::HostLocal,
        policy.limits,
        policy.attachment_policy,
        credits,
        resolver,
        PeerIdentityPolicy::accepted(expected_peer),
    )
    .map_err(|error| error.to_string())?;
    let engine = tokio::time::timeout(
        Duration::from_secs(5),
        SessionEngine::establish_responder(
            transport,
            policy.clone(),
            d2b_session::HandshakeCredentials::Nn,
            Instant::now(),
        ),
    )
    .await
    .map_err(|_| "interaction-handshake-timeout".to_owned())?
    .map_err(|error| error.to_string())?;
    let acceptor = {
        let guard = runtime.lock().await;
        let composition = guard
            .as_ref()
            .and_then(|set| set.runtime_for(&zone))
            .ok_or_else(|| "interaction runtime unavailable".to_owned())?;
        composition
            .registrar()
            .component_session_acceptor(policy, verified_peer)
            .map_err(|error| error.to_string())?
    };
    let evidence = TransportEvidence::new(
        EvidenceClass::UnixPeer,
        binding_digest(
            &interaction_endpoint_policy(&service, 1)
                .expect("service policy was already validated"),
        ),
    );
    let request_receiver = {
        let mut guard = runtime.lock().await;
        let composition = guard
            .as_mut()
            .and_then(|set| set.runtime_for_mut(&zone))
            .ok_or_else(|| "interaction runtime unavailable".to_owned())?;
        let registered = composition
            .admit_and_register(acceptor, engine, evidence, 1)
            .await
            .map_err(|error| error.to_string())?;
        let session_key = registered.session_key();
        (session_key, registered.request_receiver())
    };
    let (session_key, request_receiver) = request_receiver;
    loop {
        let frame = tokio::select! {
            frame = request_receiver.recv() => match frame {
                Ok(frame) => frame,
                Err(_) => break,
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                continue;
            }
        };
        let mut guard = runtime.lock().await;
        let composition = guard
            .as_mut()
            .and_then(|set| set.runtime_for_mut(&zone))
            .ok_or_else(|| "interaction runtime unavailable".to_owned())?;
        let attachments = if request_accepts_clipboard_attachments(&frame) {
            request_receiver
                .recv_attachments()
                .await
                .map_err(|_| "interaction-attachment-receive-failed".to_owned())?
        } else {
            Vec::new()
        };
        if let Err(error) = composition
            .dispatch_component_request_for_session(&session_key, frame, attachments)
            .await
        {
            tracing::debug!(%error, service = %service, "interaction request rejected");
        }
        if !composition.has_session(&session_key) {
            break;
        }
    }
    let mut guard = runtime.lock().await;
    guard
        .as_mut()
        .ok_or_else(|| "interaction runtime unavailable".to_owned())?
        .remove_session(&zone, &session_key)
        .await?;
    Ok(())
}

fn request_accepts_clipboard_attachments(frame: &[u8]) -> bool {
    let Some(payload) = frame.get(ttrpc::proto::MESSAGE_HEADER_LENGTH..) else {
        return false;
    };
    let Ok(request) = TtrpcRequest::parse_from_bytes(payload) else {
        return false;
    };
    if !matches!(
        request.method.as_str(),
        "ClipboardBridgeService/CaptureGuest" | "ClipboardBridgeService/CaptureHost"
    ) {
        return false;
    }
    serde_json::from_slice::<ClipboardCaptureRequest>(&request.payload)
        .is_ok_and(|request| request.bytes.is_none())
}

fn configuration_digest(
    binding: &DisplayLaunchBinding,
    label: &[u8],
) -> d2b_process::ConfigurationDigest {
    let mut digest = Sha256::new();
    digest.update(b"d2bd-display-config-v1");
    digest.update(label);
    digest.update(binding.attachment_digest());
    digest.update(binding.policy_digest());
    digest.update(binding.policy_generation().to_be_bytes());
    digest.update(binding.teardown_generation().to_be_bytes());
    d2b_process::ConfigurationDigest::from_bytes(digest.finalize().into())
}

fn resource_uid(binding: &DisplayLaunchBinding, label: &[u8]) -> ResourceUid {
    let mut digest = Sha256::new();
    digest.update(b"d2bd-display-resource-v1");
    digest.update(label);
    digest.update(binding.attachment_digest());
    digest.update(binding.policy_generation().to_be_bytes());
    let mut bytes: [u8; 16] = digest.finalize()[..16]
        .try_into()
        .expect("fixed digest length");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let value = format!(
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
    ResourceUid::parse(value).expect("uuid bytes are canonical")
}

fn run_effect<T, F, Fut>(operation: F) -> Result<T, WorkerEffectError>
where
    T: Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, WorkerEffectError>> + Send + 'static,
{
    thread::Builder::new()
        .name("d2bd-provider-effect".to_owned())
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| WorkerEffectError::WorkerUnavailable)?
                .block_on(operation())
        })
        .map_err(|_| WorkerEffectError::WorkerUnavailable)?
        .join()
        .map_err(|_| WorkerEffectError::WorkerUnavailable)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::component_session::RequestId;
    use d2b_process::{
        BackendLaunch, BackendObservation, ObservedIdentity, ProcessEffectBackend,
        ProcessEffectError, ProcessRequest, ProcessStopClass, WaitReapOwner,
    };
    use d2b_resource_api::authz::{
        ApiCatalog, BindingScope, BoundSubject, CompiledRole, CompiledRoleBinding,
        NativeAuthorizer, PolicyRule, PolicySet, SessionVerb,
    };
    use d2b_resource_store::PolicySnapshot;
    use d2b_session::ComponentSessionDriver;
    use d2b_session_unix::DescriptorPolicyResolver;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, Default)]
    struct Backend {
        launches: std::sync::Arc<AtomicUsize>,
        observes: std::sync::Arc<AtomicUsize>,
        stops: std::sync::Arc<AtomicUsize>,
    }

    impl ProcessEffectBackend for Backend {
        type Handle = ();

        fn launch(
            &self,
            _request: ProcessRequest,
        ) -> Result<BackendLaunch<Self::Handle>, ProcessEffectError> {
            let seed = self.launches.fetch_add(1, Ordering::AcqRel) as u8 + 1;
            Ok(BackendLaunch::new(
                BackendObservation::new(
                    ProcessIdentityDigest::from_bytes([seed; 32]),
                    ObservedIdentity::from_verified([IdentityBinding::Cgroup]),
                    WaitReapOwner::Local,
                ),
                (),
            ))
        }

        fn observe(
            &self,
            _request: ProcessRequest,
        ) -> Result<Option<BackendObservation>, ProcessEffectError> {
            self.observes.fetch_add(1, Ordering::AcqRel);
            Ok(None)
        }

        fn open_pidfd(
            &self,
            _observation: BackendObservation,
        ) -> Result<Self::Handle, ProcessEffectError> {
            Ok(())
        }

        fn stop(
            &self,
            _handle: &Self::Handle,
            _class: ProcessStopClass,
        ) -> Result<(), ProcessEffectError> {
            self.stops.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct TestGuestFrontend;

    type TestInteractionRuntime = Arc<
        AsyncMutex<Option<InteractionRuntimeSet<ProviderSupervisor<Backend>, TestGuestFrontend>>>,
    >;

    impl GuestFrontendEffectPort for TestGuestFrontend {
        fn ensure(
            &mut self,
            _guest: &ResourceRef,
            policy_generation: u64,
            teardown_generation: u64,
            session_digest: [u8; 32],
        ) -> Result<WorkerLaunchReceipt, WorkerEffectError> {
            Ok(WorkerLaunchReceipt::from_supervisor(
                DisplayProcessRole::GuestFrontend,
                WorkerState::Ready { generation: 1 },
                policy_generation,
                teardown_generation,
                session_digest,
            ))
        }

        fn stop(
            &mut self,
            _guest: &ResourceRef,
            policy_generation: u64,
            teardown_generation: u64,
            session_digest: [u8; 32],
        ) -> Result<WorkerLaunchReceipt, WorkerEffectError> {
            Ok(WorkerLaunchReceipt::from_supervisor(
                DisplayProcessRole::GuestFrontend,
                WorkerState::Terminal { deleted: true },
                policy_generation,
                teardown_generation,
                session_digest,
            ))
        }
    }

    #[test]
    fn neutral_ticket_keeps_fd_metadata_and_supervises_exact_worker() {
        let binding = DisplayLaunchBinding::from_ticket(
            d2b_provider_display_wayland::LaunchTicket::new_for_daemon(
                DisplayProcessRole::HostProxy,
                Some(d2b_provider_display_wayland::AttachmentGrantHandle::from_daemon([1; 32])),
                d2b_provider_display_wayland::AttachmentGrantHandle::from_daemon([2; 32]),
                "sha256:".to_owned() + &"a".repeat(64),
                1,
                "session",
                1,
            )
            .unwrap(),
        );
        let ticket = process_ticket(&binding).unwrap();
        assert_eq!(ticket.inherited_fd_table().count(), 2);
        let _ = ticket.digests().fd_table;
    }

    #[test]
    fn display_effect_observes_then_launches_and_stops_through_supervisor() {
        let backend = Backend::default();
        let supervisor = d2b_provider_supervisor::ProviderSupervisor::new(backend.clone());
        let mut effects = DisplaySupervisorEffects::new(supervisor);
        let ticket = d2b_provider_display_wayland::LaunchTicket::new_for_daemon(
            DisplayProcessRole::HostProxy,
            Some(d2b_provider_display_wayland::AttachmentGrantHandle::from_daemon([3; 32])),
            d2b_provider_display_wayland::AttachmentGrantHandle::from_daemon([4; 32]),
            "sha256:".to_owned() + &"b".repeat(64),
            1,
            "session",
            1,
        )
        .unwrap();
        effects.policy_generation = 1;
        effects.teardown_generation = 1;
        effects.session_digest = [8; 32];
        effects.launch(ticket).unwrap();
        effects.stop(DisplayProcessRole::HostProxy).unwrap();
        assert_eq!(backend.observes.load(Ordering::Acquire), 1);
        assert_eq!(backend.launches.load(Ordering::Acquire), 1);
        assert_eq!(backend.stops.load(Ordering::Acquire), 1);
    }

    #[test]
    fn display_supervision_refresh_uses_live_worker_observation() {
        let backend = Backend::default();
        let supervisor = d2b_provider_supervisor::ProviderSupervisor::new(backend.clone());
        let mut effects = DisplaySupervisorEffects::new(supervisor);
        effects.policy_generation = 1;
        effects.teardown_generation = 1;
        effects.session_digest = [10; 32];
        let ticket = d2b_provider_display_wayland::LaunchTicket::new_for_daemon(
            DisplayProcessRole::HostProxy,
            Some(d2b_provider_display_wayland::AttachmentGrantHandle::from_daemon([5; 32])),
            d2b_provider_display_wayland::AttachmentGrantHandle::from_daemon([6; 32]),
            "sha256:".to_owned() + &"c".repeat(64),
            1,
            "session",
            1,
        )
        .unwrap();
        effects.launch(ticket).unwrap();
        let observation_before_refresh = daemon_monotonic_ms();
        let evidence = effects.current_supervision();
        assert!(evidence.observed_at_ms() >= observation_before_refresh);
        assert!(evidence.proxy_last_failure_ms().is_some());
        assert!(backend.observes.load(Ordering::Acquire) >= 2);
    }

    #[test]
    fn display_stop_is_idempotent_when_worker_was_already_adopted_elsewhere() {
        let supervisor = d2b_provider_supervisor::ProviderSupervisor::new(Backend::default());
        let mut effects = DisplaySupervisorEffects::new(supervisor);
        effects.policy_generation = 1;
        effects.teardown_generation = 1;
        effects.session_digest = [9; 32];

        let receipt = effects.stop(DisplayProcessRole::GuestFrontend).unwrap();

        assert_eq!(receipt.state(), WorkerState::Terminal { deleted: true });
        assert_eq!(effects.live_worker_count(), 0);
    }

    #[test]
    fn persisted_observation_records_round_trip_without_raw_launch_authority() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("observations.json");
        let record = PersistedObservation {
            process_uid: "11111111-1111-4111-8111-111111111111".to_owned(),
            vm_id: "guest-a".to_owned(),
            execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
            domain: ExecutionDomain::System,
            user_ref: None,
            role_id: "wayland-proxy".to_owned(),
            role: RunnerRole::WaylandProxy,
            bundle_runner_intent_ref: "runner:guest-a:wayland-proxy".to_owned(),
            provider_identity: [1; 32],
            template_identity: [2; 32],
            generation: 3,
            resource_ref: ResourceRef::parse("EphemeralProcess/display-host-proxy").unwrap(),
            resource_uid: ResourceUid::parse("22222222-2222-4222-8222-222222222222").unwrap(),
            bundle_content_identity: "sha256:test".to_owned(),
            sandbox_plan: None,
            pid: 42,
            start_time_ticks: 99,
            cgroup_verified: true,
            executable_verified: true,
        };
        std::fs::write(&path, serde_json::to_vec(&[record]).unwrap()).unwrap();

        let observations = load_observations(&path).unwrap();
        let observed = observations
            .get("11111111-1111-4111-8111-111111111111")
            .unwrap();
        assert_eq!(observed.pid, 42);
        assert_eq!(observed.start_time_ticks, 99);
        assert_eq!(observed.intent.role, RunnerRole::WaylandProxy);
        assert_eq!(
            observed.intent.execution_ref.to_canonical_string(),
            "Host/host-system"
        );
        assert_eq!(
            observed.intent.resource_ref.to_canonical_string(),
            "EphemeralProcess/display-host-proxy"
        );
    }

    #[test]
    fn display_sandbox_plan_is_derived_from_trusted_runner_intent() {
        let intent = d2b_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_capabilities(vec!["CAP_NET_RAW".to_owned()])
            .with_seccomp_policy_ref(Some("strict"))
            .build();

        let plan = display_sandbox_plan(&intent, ExecutionDomain::System).unwrap();

        assert_eq!(plan.domain, ExecutionDomain::System);
        assert_eq!(plan.capability_classes, vec![CapabilityClass::NetworkRaw]);
        assert_eq!(plan.seccomp_class.as_str(), "strict");
        assert!(plan.no_new_privileges);
        assert!(plan.read_only_root);
    }

    #[test]
    fn interaction_response_is_a_correlated_ttrpc_response() {
        let frame = encode_interaction_response(41, TtrpcCode::OK, b"{\"ok\":true}".to_vec())
            .expect("response encoding");
        assert!(d2b_session::ttrpc_is_response(&frame));
        assert_eq!(d2b_session::ttrpc_stream_id(&frame).unwrap(), 41);
        let header = MessageHeader::from(&frame[..ttrpc::proto::MESSAGE_HEADER_LENGTH]);
        let response =
            TtrpcResponse::parse_from_bytes(&frame[ttrpc::proto::MESSAGE_HEADER_LENGTH..])
                .expect("response protobuf");
        assert_eq!(header.stream_id, 41);
        assert_eq!(response.status().code(), TtrpcCode::OK);
        assert_eq!(response.payload, b"{\"ok\":true}");
    }

    #[test]
    fn interaction_listener_policy_binds_service_and_transport() {
        let policy = interaction_endpoint_policy(d2b_provider_display_wayland::SERVICE_PACKAGE, 7)
            .expect("display policy");
        assert_eq!(policy.service, ServicePackage::DisplayV3,);
        assert_eq!(policy.reconnect_generation, 7);
        assert_eq!(
            policy.transport_binding.transport,
            TransportClass::UnixSeqpacket,
        );
    }

    #[test]
    fn listener_stop_removes_daemon_owned_socket_paths() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("interaction.sock");
        let listener = bind_interaction_listener(&path, rustix::process::geteuid().as_raw())
            .expect("listener socket");
        drop(listener);
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        let listeners = InteractionListenerSet {
            paths: vec![path.clone()],
            socket_identities: vec![(metadata.dev(), metadata.ino())],
            parent_identities: {
                let parent = std::fs::symlink_metadata(directory.path()).unwrap();
                vec![(parent.dev(), parent.ino())]
            },
            stop: Arc::new(AtomicBool::new(false)),
            threads: Mutex::new(Vec::new()),
            handlers: Arc::new(Mutex::new(Vec::new())),
        };

        assert!(path.exists());
        listeners.stop();
        assert!(!path.exists());
    }

    fn test_interaction_composition(
        zone: &ZoneId,
        uid: u32,
    ) -> InteractionComposition<ProviderSupervisor<Backend>, TestGuestFrontend> {
        let catalog = ApiCatalog::standard();
        let subject_ref =
            ResourceRef::parse(&format!("Guest/uid-{uid}")).expect("guest subject reference");
        let role = CompiledRole::new(
            ResourceRef::parse("Role/interaction-provider").expect("role reference"),
            vec![
                PolicyRule::new(
                    &catalog,
                    [],
                    [],
                    [
                        SessionVerb::Connect,
                        SessionVerb::Invoke,
                        SessionVerb::OpenStream,
                        SessionVerb::Cancel,
                        SessionVerb::Observe,
                        SessionVerb::AuditExport,
                        SessionVerb::SupportBundle,
                    ],
                    [],
                    [],
                    [zone.clone()],
                    [],
                )
                .expect("interaction policy rule"),
            ],
        )
        .expect("interaction role");
        let binding = CompiledRoleBinding::new(
            role.role_ref.clone(),
            [BoundSubject {
                subject_ref,
                subject_uid: unix_guest_subject_uid(uid),
            }],
            BindingScope::default(),
            d2b_resource_api::authz::RelayGrantAuthority::None,
        )
        .expect("interaction role binding");
        let policy =
            PolicySet::new(&catalog, 1, vec![role], vec![binding]).expect("interaction policy");
        let authorizer =
            BusAuthorizer::new(
                NativeAuthorizer::new(catalog, Some(policy)).unwrap(),
                d2b_resource_api::authz::AuthorizationState {
                    snapshot: PolicySnapshot {
                        policy_revision: 1,
                        api_catalog_revision: 1,
                        active_configuration_revision:
                            d2b_contracts::v3::ConfigurationGeneration::new(1).unwrap(),
                        controller_generation: Some(
                            d2b_contracts::v3::ControllerGeneration::new(1).unwrap(),
                        ),
                    },
                    zone_policy_revision: ZoneRevision::new(1),
                    bootstrap_phase: BootstrapPhase::Disabled,
                    now_tick: 1,
                },
            )
            .unwrap();
        let (_bus, registrar) = ZoneBus::with_clock_observer_and_metrics(
            zone.clone(),
            authorizer,
            BusConfig::default(),
            Arc::new(d2b_bus::ManualClock::new(1)),
            Arc::new(NoopBusObserver),
            Arc::new(d2b_bus::metrics::NoopBusTelemetry),
        )
        .unwrap();
        let mut composition = InteractionComposition::new_with_guest_frontend(
            registrar,
            ProviderSupervisor::new(Backend::default()),
            TestGuestFrontend,
        );
        composition.bind_display_resource_evidence(
            CoreDisplayResourceEvidence::from_committed_policy(
                ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/display-wayland")
                    .unwrap(),
                PolicySnapshot {
                    policy_revision: 1,
                    api_catalog_revision: 1,
                    active_configuration_revision: d2b_contracts::v3::ConfigurationGeneration::new(
                        1,
                    )
                    .unwrap(),
                    controller_generation: Some(
                        d2b_contracts::v3::ControllerGeneration::new(1).unwrap(),
                    ),
                },
                1,
                FilterInput::default(),
                FilterInput::default(),
                DependencyState::ready().with_zone(zone.clone()),
                ResourceRef::parse(&format!("User/uid-{uid}")).unwrap(),
                ZoneRevision::new(1),
                true,
            )
            .unwrap(),
        );
        composition
    }

    fn test_interaction_runtime(
        zone: &ZoneId,
        uid: u32,
    ) -> InteractionRuntimeSet<ProviderSupervisor<Backend>, TestGuestFrontend> {
        let mut runtimes = InteractionRuntimeSet::new();
        runtimes.insert(zone.clone(), test_interaction_composition(zone, uid));
        runtimes
    }

    fn test_unix_transport(
        socket: SeqpacketSocket,
        peer: d2b_session_unix::PeerCredentials,
        policy: &EndpointPolicy,
    ) -> UnixSeqpacketTransport {
        let credits = CreditScopeSet::new(
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
            CreditPool::new(8).unwrap(),
        );
        let resolver: DescriptorPolicyResolver =
            Arc::new(|_| Err(UnixSessionError::DescriptorMismatch));
        UnixSeqpacketTransport::new(
            socket,
            TransportLocality::HostLocal,
            policy.limits,
            policy.attachment_policy,
            credits,
            resolver,
            PeerIdentityPolicy::accepted(peer),
        )
        .unwrap()
    }

    fn request_frame_for_test(
        service: &str,
        stream_id: u32,
        method: &str,
        request_payload: Vec<u8>,
    ) -> Vec<u8> {
        let request = TtrpcRequest {
            service: service.to_owned(),
            method: method.to_owned(),
            payload: request_payload,
            ..TtrpcRequest::default()
        };
        let payload = request.write_to_bytes().unwrap();
        let mut frame = Vec::from(MessageHeader::new_request(
            stream_id,
            u32::try_from(payload.len()).unwrap(),
        ));
        frame.extend_from_slice(&payload);
        frame
    }

    async fn establish_test_client(
        listener: &Socket,
        runtime: &TestInteractionRuntime,
        zone: &ZoneId,
        service: &str,
        uid: u32,
        path: &std::path::Path,
    ) -> (
        Arc<dyn ComponentSessionDriver>,
        tokio::task::JoinHandle<Result<(), String>>,
    ) {
        let client_socket =
            Socket::new(Domain::UNIX, Type::from(libc::SOCK_SEQPACKET), None).unwrap();
        client_socket
            .connect(&SockAddr::unix(path).unwrap())
            .expect("connect interaction listener");
        client_socket.set_nonblocking(true).unwrap();
        let accepted = loop {
            match accept_with(
                listener.as_fd(),
                SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            ) {
                Ok(accepted) => break accepted,
                Err(rustix::io::Errno::AGAIN) => thread::yield_now(),
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        let server_runtime = Arc::clone(runtime);
        let server_zone = zone.clone();
        let server_service = service.to_owned();
        let server = tokio::spawn(async move {
            admit_interaction_socket(
                accepted,
                server_runtime,
                server_zone,
                server_service,
                uid,
                Arc::new(AtomicBool::new(false)),
            )
            .await
        });

        let policy = interaction_endpoint_policy(service, 1).unwrap();
        let client_seqpacket = SeqpacketSocket::from_owned(client_socket.into()).unwrap();
        let client_peer = client_seqpacket.acceptor_peer_credentials().unwrap();
        let transport = test_unix_transport(client_seqpacket, client_peer, &policy);
        let engine = tokio::time::timeout(
            Duration::from_secs(5),
            SessionEngine::establish_initiator(
                transport,
                policy,
                d2b_session::HandshakeCredentials::Nn,
                Instant::now(),
            ),
        )
        .await
        .expect("client handshake timeout")
        .unwrap_or_else(|error| panic!("client handshake failed for {service}: {error}"));
        (Arc::new(engine.into_driver()), server)
    }

    async fn dispatch_test_request(
        driver: &Arc<dyn ComponentSessionDriver>,
        service: &str,
        stream_id: u32,
        method: &str,
        request_payload: Vec<u8>,
    ) -> TtrpcResponse {
        let frame = request_frame_for_test(service, stream_id, method, request_payload);
        let request_id = d2b_session::ttrpc_request_id(1, &frame).unwrap();
        driver.start_ttrpc(request_id.clone(), frame).await.unwrap();
        let response = tokio::time::timeout(Duration::from_secs(5), driver.receive_ttrpc())
            .await
            .expect("interaction response timeout")
            .unwrap();
        assert!(driver.complete_ttrpc(request_id).await.unwrap());
        TtrpcResponse::parse_from_bytes(&response[ttrpc::proto::MESSAGE_HEADER_LENGTH..]).unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hermetic_listener_dispatches_clipboard_notification_and_ordered_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let zone = ZoneId::parse("work").unwrap();
        let uid = nix::unistd::getuid().as_raw();
        let runtime = Arc::new(AsyncMutex::new(Some(test_interaction_runtime(&zone, uid))));
        let services = [
            d2b_provider_display_wayland::SERVICE_PACKAGE,
            d2b_provider_clipboard_wayland::BRIDGE_SERVICE,
            d2b_provider_clipboard_wayland::PICKER_SERVICE,
            d2b_provider_notification_desktop::SERVICE_PACKAGE,
        ];
        let mut clients = Vec::new();
        for service in services {
            let path = directory
                .path()
                .join(service.replace('.', "-"))
                .with_extension("sock");
            let listener = bind_interaction_listener(&path, uid).unwrap();
            let (client, server) =
                establish_test_client(&listener, &runtime, &zone, service, uid, &path).await;
            clients.push((service, path, listener, client, server));
        }

        let (display_service, _, _, display, _) = &clients[0];
        let display_spec = WaylandSessionSpec::new(
            ResourceRef::parse(&format!("Guest/uid-{uid}")).unwrap(),
            ResourceRef::parse("Host/host-system").unwrap(),
            ResourceRef::parse(&format!("User/uid-{uid}")).unwrap(),
            ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/display-wayland").unwrap(),
            d2b_provider_display_wayland::DisplayIdentity::new(
                "test-display",
                "#112233",
                "#223344",
                "#334455",
            )
            .unwrap(),
            true,
        )
        .unwrap();
        let display_payload = serde_json::to_vec(&serde_json::json!({
            "spec": display_spec,
        }))
        .unwrap();
        let reconcile = dispatch_test_request(
            display,
            display_service,
            100,
            "DisplayService/Reconcile",
            display_payload,
        )
        .await;
        assert_eq!(reconcile.status().code(), TtrpcCode::OK);
        let observe = dispatch_test_request(
            display,
            display_service,
            101,
            "DisplayService/Observe",
            Vec::new(),
        )
        .await;
        assert_eq!(observe.status().code(), TtrpcCode::OK);
        assert!(
            String::from_utf8(observe.payload)
                .unwrap()
                .contains("\"ready\":true")
        );

        let (bridge_service, _, _, bridge, _) = &clients[1];
        let capture = dispatch_test_request(
            bridge,
            bridge_service,
            102,
            "ClipboardBridgeService/CaptureGuest",
            serde_json::to_vec(&serde_json::json!({
                "mime": "text/plain",
                "bytes": [99, 108, 105, 112],
                "now_secs": 100,
            }))
            .unwrap(),
        )
        .await;
        assert_eq!(capture.status().code(), TtrpcCode::OK);
        let capture_payload: serde_json::Value = serde_json::from_slice(&capture.payload).unwrap();
        let entry_digest = capture_payload["entry_digest"].as_str().unwrap().to_owned();

        let (picker_service, _, _, picker, _) = &clients[2];
        let completion_payload = serde_json::to_vec(&serde_json::json!({
            "entry_digest": entry_digest,
            "mime_types": ["text/plain"],
            "selected_digest": entry_digest,
            "now_secs": 101,
        }))
        .unwrap();
        let completion = dispatch_test_request(
            picker,
            picker_service,
            103,
            "ClipboardPickerService/Complete",
            completion_payload.clone(),
        )
        .await;
        assert_eq!(completion.status().code(), TtrpcCode::OK);
        let completion_value: serde_json::Value =
            serde_json::from_slice(&completion.payload).unwrap();
        let operation_id = completion_value["operation_id"].as_str().unwrap();
        let materialized = dispatch_test_request(
            picker,
            picker_service,
            104,
            "ClipboardPickerService/Materialize",
            serde_json::to_vec(&serde_json::json!({
                "operation_id": operation_id,
                "entry_digest": entry_digest,
            }))
            .unwrap(),
        )
        .await;
        assert_eq!(materialized.status().code(), TtrpcCode::OK);
        let materialized_value: serde_json::Value =
            serde_json::from_slice(&materialized.payload).unwrap();
        assert_eq!(
            materialized_value["bytes"],
            serde_json::json!([99, 108, 105, 112])
        );
        let replay = dispatch_test_request(
            picker,
            picker_service,
            105,
            "ClipboardPickerService/Complete",
            completion_payload,
        )
        .await;
        assert_eq!(replay.status().code(), TtrpcCode::FAILED_PRECONDITION);

        let (notification_service, _, _, notification, _) = &clients[3];
        let deliver = dispatch_test_request(
            notification,
            notification_service,
            106,
            "NotificationService/Deliver",
            serde_json::to_vec(&serde_json::json!({
                "request": {
                    "summary": "Update",
                    "body": "A bounded body",
                    "category": "system.info",
                    "actions": [{"id": "open", "label": "Open"}],
                    "idempotencyKey": "notification-1",
                },
                "now_secs": 101,
            }))
            .unwrap(),
        )
        .await;
        assert_eq!(deliver.status().code(), TtrpcCode::OK);
        let deliver_payload: serde_json::Value = serde_json::from_slice(&deliver.payload).unwrap();
        assert_eq!(deliver_payload["accepted"], true);
        assert_eq!(deliver_payload["action_count"], 1);
        assert!(deliver_payload.get("action_nonces").is_none());

        let action = dispatch_test_request(
            notification,
            notification_service,
            107,
            "NotificationService/InvokeAction",
            serde_json::to_vec(&serde_json::json!({
                "action_key": "guest-provided",
                "now_secs": 102,
            }))
            .unwrap(),
        )
        .await;
        assert_eq!(action.status().code(), TtrpcCode::UNIMPLEMENTED);
        let close = dispatch_test_request(
            notification,
            notification_service,
            108,
            "NotificationService/CloseObserver",
            Vec::new(),
        )
        .await;
        assert_eq!(close.status().code(), TtrpcCode::UNIMPLEMENTED);

        let finalize = dispatch_test_request(
            display,
            display_service,
            109,
            "DisplayService/Finalize",
            Vec::new(),
        )
        .await;
        assert_eq!(finalize.status().code(), TtrpcCode::OK);
        for (_, _, _, _, server) in clients {
            assert!(server.await.unwrap().is_ok());
        }
        assert_eq!(
            runtime
                .lock()
                .await
                .as_ref()
                .and_then(|set| set.runtime_for(&zone))
                .map_or(0, InteractionComposition::session_count),
            0
        );
    }

    #[test]
    fn notification_presentation_effect_consumes_sanitized_payload_and_bounds_queue() {
        let request =
            NotificationRequest::new("Update", "A bounded body", Category::SystemInfo).unwrap();
        let sanitized = d2b_provider_notification_desktop::sanitize(&request).unwrap();
        let mut port = InteractionNotificationPort::default();
        port.activate().unwrap();
        for _ in 0..64 {
            port.notify(&sanitized).unwrap();
        }
        assert_eq!(port.presented.len(), 64);
        assert_eq!(port.presented.front().unwrap().summary(), "Update");
        assert_eq!(
            port.notify(&sanitized),
            Err(d2b_provider_notification_desktop::SinkError::Unavailable)
        );
    }

    #[test]
    fn notification_authority_release_refuses_active_effects() {
        let port: Arc<Mutex<Box<dyn DesktopNotificationPort + Send>>> =
            Arc::new(Mutex::new(Box::new(InteractionNotificationPort::default())));
        let mut effects = InteractionDrainEffects::new(port);
        let source = NotificationSourceIdentity::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse("Provider/notification-desktop").unwrap(),
            ResourceRef::parse("Guest/guest").unwrap(),
            1,
            1,
            "sha256:source",
        )
        .unwrap();
        let plan = NotificationLifecyclePlan::new(
            ZoneId::parse("work").unwrap(),
            ResourceRef::parse("Provider/notification-desktop").unwrap(),
            vec![source],
            Vec::new(),
            None,
            None,
        )
        .unwrap();
        effects
            .notification_lifecycle
            .as_ref()
            .unwrap()
            .apply(&plan)
            .unwrap();

        assert_eq!(
            d2b_provider_notification_desktop::NotificationProcessEffectPort::release_authority(
                &mut effects
            ),
            Err("notification-authority-release-incomplete")
        );
        assert!(!effects.authority_released());
    }

    #[test]
    fn listener_handler_reservations_are_bounded() {
        let active_handlers = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = std::sync::mpsc::channel();
        thread::scope(|scope| {
            for _ in 0..MAX_INTERACTION_HANDLERS + 16 {
                let active_handlers = Arc::clone(&active_handlers);
                let sender = sender.clone();
                scope.spawn(move || {
                    sender
                        .send(reserve_interaction_handler(&active_handlers))
                        .unwrap();
                });
            }
        });
        drop(sender);

        assert_eq!(
            receiver.into_iter().filter(|reserved| *reserved).count(),
            MAX_INTERACTION_HANDLERS
        );
        assert_eq!(
            active_handlers.load(Ordering::Acquire),
            MAX_INTERACTION_HANDLERS
        );
    }

    #[test]
    fn completed_listener_handlers_are_reaped() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let handler = thread::spawn(move || sender.send(()).unwrap());
        receiver.recv().unwrap();
        while !handler.is_finished() {
            thread::yield_now();
        }
        let handlers = Mutex::new(vec![handler]);

        reap_finished_handlers(&handlers);

        assert!(handlers.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hermetic_listener_authenticates_dispatches_finalizes_and_refuses_replay() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("interaction.sock");
        let zone = ZoneId::parse("work").unwrap();
        let uid = nix::unistd::getuid().as_raw();
        let listener = bind_interaction_listener(&path, uid).unwrap();
        let runtime = Arc::new(AsyncMutex::new(Some(test_interaction_runtime(&zone, uid))));

        let client_socket =
            Socket::new(Domain::UNIX, Type::from(libc::SOCK_SEQPACKET), None).unwrap();
        client_socket
            .connect(&SockAddr::unix(&path).unwrap())
            .unwrap();
        client_socket.set_nonblocking(true).unwrap();
        let accepted = loop {
            match accept_with(
                listener.as_fd(),
                SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            ) {
                Ok(accepted) => break accepted,
                Err(rustix::io::Errno::AGAIN) => thread::yield_now(),
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        let server_runtime = Arc::clone(&runtime);
        let server_zone = zone.clone();
        let server = tokio::spawn(async move {
            admit_interaction_socket(
                accepted,
                server_runtime,
                server_zone,
                d2b_provider_display_wayland::SERVICE_PACKAGE.to_owned(),
                uid,
                Arc::new(AtomicBool::new(false)),
            )
            .await
        });

        let policy =
            interaction_endpoint_policy(d2b_provider_display_wayland::SERVICE_PACKAGE, 1).unwrap();
        let client_seqpacket = SeqpacketSocket::from_owned(client_socket.into()).unwrap();
        let client_peer = client_seqpacket.acceptor_peer_credentials().unwrap();
        let transport = test_unix_transport(client_seqpacket, client_peer, &policy);
        let engine = tokio::time::timeout(
            Duration::from_secs(5),
            SessionEngine::establish_initiator(
                transport,
                policy,
                d2b_session::HandshakeCredentials::Nn,
                Instant::now(),
            ),
        )
        .await
        .expect("client handshake timeout")
        .unwrap();
        let driver = engine.into_driver();

        let observe_frame = request_frame_for_test(
            d2b_provider_display_wayland::SERVICE_PACKAGE,
            41,
            "DisplayService/Observe",
            Vec::new(),
        );
        let observe_id = d2b_session::ttrpc_request_id(1, &observe_frame).unwrap();
        driver
            .start_ttrpc(observe_id.clone(), observe_frame)
            .await
            .unwrap();
        let observe_response = tokio::time::timeout(Duration::from_secs(5), driver.receive_ttrpc())
            .await
            .expect("observe response timeout")
            .unwrap();
        let observe_payload = &observe_response[ttrpc::proto::MESSAGE_HEADER_LENGTH..];
        let observe = TtrpcResponse::parse_from_bytes(observe_payload).unwrap();
        assert_eq!(observe.status().code(), TtrpcCode::OK);
        assert!(
            String::from_utf8(observe.payload)
                .unwrap()
                .contains("\"ready\":false")
        );
        assert!(driver.complete_ttrpc(observe_id).await.unwrap());

        let finalize_frame = request_frame_for_test(
            d2b_provider_display_wayland::SERVICE_PACKAGE,
            42,
            "DisplayService/Finalize",
            Vec::new(),
        );
        let finalize_id = d2b_session::ttrpc_request_id(1, &finalize_frame).unwrap();
        driver
            .start_ttrpc(finalize_id.clone(), finalize_frame)
            .await
            .unwrap();
        let finalize_response =
            tokio::time::timeout(Duration::from_secs(5), driver.receive_ttrpc())
                .await
                .expect("finalize response timeout")
                .unwrap();
        let finalize_payload = &finalize_response[ttrpc::proto::MESSAGE_HEADER_LENGTH..];
        let finalize = TtrpcResponse::parse_from_bytes(finalize_payload).unwrap();
        assert_eq!(finalize.status().code(), TtrpcCode::OK);
        assert!(server.await.unwrap().is_ok());
        assert_eq!(
            runtime
                .lock()
                .await
                .as_ref()
                .and_then(|set| set.runtime_for(&zone))
                .map_or(0, InteractionComposition::session_count),
            0
        );

        let replay_frame = request_frame_for_test(
            d2b_provider_display_wayland::SERVICE_PACKAGE,
            43,
            "DisplayService/Observe",
            Vec::new(),
        );
        let replay = RequestId::new(vec![0x43; 16]).unwrap();
        assert!(driver.start_ttrpc(replay, replay_frame,).await.is_err());
        drop(listener);
        std::fs::remove_file(&path).unwrap();
        assert!(!path.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unix_listener_observes_real_peer_credentials_before_session_admission() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("interaction.sock");
        let listener = bind_interaction_listener(&path, nix::unistd::getuid().as_raw())
            .expect("listener socket");
        let client = Socket::new(Domain::UNIX, Type::from(libc::SOCK_SEQPACKET), None).unwrap();
        client.connect(&SockAddr::unix(&path).unwrap()).unwrap();
        let accepted = loop {
            match accept_with(
                listener.as_fd(),
                SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            ) {
                Ok(accepted) => break accepted,
                Err(rustix::io::Errno::AGAIN) => thread::yield_now(),
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        let accepted = SeqpacketSocket::from_owned(accepted).unwrap();
        let peer = VerifiedUnixPeer::verify_seqpacket(&accepted).unwrap();
        assert_eq!(
            peer.credentials().uid().as_raw(),
            nix::unistd::getuid().as_raw()
        );
        drop(client);
        drop(accepted);
    }
}
