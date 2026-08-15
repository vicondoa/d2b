//! Daemon-owned composition for authenticated interaction Providers.
//!
//! This is the only layer that may join a sealed ComponentSession admission
//! to process effects.  Provider crates receive authenticated sessions and
//! opaque evidence; they never construct a session, resolve a process, or
//! retain a persistent service unit.

use std::{
    collections::BTreeMap,
    future::Future,
    os::fd::AsFd,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use d2b_bus::{
    BusAuthorizer, BusConfig, BusError, BusIngress, ComponentRequestReceiver,
    ComponentSessionAdmission, NoopBusObserver, OperationId, OperationSpec, RouteGenerations,
    RouteKey, RouteMember, RouteTarget, ZoneBus, ZoneRegistrar,
};
use d2b_contracts::v3::{
    ConfigurationGeneration, ControllerGeneration, EvidenceClass,
    ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
    component_session::{
        AttachmentPolicy, AttachmentPolicyKind, EndpointPolicy, EndpointPurpose, EndpointRole,
        IdentityEvidenceRequirement, LimitProfile, Locality as TransportLocality, NoiseProfile,
        PurposeClass, ServicePackage, TransportBinding, TransportClass,
    },
    ServiceName,
    execution_policy::{BoundedToken, ExecutionDomain},
};
use d2b_contracts::{
    broker_wire::{BrokerCallerRole, RunnerRole},
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
use d2b_provider_clipboard_wayland::{ClipboardProcessEffectPort, ClipboardServiceError};
use d2b_provider_display_wayland::{
    AuthenticatedDisplaySession, CleanupState, DisplayDependencyProof, DisplayLaunchBinding,
    DisplayController, DisplayProcessEffectPort, DisplayProcessRole, DisplayRuntime,
    DisplayRuntimeError, DependencyState, FilterInput, LaunchGrants, VolumeState,
    WaylandPolicySnapshot, WaylandSessionSpec, WorkerEffectError, WorkerLaunchReceipt, WorkerState,
    WorkerRestartEvidence,
};
use d2b_provider_notification_desktop::{
    NotificationProcessEffectPort, SourceProcessEffectPort, SourceProcessEffectReceipt,
    SourceReconcileResult,
};
use d2b_provider_supervisor::{
    BrokerLaunchIntent, BrokerLaunchResolver, BrokerObservedProcess, BrokerProcessBackend,
    ProviderSupervisor,
};
use d2b_resource_api::authz::{
    ApiCatalog, BindingScope, BootstrapPhase, BoundSubject, CompiledRole, CompiledRoleBinding,
    NativeAuthorizer, PolicyRule, PolicySet, SessionVerb,
};
use d2b_resource_store::PolicySnapshot;
use d2b_session::{
    AuthenticatedSessionRouteBinding, OwnedTransport, SessionAcceptor, SessionEngine,
    TransportEvidence, operation_catalog_entry, ttrpc_stream_id,
};
use d2b_session_unix::{
    CreditPool, CreditScopeSet, PeerIdentityPolicy, SeqpacketSocket,
    UnixSeqpacketTransport, UnixSessionError, VerifiedUnixPeer,
};
use sha2::{Digest, Sha256};
use socket2::{Domain, SockAddr, Socket, Type};
use protobuf::Message;
use tokio::sync::Mutex as AsyncMutex;
use serde::Deserialize;
use ttrpc::proto::{Code as TtrpcCode, MessageHeader, Request as TtrpcRequest, Response as TtrpcResponse, Status as TtrpcStatus};
use rustix::net::{SocketFlags, accept_with};
use nix::unistd::{getgid, Group};

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

    /// Clone the daemon-owned request receiver demultiplexed by the bus.
    pub fn request_receiver(&self) -> ComponentRequestReceiver {
        self.ingress.component_request_receiver()
    }
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
pub struct InteractionComposition<
    S,
    G = UnavailableGuestFrontendEffects,
>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
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
    bytes: Vec<u8>,
    now_secs: u64,
}

#[derive(Debug, Deserialize)]
struct DisplayReconcileRequest {
    spec: WaylandSessionSpec,
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
    status.set_message(match code {
        TtrpcCode::OK => "",
        TtrpcCode::UNIMPLEMENTED => "interaction-operation-unsupported",
        TtrpcCode::UNAUTHENTICATED => "interaction-session-unavailable",
        TtrpcCode::INVALID_ARGUMENT => "interaction-request-invalid",
        TtrpcCode::FAILED_PRECONDITION => "interaction-runtime-rejected",
        TtrpcCode::UNAVAILABLE => "interaction-response-unavailable",
        _ => "interaction-request-failed",
    }.to_owned());
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
        Self {
            registrar,
            supervisor,
            guest_frontend: UnavailableGuestFrontendEffects,
            sessions: BTreeMap::new(),
            display: None,
            clipboard: None,
            notification: None,
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
    pub fn new_with_guest_frontend(registrar: ZoneRegistrar, supervisor: S, guest_frontend: G) -> Self {
        Self {
            registrar,
            supervisor,
            guest_frontend,
            sessions: BTreeMap::new(),
            display: None,
            clipboard: None,
            notification: None,
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
        if self.sessions.contains_key(&service) {
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
        ) {
            if self.ensure_clipboard().is_err() {
                let RegisteredInteractionSession { ingress, .. } = session;
                let _ = self.registrar.revoke(ingress).await;
                return Err(InteractionAdmissionError::ServiceUnavailable);
            }
        } else if service == d2b_provider_notification_desktop::SERVICE_PACKAGE
            && self.ensure_notification().is_err()
        {
            let RegisteredInteractionSession { ingress, .. } = session;
            let _ = self.registrar.revoke(ingress).await;
            return Err(InteractionAdmissionError::ServiceUnavailable);
        }
        self.sessions.insert(service.clone(), session);
        Ok(self
            .sessions
            .get(&service)
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
    pub fn route_for_service(
        &self,
        service: &str,
    ) -> Option<&AuthenticatedSessionRouteBinding> {
        self.sessions.get(service).map(RegisteredInteractionSession::route)
    }

    /// Return the number of live authenticated interaction sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Whether one exact interaction service has an admitted session.
    pub fn has_service_session(&self, service: &str) -> bool {
        self.sessions.contains_key(service)
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
        let stream_id = ttrpc_stream_id(&frame).map_err(|_| "invalid-request-frame")?;
        let payload = frame
            .get(ttrpc::proto::MESSAGE_HEADER_LENGTH..)
            .ok_or("invalid-request-frame")?;
        let request = match TtrpcRequest::parse_from_bytes(payload) {
            Ok(request) => request,
            Err(_) => {
                self.send_component_response(
                    service,
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
                service,
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
        let Some(entry) =
            operation_catalog_entry(service, &request.method, d2b_session::OperationKind::Method)
        else {
            self.send_component_response(
                service,
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
                .get(service)
                .ok_or("interaction-session-unavailable")?;
            interaction_route_for_member(registered.route(), &request.method)
                .map_err(|_| "interaction-route-invalid")?
        };
        let operation_id = OperationId::parse(format!(
            "interaction:{service}:{stream_id}:{}",
            request.method
        ))
        .map_err(|_| "interaction-operation-invalid")?;
        let operation = OperationSpec::new(operation_id, 60_000)
            .map_err(|_| "interaction-operation-invalid")?;
        let ingress = self
            .sessions
            .get(service)
            .ok_or("interaction-session-unavailable")?
            .ingress();
        let lease = match ingress.begin_local_invoke(route, operation).await {
            Ok(lease) => lease,
            Err(_) => {
                self.send_component_response(
                    service,
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
            .dispatch_interaction_operation(service, &request.method, &request.payload)
        {
            Ok((payload, finalize_after_response)) => {
                (TtrpcCode::OK, payload, finalize_after_response)
            }
            Err(error) => (error.code(), Vec::new(), false),
        };
        self.send_component_response(
            service,
            encode_interaction_response(stream_id, code, response_payload)
                .map_err(|_| "response-encode-failed")?,
        )
        .await?;
        lease.finish().map_err(|_| "interaction-operation-failed")?;
        if finalize_after_response {
            self.finalize_async(d2b_provider_display_wayland::GraceState::Expired)
                .await
                .map_err(|_| "interaction-finalization-failed")?;
        }
        let _ = entry;
        Ok(())
    }

    async fn send_component_response(
        &self,
        service: &str,
        frame: Vec<u8>,
    ) -> Result<(), String> {
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
        service: &str,
        method: &str,
        payload: &[u8],
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
                let token = self
                    .capture_guest_clipboard(&request.mime, &request.bytes, request.now_secs)
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                Ok((
                    serde_json::to_vec(&serde_json::json!({"entry_digest": token}))
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }
            (
                d2b_provider_clipboard_wayland::BRIDGE_SERVICE,
                "ClipboardBridgeService/CaptureHost",
            ) => {
                let request: ClipboardCaptureRequest = serde_json::from_slice(payload)
                    .map_err(|_| InteractionDispatchError::InvalidPayload)?;
                let token = self
                    .capture_host_clipboard(
                        &request.mime,
                        &request.bytes,
                        None,
                        request.now_secs,
                    )
                    .map_err(|_| InteractionDispatchError::RuntimeFailure)?;
                Ok((
                    serde_json::to_vec(&serde_json::json!({"entry_digest": token}))
                        .map_err(|_| InteractionDispatchError::RuntimeFailure)?,
                    false,
                ))
            }
            (
                d2b_provider_clipboard_wayland::MANAGEMENT_SERVICE,
                "ClipboardService/Drain",
            )
            | (
                d2b_provider_clipboard_wayland::BRIDGE_SERVICE,
                "ClipboardBridgeService/Drain",
            ) => {
                if !payload.is_empty() {
                    return Err(InteractionDispatchError::InvalidPayload);
                }
                let route = self
                    .route_for_service(service)
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
            (
                d2b_provider_notification_desktop::SERVICE_PACKAGE,
                "NotificationService/Drain",
            ) => {
                if !payload.is_empty() {
                    return Err(InteractionDispatchError::InvalidPayload);
                }
                if self
                    .route_for_service(service)
                    .is_none()
                {
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
            (
                d2b_provider_clipboard_wayland::MANAGEMENT_SERVICE,
                "ClipboardService/Reconcile",
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
            self.clipboard = Some(
                d2b_provider_clipboard_wayland::ClipboardRuntime::new(
                    d2b_provider_clipboard_wayland::Policy::default(),
                    128,
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
            let config =
                d2b_provider_notification_desktop::NotificationProviderConfig::new(Vec::new())?;
            self.notification = Some(
                d2b_provider_notification_desktop::NotificationRuntime::new(
                    config,
                    InteractionDrainEffects::default(),
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
        let route = self
            .route_for_service(d2b_provider_display_wayland::SERVICE_PACKAGE)
            .ok_or(InteractionDependencyError::SessionUnauthenticated)?
            .clone();
        let clipboard_dependency =
            d2b_provider_clipboard_wayland::DisplayDependencyEvidence::from_authenticated_route(
                route.clone(),
            )
            .map_err(|_| InteractionDependencyError::DisplayUnavailable)?;
        if let Some(clipboard) = self.clipboard.as_mut() {
            clipboard
                .reconcile_display(Some(clipboard_dependency))
                .map_err(InteractionDependencyError::Clipboard)?;
        }
        let source_routes = self
            .route_for_service(d2b_provider_notification_desktop::SERVICE_PACKAGE)
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
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
            .route_for_service(d2b_provider_clipboard_wayland::BRIDGE_SERVICE)
            .ok_or(ClipboardServiceError::SessionUnauthenticated)?
            .clone();
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
                DisplaySupervisorEffects::new_with_guest_frontend(
                    supervisor,
                    guest_frontend,
                ),
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
        let policy = WaylandPolicySnapshot::from_authenticated_route(
            &route,
            request.spec.policy_ref().clone(),
            policy_generation,
            FilterInput::default(),
            request.spec.filter().clone(),
        )
        .map_err(|_| DisplayRuntimeError::InvalidPolicy)?;
        self.reconcile_display(
            DisplayController::new(8),
            &request.spec,
            DependencyState::from_authenticated_route(&route)
                .map_err(|_| DisplayRuntimeError::InvalidPolicy)?,
            WorkerRestartEvidence::from_supervisor(1, None, None, 1),
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
        self.sessions.clear();
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
        let sessions = std::mem::take(&mut self.sessions);
        let mut registration_failed = false;
        for (_, session) in sessions {
            if self.registrar.revoke(session.ingress).await.is_err() {
                registration_failed = true;
            }
        }
        if registration_failed {
            failure.get_or_insert(InteractionFinalizeError::Registration);
        }
        failure.map_or(Ok(report), Err)
    }

    async fn remove_service_session(&mut self, service: &str) -> Result<(), String> {
        let Some(session) = self.sessions.remove(service) else {
            return Ok(());
        };
        self.registrar
            .revoke(session.ingress)
            .await
            .map_err(|_| "interaction-session-revocation-failed".to_owned())
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
    session_digest: [u8; 32],
    reconnect_generation: u64,
    policy_generation: u64,
    teardown_generation: u64,
}

struct LiveWorker {
    identity: ProcessIdentityDigest,
    policy_generation: u64,
    teardown_generation: u64,
    session_digest: [u8; 32],
}

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
        let compositor = grant_digest(
            "compositor",
            session_digest,
            session.reconnect_generation(),
            teardown_generation,
        );
        let gpu = grant_digest(
            "gpu",
            session_digest,
            session.reconnect_generation(),
            teardown_generation,
        );
        let frontend_gpu = grant_digest(
            "frontend-gpu",
            session_digest,
            session.reconnect_generation(),
            teardown_generation,
        );
        Ok(LaunchGrants::from_daemon(
            compositor,
            gpu,
            frontend_gpu,
            session_digest,
            session.reconnect_generation(),
            teardown_generation,
        ))
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
            if let Some(previous) = self.guest_worker.take() {
                self.guest_frontend
                    .stop(
                        &guest,
                        previous.policy_generation,
                        previous.teardown_generation,
                        previous.session_digest,
                    )
                    .map_err(|_| WorkerEffectError::CleanupIncomplete)?;
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
        if let Some(previous) = self.identities.remove(&role) {
            let supervisor = self.supervisor.clone();
            run_effect(move || async move {
                supervisor
                    .stop(&previous.identity, StopClass::Terminate)
                    .await
                    .map_err(|_| WorkerEffectError::CleanupIncomplete)
            })?;
        }
        let supervisor = self.supervisor.clone();
        let adopted = run_effect(move || {
            let supervisor = supervisor.clone();
            let process_ticket = process_ticket.clone();
            async move {
                if let Some(candidate) = supervisor
                    .observe(&process_ticket)
                    .await
                    .map_err(|_| WorkerEffectError::WorkerUnavailable)?
                {
                    supervisor
                        .open_pidfd(&candidate)
                        .await
                        .map_err(|_| WorkerEffectError::WorkerUnavailable)?;
                    Ok(candidate.identity)
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
            let Some(worker) = self.guest_worker.take() else {
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
            return self.guest_frontend.stop(
                &guest,
                worker.policy_generation,
                worker.teardown_generation,
                worker.session_digest,
            );
        }
        let Some(worker) = self.identities.remove(&role) else {
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
#[derive(Debug, Default)]
pub struct InteractionDrainEffects {
    drained: bool,
    authority_released: bool,
    source_effects: usize,
}

impl InteractionDrainEffects {
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

impl SourceProcessEffectPort for InteractionDrainEffects {
    fn apply(
        &mut self,
        plan: &SourceReconcileResult,
    ) -> Result<SourceProcessEffectReceipt, &'static str> {
        self.source_effects = self
            .source_effects
            .saturating_add(plan.start_endpoints.len() + plan.stop_endpoints.len());
        SourceProcessEffectReceipt::from_daemon(plan)
    }
}

impl NotificationProcessEffectPort for InteractionDrainEffects {
    fn release_authority(&mut self) -> Result<(), &'static str> {
        if self.source_effects > 0 || self.drained {
            self.authority_released = true;
            Ok(())
        } else {
            Ok(())
        }
    }
}

fn grant_digest(
    label: &str,
    session: [u8; 32],
    reconnect_generation: u64,
    teardown: u64,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2bd-display-grant-v1");
    digest.update(label.as_bytes());
    digest.update([0]);
    digest.update(session);
    digest.update(reconnect_generation.to_be_bytes());
    digest.update(teardown.to_be_bytes());
    digest.finalize().into()
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
                provider_identity: observed.intent.provider_identity,
                template_identity: observed.intent.template_identity,
                generation: observed.intent.generation,
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
        Ok(BrokerLaunchIntent {
            vm_id: VmId::new(intent.vm_name.clone()),
            role_id: RoleId::new(intent.role_id.clone()),
            role: RunnerRole::WaylandProxy,
            bundle_runner_intent_ref: BundleOpId::new(intent.intent_id.clone()),
            provider_identity,
            template_identity,
            generation: ticket.resource_generation().get(),
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
    role_id: String,
    role: RunnerRole,
    bundle_runner_intent_ref: String,
    provider_identity: [u8; 32],
    template_identity: [u8; 32],
    generation: u64,
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
                    role_id: RoleId::new(record.role_id),
                    role: record.role,
                    bundle_runner_intent_ref: BundleOpId::new(record.bundle_runner_intent_ref),
                    provider_identity: record.provider_identity,
                    template_identity: record.template_identity,
                    generation: record.generation,
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

/// Construct the daemon-owned authenticated interaction composition for one
/// trusted Zone.  The registrar is created here, rather than in Provider
/// code, and its production resolver derives the Provider identity from the
/// verified local peer.
pub fn production_interaction_composition(
    bundle: BundleResolver,
    daemon_uid: u32,
    observation_path: PathBuf,
    zone: ZoneId,
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
        [zone.clone()],
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
    let policy = PolicySet::new(&catalog, 1, vec![role], vec![binding])
        .map_err(|_| BusError::InvalidConfig)?;
    let native =
        NativeAuthorizer::new(catalog, Some(policy)).map_err(|_| BusError::InvalidConfig)?;
    let state = d2b_resource_api::authz::AuthorizationState {
        snapshot: PolicySnapshot {
            policy_revision: 1,
            api_catalog_revision: 1,
            active_configuration_revision: ConfigurationGeneration::new(1)
                .map_err(|_| BusError::InvalidConfig)?,
            controller_generation: Some(
                ControllerGeneration::new(1).map_err(|_| BusError::InvalidConfig)?,
            ),
        },
        zone_policy_revision: ZoneRevision::new(1),
        bootstrap_phase: BootstrapPhase::Disabled,
        now_tick: 1,
    };
    let authorizer = BusAuthorizer::new(native, state).map_err(|_| BusError::InvalidConfig)?;
    let (_bus, registrar) = ZoneBus::with_clock_observer_and_metrics(
        zone,
        authorizer,
        BusConfig::default(),
        std::sync::Arc::new(d2b_bus::ManualClock::new(1)),
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
    Ok(InteractionComposition::new_with_guest_frontend(
        registrar,
        production_display_supervisor(bundle, daemon_uid, observation_path),
        guest_frontend,
    ))
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
    runtime: Arc<AsyncMutex<Option<InteractionComposition<S, G>>>>,
    state_dir: PathBuf,
    zone: ZoneId,
    expected_peer_uid: u32,
) -> Result<InteractionListenerSet, String>
where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    std::fs::create_dir_all(&state_dir).map_err(|error| error.to_string())?;
    let mut paths = Vec::with_capacity(INTERACTION_SERVICES.len());
    let stop = Arc::new(AtomicBool::new(false));
    let mut threads = Vec::with_capacity(INTERACTION_SERVICES.len());
    for (service, _) in INTERACTION_SERVICES {
        let slug = service.replace('.', "-");
        let path = state_dir.join(format!("interaction-{slug}.sock"));
        let listener = bind_interaction_listener(&path).map_err(|error| {
            format!("bind interaction listener {}: {error}", path.display())
        })?;
        let runtime = Arc::clone(&runtime);
        let zone = zone.clone();
        let service = (*service).to_owned();
        let thread_stop = Arc::clone(&stop);
        let failure_stop = Arc::clone(&stop);
        let thread_name = format!("d2bd-interaction-{}", service.replace('.', "-"));
        thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                interaction_accept_loop(
                    listener,
                    runtime,
                    zone,
                    service,
                    expected_peer_uid,
                    thread_stop,
                )
            })
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
    Ok(InteractionListenerSet {
        paths,
        stop,
        threads: Mutex::new(threads),
    })
}

/// Daemon-owned handles for the interaction listener set.
pub struct InteractionListenerSet {
    paths: Vec<PathBuf>,
    stop: Arc<AtomicBool>,
    threads: Mutex<Vec<thread::JoinHandle<()>>>,
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
        self.remove_socket_paths();
    }

    fn remove_socket_paths(&self) {
        for path in &self.paths {
            if std::fs::symlink_metadata(path)
                .is_ok_and(|metadata| metadata.file_type().is_socket())
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
        for path in &self.paths {
            if std::fs::symlink_metadata(path)
                .is_ok_and(|metadata| metadata.file_type().is_socket())
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn bind_interaction_listener(path: &std::path::Path) -> std::io::Result<Socket> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => std::fs::remove_file(path)?,
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "interaction listener path is not a socket",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = Socket::new(
        Domain::UNIX,
        Type::from(libc::SOCK_SEQPACKET),
        None,
    )?;
    listener.set_nonblocking(true)?;
    listener.bind(&SockAddr::unix(path)?)?;
    listener.listen(32)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
    Ok(listener)
}

fn interaction_accept_loop<S, G>(
    listener: Socket,
    runtime: Arc<AsyncMutex<Option<InteractionComposition<S, G>>>>,
    zone: ZoneId,
    service: String,
    expected_peer_uid: u32,
    stop: Arc<AtomicBool>,
) where
    S: ProcessLaunchEffectPort + Clone + Send + Sync + 'static,
    G: GuestFrontendEffectPort + 'static,
{
    while !stop.load(Ordering::Acquire) {
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
        let _ = thread::Builder::new()
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
                        ))
                    });
                if let Err(error) = result {
                    tracing::debug!(%error, "interaction ComponentSession refused");
                }
            });
    }
}

async fn admit_interaction_socket<S, G>(
    socket: std::os::fd::OwnedFd,
    runtime: Arc<AsyncMutex<Option<InteractionComposition<S, G>>>>,
    zone: ZoneId,
    service: String,
    expected_peer_uid: u32,
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
    let resolver: d2b_session_unix::DescriptorPolicyResolver =
        Arc::new(|_| Err(UnixSessionError::DescriptorMismatch));
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
    let engine = SessionEngine::establish_responder(
        transport,
        policy.clone(),
        d2b_session::HandshakeCredentials::Nn,
        Instant::now(),
    )
    .await
    .map_err(|error| error.to_string())?;
    let acceptor = {
        let guard = runtime.lock().await;
        let composition = guard
            .as_ref()
            .ok_or_else(|| "interaction runtime unavailable".to_owned())?;
        composition
            .registrar()
            .component_session_acceptor(policy, verified_peer)
            .map_err(|error| error.to_string())?
    };
    let evidence = TransportEvidence::new(EvidenceClass::UnixPeer, binding_digest(
        &interaction_endpoint_policy(&service, 1)
            .expect("service policy was already validated"),
    ));
    let request_receiver = {
        let mut guard = runtime.lock().await;
        let composition = guard
            .as_mut()
            .ok_or_else(|| "interaction runtime unavailable".to_owned())?;
        let registered = composition
            .admit_and_register(acceptor, engine, evidence, 1)
            .await
            .map_err(|error| error.to_string())?;
        registered.request_receiver()
    };
    loop {
        let frame = match request_receiver.recv().await {
            Ok(frame) => frame,
            Err(_) => break,
        };
        let mut guard = runtime.lock().await;
        let composition = guard
            .as_mut()
            .ok_or_else(|| "interaction runtime unavailable".to_owned())?;
        if let Err(error) = composition
            .dispatch_component_request(&service, frame)
            .await
        {
            tracing::debug!(%error, service = %service, "interaction request rejected");
        }
        if !composition.has_service_session(&service) {
            break;
        }
    }
    let mut guard = runtime.lock().await;
    let composition = guard
        .as_mut()
        .ok_or_else(|| "interaction runtime unavailable".to_owned())?;
    composition.remove_service_session(&service).await?;
    let _ = zone;
    Ok(())
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
    use d2b_process::{
        BackendLaunch, BackendObservation, ObservedIdentity, ProcessEffectBackend,
        ProcessEffectError, ProcessRequest, ProcessStopClass, WaitReapOwner,
    };
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
            role_id: "wayland-proxy".to_owned(),
            role: RunnerRole::WaylandProxy,
            bundle_runner_intent_ref: "runner:guest-a:wayland-proxy".to_owned(),
            provider_identity: [1; 32],
            template_identity: [2; 32],
            generation: 3,
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
    }

    #[test]
    fn interaction_response_is_a_correlated_ttrpc_response() {
        let frame = encode_interaction_response(41, TtrpcCode::OK, b"{\"ok\":true}".to_vec())
            .expect("response encoding");
        assert!(d2b_session::ttrpc_is_response(&frame));
        assert_eq!(d2b_session::ttrpc_stream_id(&frame).unwrap(), 41);
        let header = MessageHeader::from(&frame[..ttrpc::proto::MESSAGE_HEADER_LENGTH]);
        let response = TtrpcResponse::parse_from_bytes(&frame[ttrpc::proto::MESSAGE_HEADER_LENGTH..])
            .expect("response protobuf");
        assert_eq!(header.stream_id, 41);
        assert_eq!(response.status().code(), TtrpcCode::OK);
        assert_eq!(response.payload, b"{\"ok\":true}");
    }

    #[test]
    fn interaction_listener_policy_binds_service_and_transport() {
        let policy = interaction_endpoint_policy(d2b_provider_display_wayland::SERVICE_PACKAGE, 7)
            .expect("display policy");
        assert_eq!(
            policy.service,
            ServicePackage::DisplayV3,
        );
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
        let listener = bind_interaction_listener(&path).expect("listener socket");
        drop(listener);
        let listeners = InteractionListenerSet {
            paths: vec![path.clone()],
            stop: Arc::new(AtomicBool::new(false)),
            threads: Mutex::new(Vec::new()),
        };

        assert!(path.exists());
        listeners.stop();
        assert!(!path.exists());
    }
}
