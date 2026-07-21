use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        AttachmentPolicy, AttachmentPolicyKind, EndpointPolicy, EndpointPurpose, EndpointRole,
        IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, PurposeClass,
        ServicePackage, TransportBinding, TransportClass,
    },
    v2_services::{
        SERVICE_INVENTORY,
        common::{
            CancelOutcome as WireCancelOutcome, CancelRequest as WireCancelRequest,
            CancelResponse as WireCancelResponse, DesiredState as WireDesiredState, ErrorEnvelope,
            ErrorKind, Outcome, RetryClass, ServiceRequest, ServiceResponse,
        },
        runtime_systemd_user_ttrpc::{
            RuntimeSystemdUserService as RuntimeSystemdUserTtrpc,
            create_runtime_systemd_user_service,
        },
        service_schema_fingerprint,
        shell_ttrpc::{ShellService as ShellTtrpc, create_shell_service},
        tty_ttrpc::{TtyService as TtyTtrpc, create_tty_service},
    },
};
use d2b_session::{
    ComponentSessionDriver, HandshakeCredentials, SessionEngine, serve_ttrpc_services,
};
use d2b_session_unix::{
    ActivatedSeqpacketListeners, CreditPool, CreditScopeSet, DescriptorPolicyResolver,
    PeerIdentityPolicy, SeqpacketSocket, UnixSeqpacketTransport, UnixSessionError,
};
use nix::unistd::geteuid;
use protobuf::{EnumOrUnknown, MessageField};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fmt,
    future::Future,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinSet;
use ttrpc::r#async::TtrpcContext;

use crate::{
    controller_allowlist::ControllerAllowlist,
    services::{
        AuthenticatedRuntimeSession, CompositionError, RuntimeComposition,
        runtime_systemd_user::{
            AuthenticatedTerminalAttachment, CancelRequest, CancelResult,
            ConfiguredProcessResolver, DesiredState, ResolvedProcess, RuntimeMethod, RuntimeOwner,
            RuntimeRequest, RuntimeResource, RuntimeServiceError, SystemdUserRuntimePort,
            WaylandControlPort, WaylandDisplayLease,
        },
    },
    shell_runtime::{
        AuthenticatedSystemdUserRuntime, CancelOutcome as ShellCancelOutcome, ShellMethod,
        ShellOwner, ShellRequest, ShellServiceError, VerifiedTransientScope,
    },
    tty_exec::{
        TransientUserScope, TtyOneShotError, TtyOneShotRequest, TtyOneShotRuntime, TtyOneShotSpec,
        ValidatedTerminal,
    },
};

pub const ACTIVATED_LISTENER_NAME: &str = "runtime-systemd-user";
const MAX_ACTIVE_SESSIONS: usize = 64;
const SHELL_OUTPUT_BUDGET: usize = crate::output_ring::MAX_TOTAL_RING_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerError {
    Activation,
    InvalidIdentity,
    Generation,
    Signal,
    Allowlist,
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Activation => "socket-activation-failed",
            Self::InvalidIdentity => "runtime-identity-invalid",
            Self::Generation => "runtime-generation-unavailable",
            Self::Signal => "shutdown-signal-unavailable",
            Self::Allowlist => "controller-allowlist-invalid",
        })
    }
}

impl std::error::Error for ServerError {}

/// Immutable Nix-owned document naming the exact, bounded set of enabled
/// host-local realm controller UIDs authorized to reach this requesting
/// user's endpoint (see `nixos-modules/unsafe-local-helper.nix`). Absent
/// means no wiring has been provisioned yet; the helper then falls back to
/// the safe same-uid-only default rather than failing to start.
const CONTROLLER_ALLOWLIST_ENV: &str = "D2B_UNSAFE_LOCAL_CONTROLLER_ALLOWLIST";

fn load_controller_allowlist(uid: u32) -> Result<ControllerAllowlist, ServerError> {
    let Some(path) = std::env::var_os(CONTROLLER_ALLOWLIST_ENV) else {
        return Ok(ControllerAllowlist::empty());
    };
    // The allowlist document is keyed by this process's own username, never
    // by anything peer-supplied, so a connecting peer can never select
    // which row authorizes it.
    let username = uzers::get_user_by_uid(uid)
        .and_then(|user| user.name().to_str().map(str::to_owned))
        .ok_or(ServerError::Allowlist)?;
    let document = std::fs::read(&path).map_err(|_| ServerError::Allowlist)?;
    ControllerAllowlist::resolve(&document, &username).map_err(|_| ServerError::Allowlist)
}

pub async fn run() -> Result<(), ServerError> {
    let uid = geteuid().as_raw();
    if uid == 0 || uid != nix::unistd::getuid().as_raw() {
        return Err(ServerError::InvalidIdentity);
    }
    let allowlist = load_controller_allowlist(uid)?;
    let generation = random_generation()?;
    let listeners = ActivatedSeqpacketListeners::from_systemd(&[ACTIVATED_LISTENER_NAME])
        .map_err(|_| ServerError::Activation)?;
    let shutdown = async {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .map_err(|_| ServerError::Signal)?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                .map_err(|_| ServerError::Signal)?;
        tokio::select! {
            _ = terminate.recv() => Ok(()),
            _ = interrupt.recv() => Ok(()),
        }
    };
    serve_until_shutdown(&listeners, generation, &allowlist, shutdown).await
}

fn random_generation() -> Result<u64, ServerError> {
    let mut bytes = [0_u8; 8];
    getrandom::getrandom(&mut bytes).map_err(|_| ServerError::Generation)?;
    let generation = u64::from_ne_bytes(bytes);
    Ok(generation.max(1))
}

#[async_trait]
trait ActivatedListener {
    async fn accept(&self) -> Result<SeqpacketSocket, ServerError>;
}

#[async_trait]
impl ActivatedListener for ActivatedSeqpacketListeners {
    async fn accept(&self) -> Result<SeqpacketSocket, ServerError> {
        self.accept(ACTIVATED_LISTENER_NAME)
            .await
            .map_err(|_| ServerError::Activation)
    }
}

async fn serve_until_shutdown<L, S>(
    listener: &L,
    generation: u64,
    allowlist: &ControllerAllowlist,
    shutdown: S,
) -> Result<(), ServerError>
where
    L: ActivatedListener + Sync,
    S: Future<Output = Result<(), ServerError>>,
{
    tokio::pin!(shutdown);
    let mut sessions = JoinSet::new();
    loop {
        tokio::select! {
            result = &mut shutdown => {
                result?;
                sessions.abort_all();
                while sessions.join_next().await.is_some() {}
                return Ok(());
            }
            accepted = listener.accept(), if sessions.len() < MAX_ACTIVE_SESSIONS => {
                let socket = accepted?;
                let allowlist = allowlist.clone();
                sessions.spawn(async move {
                    let _ = serve_socket(socket, generation, allowlist).await;
                });
            }
            completed = sessions.join_next(), if !sessions.is_empty() => {
                let _ = completed;
            }
        }
    }
}

/// Whether an already-authenticated peer uid may open a session on this
/// endpoint. This is a pure boolean decision: it never selects, returns, or
/// otherwise influences which uid anything executes as. The helper always
/// executes as `own_uid` (enforced once, in `run`); this only gates which
/// *other* connecting uid is additionally trusted to reach it.
fn peer_is_authorized(peer_uid: u32, own_uid: u32, allowlist: &ControllerAllowlist) -> bool {
    peer_uid != 0 && own_uid != 0 && (peer_uid == own_uid || allowlist.contains(peer_uid))
}

async fn serve_socket(
    socket: SeqpacketSocket,
    generation: u64,
    allowlist: ControllerAllowlist,
) -> Result<(), ()> {
    let peer = socket.acceptor_peer_credentials().map_err(|_| ())?;
    let uid = geteuid().as_raw();
    if !peer_is_authorized(peer.uid().as_raw(), uid, &allowlist) {
        return Err(());
    }
    let gid = peer.gid().as_raw();
    let policy = endpoint_policy(uid, gid, generation).ok_or(())?;
    let expected_peer = peer;
    let resolver: DescriptorPolicyResolver =
        Arc::new(|_| Err(UnixSessionError::DescriptorMismatch));
    let transport = UnixSeqpacketTransport::new(
        socket,
        Locality::HostLocal,
        policy.limits,
        policy.attachment_policy,
        credit_scopes(policy.attachment_policy.max_per_session),
        resolver,
        PeerIdentityPolicy::accepted(expected_peer),
    )
    .map_err(|_| ())?;
    let engine = SessionEngine::establish_responder(
        transport,
        policy,
        HandshakeCredentials::Nn,
        Instant::now(),
    )
    .await
    .map_err(|_| ())?;
    let driver: Arc<dyn ComponentSessionDriver> = Arc::new(engine.into_driver());
    let services = service_registry(generation);
    serve_ttrpc_services(driver, services).await.map_err(|_| ())
}

pub fn endpoint_policy(uid: u32, gid: u32, generation: u64) -> Option<EndpointPolicy> {
    if uid == 0 || generation == 0 {
        return None;
    }
    let service = SERVICE_INVENTORY.iter().find(|service| {
        service.package == "d2b.runtime.systemd-user.v2"
            && service.service == "RuntimeSystemdUserService"
    })?;
    Some(EndpointPolicy {
        purpose: EndpointPurpose::RuntimeSystemdUser,
        purpose_class: PurposeClass::Local,
        initiator_role: EndpointRole::LocalRootController,
        responder_role: EndpointRole::RuntimeSystemdUserAgent,
        service: ServicePackage::RuntimeSystemdUserV2,
        schema_fingerprint: service_schema_fingerprint(service),
        noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport: TransportClass::UnixSeqpacket,
            locality: Locality::HostLocal,
            channel_binding: channel_binding(uid, gid),
            identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
        },
        reconnect_generation: generation,
        attachment_policy: AttachmentPolicy {
            kind: AttachmentPolicyKind::PacketAtomic,
            max_per_packet: 1,
            max_per_request: 1,
            max_per_operation: 1,
            max_per_session: 64,
            credentials_allowed: false,
        },
    })
}

pub fn channel_binding(uid: u32, gid: u32) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b.runtime.systemd-user.v2\0unix-seqpacket\0");
    digest.update(uid.to_be_bytes());
    digest.update(gid.to_be_bytes());
    digest.update(EndpointRole::RuntimeSystemdUserAgent.as_str().as_bytes());
    digest.finalize().into()
}

fn credit_scopes(limit: u16) -> CreditScopeSet {
    let limit = usize::from(limit.max(1));
    CreditScopeSet::new(
        CreditPool::new(limit).expect("positive packet credit"),
        CreditPool::new(limit).expect("positive request credit"),
        CreditPool::new(limit).expect("positive operation credit"),
        CreditPool::new(limit).expect("positive session credit"),
        CreditPool::new(limit).expect("positive process credit"),
        CreditPool::new(limit).expect("positive host credit"),
    )
}

fn service_registry(generation: u64) -> HashMap<String, ttrpc::r#async::Service> {
    let state = Arc::new(SessionServices::new(generation));
    let mut services =
        create_runtime_systemd_user_service(Arc::new(RuntimeAdapter(Arc::clone(&state))));
    services.extend(create_shell_service(Arc::new(ShellAdapter(Arc::clone(
        &state,
    )))));
    services.extend(create_tty_service(Arc::new(TtyUnavailable)));
    services
}

type Composition = RuntimeComposition<UnavailableResolver, UnavailableWayland, UnavailableBackend>;

struct SessionServices {
    generation: u64,
    composition: Mutex<Option<Composition>>,
}

impl SessionServices {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            composition: Mutex::new(None),
        }
    }

    fn with_composition<T>(
        &self,
        realm_id: &str,
        workload_id: &str,
        dispatch: impl FnOnce(&mut Composition) -> Result<T, CompositionError>,
    ) -> Result<T, CompositionError> {
        let mut guard = self
            .composition
            .lock()
            .map_err(|_| CompositionError::SessionUnavailable)?;
        if guard.is_none() {
            let session = AuthenticatedRuntimeSession::for_current_process(
                self.generation,
                realm_id.to_owned(),
                workload_id.to_owned(),
            )?;
            *guard = Some(RuntimeComposition::new(
                session,
                UnavailableResolver,
                UnavailableWayland,
                UnavailableBackend,
                SHELL_OUTPUT_BUDGET,
            )?);
        }
        dispatch(guard.as_mut().ok_or(CompositionError::SessionUnavailable)?)
    }
}

struct RuntimeAdapter(Arc<SessionServices>);
struct ShellAdapter(Arc<SessionServices>);
struct TtyUnavailable;

impl RuntimeAdapter {
    fn dispatch(
        &self,
        method: RuntimeMethod,
        wire: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        let request = match decode_runtime_request(&wire) {
            Ok(request) => request,
            Err(kind) => return Ok(error_response(kind)),
        };
        let now = now_unix_ms();
        let result =
            self.0
                .with_composition(&request.realm_id, &request.workload_id, |composition| {
                    composition.dispatch_runtime(method, &request, &[], now)
                });
        Ok(match result {
            Ok(response) => {
                let mut wire = ServiceResponse::new();
                wire.outcome = EnumOrUnknown::new(match response.outcome {
                    crate::services::runtime_systemd_user::RuntimeOutcome::Succeeded => {
                        Outcome::OUTCOME_SUCCEEDED
                    }
                    crate::services::runtime_systemd_user::RuntimeOutcome::Degraded => {
                        Outcome::OUTCOME_DEGRADED
                    }
                });
                wire.operation_id = response.operation_id;
                wire.resource_handle = response.resource_handle;
                wire.stream_id = response.stream_id;
                wire.result_digest = response.result_digest.to_vec();
                wire.attachment_indexes = response.attachment_indexes;
                wire
            }
            Err(error) => error_response(composition_error_kind(error)),
        })
    }

    fn cancel(&self, wire: WireCancelRequest) -> ttrpc::Result<WireCancelResponse> {
        let request_id: [u8; 16] = match wire.request_id.try_into() {
            Ok(value) if value != [0; 16] => value,
            _ => {
                return Ok(cancel_response(
                    WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
                ));
            }
        };
        let request = CancelRequest {
            session_generation: wire.session_generation,
            request_id,
        };
        let result = self
            .0
            .composition
            .lock()
            .map_err(|_| rpc_internal())?
            .as_mut()
            .map(|composition| composition.cancel_runtime(&request));
        Ok(match result {
            Some(Ok(response)) => cancel_response(match response.outcome {
                CancelResult::CancelledBeforeDispatch => {
                    WireCancelOutcome::CANCEL_OUTCOME_CANCELLED_BEFORE_DISPATCH
                }
                CancelResult::CancellationSignalled => {
                    WireCancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
                }
                CancelResult::AlreadyTerminal => WireCancelOutcome::CANCEL_OUTCOME_ALREADY_TERMINAL,
                CancelResult::UnknownRequest => WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
            }),
            Some(Err(CompositionError::Runtime(RuntimeServiceError::GenerationMismatch))) => {
                cancel_response(WireCancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH)
            }
            _ => cancel_response(WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST),
        })
    }
}

#[async_trait]
impl RuntimeSystemdUserTtrpc for RuntimeAdapter {
    async fn ensure_scope(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(RuntimeMethod::EnsureScope, request)
    }

    async fn start_process(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(RuntimeMethod::StartProcess, request)
    }

    async fn inspect_process(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(RuntimeMethod::InspectProcess, request)
    }

    async fn adopt_process(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(RuntimeMethod::AdoptProcess, request)
    }

    async fn stop_process(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(RuntimeMethod::StopProcess, request)
    }

    async fn open_terminal(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(RuntimeMethod::OpenTerminal, request)
    }

    async fn cancel(
        &self,
        _context: &TtrpcContext,
        request: WireCancelRequest,
    ) -> ttrpc::Result<WireCancelResponse> {
        RuntimeAdapter::cancel(self, request)
    }
}

impl ShellAdapter {
    fn dispatch(
        &self,
        method: ShellMethod,
        wire: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        let request = match decode_shell_request(method, &wire) {
            Ok(request) => request,
            Err(kind) => return Ok(error_response(kind)),
        };
        let now = now_unix_ms();
        let result =
            self.0
                .with_composition(&request.realm_id, &request.workload_id, |composition| {
                    composition.dispatch_shell(&request, Vec::new(), now)
                });
        Ok(match result {
            Ok(response) => {
                let mut wire = ServiceResponse::new();
                wire.outcome = EnumOrUnknown::new(match response.state {
                    crate::shell_runtime::ShellState::Degraded => Outcome::OUTCOME_DEGRADED,
                    _ => Outcome::OUTCOME_SUCCEEDED,
                });
                wire.operation_id = response.operation_id;
                wire.resource_handle = response.resource_id;
                wire.stream_id = response.stream_id;
                wire.attachment_indexes = response.attachment_indexes;
                wire
            }
            Err(error) => error_response(composition_error_kind(error)),
        })
    }

    fn cancel(&self, request: WireCancelRequest) -> ttrpc::Result<WireCancelResponse> {
        if request.session_generation != self.0.generation {
            return Ok(cancel_response(
                WireCancelOutcome::CANCEL_OUTCOME_GENERATION_MISMATCH,
            ));
        }
        Ok(cancel_response(
            WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
        ))
    }
}

#[async_trait]
impl ShellTtrpc for ShellAdapter {
    async fn create(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(ShellMethod::Create, request)
    }

    async fn attach(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(ShellMethod::Attach, request)
    }

    async fn detach(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(ShellMethod::Detach, request)
    }

    async fn list(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(ShellMethod::List, request)
    }

    async fn inspect(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(ShellMethod::Inspect, request)
    }

    async fn kill(
        &self,
        _context: &TtrpcContext,
        request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        self.dispatch(ShellMethod::Kill, request)
    }

    async fn cancel(
        &self,
        _context: &TtrpcContext,
        request: WireCancelRequest,
    ) -> ttrpc::Result<WireCancelResponse> {
        ShellAdapter::cancel(self, request)
    }
}

#[async_trait]
impl TtyTtrpc for TtyUnavailable {
    async fn enter_raw_mode(
        &self,
        _context: &TtrpcContext,
        _request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        Ok(error_response(ErrorKind::ERROR_KIND_UNAVAILABLE))
    }

    async fn restore_mode(
        &self,
        _context: &TtrpcContext,
        _request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        Ok(error_response(ErrorKind::ERROR_KIND_UNAVAILABLE))
    }

    async fn inspect(
        &self,
        _context: &TtrpcContext,
        _request: ServiceRequest,
    ) -> ttrpc::Result<ServiceResponse> {
        Ok(error_response(ErrorKind::ERROR_KIND_UNAVAILABLE))
    }

    async fn cancel(
        &self,
        _context: &TtrpcContext,
        _request: WireCancelRequest,
    ) -> ttrpc::Result<WireCancelResponse> {
        Ok(cancel_response(
            WireCancelOutcome::CANCEL_OUTCOME_UNKNOWN_REQUEST,
        ))
    }
}

fn decode_runtime_request(wire: &ServiceRequest) -> Result<RuntimeRequest, ErrorKind> {
    let metadata = wire
        .metadata
        .as_ref()
        .ok_or(ErrorKind::ERROR_KIND_INVALID_REQUEST)?;
    let scope = wire
        .scope
        .as_ref()
        .ok_or(ErrorKind::ERROR_KIND_INVALID_REQUEST)?;
    Ok(RuntimeRequest {
        request_id: exact_array(&metadata.request_id)?,
        idempotency_key: optional_array(&metadata.idempotency_key)?,
        issued_at_unix_ms: metadata.issued_at_unix_ms,
        expires_at_unix_ms: metadata.expires_at_unix_ms,
        session_generation: metadata.session_generation,
        realm_id: scope.realm_id.clone(),
        workload_id: scope.workload_id.clone(),
        resource_id: wire.resource_id.clone(),
        operation_id: wire.operation_id.clone(),
        request_digest: optional_array(&wire.request_digest)?,
        stream_id: wire.stream_id.clone(),
        attachment_indexes: wire.attachment_indexes.clone(),
        desired_state: match wire.desired_state.enum_value() {
            Ok(WireDesiredState::DESIRED_STATE_UNSPECIFIED) => DesiredState::Unspecified,
            Ok(WireDesiredState::DESIRED_STATE_PRESENT) => DesiredState::Present,
            Ok(WireDesiredState::DESIRED_STATE_RUNNING) => DesiredState::Running,
            Ok(WireDesiredState::DESIRED_STATE_STOPPED) => DesiredState::Stopped,
            Ok(WireDesiredState::DESIRED_STATE_ATTACHED) => DesiredState::Attached,
            _ => return Err(ErrorKind::ERROR_KIND_INVALID_REQUEST),
        },
    })
}

fn decode_shell_request(
    method: ShellMethod,
    wire: &ServiceRequest,
) -> Result<ShellRequest, ErrorKind> {
    let metadata = wire
        .metadata
        .as_ref()
        .ok_or(ErrorKind::ERROR_KIND_INVALID_REQUEST)?;
    let scope = wire
        .scope
        .as_ref()
        .ok_or(ErrorKind::ERROR_KIND_INVALID_REQUEST)?;
    Ok(ShellRequest {
        method,
        request_id: exact_array(&metadata.request_id)?,
        idempotency_key: optional_array(&metadata.idempotency_key)?,
        issued_at_unix_ms: metadata.issued_at_unix_ms,
        expires_at_unix_ms: metadata.expires_at_unix_ms,
        session_generation: metadata.session_generation,
        realm_id: scope.realm_id.clone(),
        workload_id: scope.workload_id.clone(),
        resource_id: wire.resource_id.clone(),
        operation_id: wire.operation_id.clone(),
        stream_id: wire.stream_id.clone(),
        attachment_indexes: wire.attachment_indexes.clone(),
        output_ring_bytes: usize::try_from(wire.page_size)
            .map_err(|_| ErrorKind::ERROR_KIND_INVALID_REQUEST)?,
    })
}

fn exact_array<const N: usize>(bytes: &[u8]) -> Result<[u8; N], ErrorKind> {
    let value: [u8; N] = bytes
        .try_into()
        .map_err(|_| ErrorKind::ERROR_KIND_INVALID_REQUEST)?;
    (value != [0; N])
        .then_some(value)
        .ok_or(ErrorKind::ERROR_KIND_INVALID_REQUEST)
}

fn optional_array<const N: usize>(bytes: &[u8]) -> Result<Option<[u8; N]>, ErrorKind> {
    if bytes.is_empty() {
        Ok(None)
    } else {
        exact_array(bytes).map(Some)
    }
}

fn error_response(kind: ErrorKind) -> ServiceResponse {
    let mut error = ErrorEnvelope::new();
    error.kind = EnumOrUnknown::new(kind);
    error.retry = EnumOrUnknown::new(match kind {
        ErrorKind::ERROR_KIND_UNAVAILABLE => RetryClass::RETRY_CLASS_AFTER_OBSERVATION,
        ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED => RetryClass::RETRY_CLASS_SAME_OPERATION,
        _ => RetryClass::RETRY_CLASS_NEVER,
    });
    let mut response = ServiceResponse::new();
    response.outcome = EnumOrUnknown::new(Outcome::OUTCOME_FAILED);
    response.error = MessageField::some(error);
    response
}

fn cancel_response(outcome: WireCancelOutcome) -> WireCancelResponse {
    let mut response = WireCancelResponse::new();
    response.outcome = EnumOrUnknown::new(outcome);
    response
}

fn composition_error_kind(error: CompositionError) -> ErrorKind {
    match error {
        CompositionError::OwnerMismatch
        | CompositionError::Runtime(RuntimeServiceError::OwnerMismatch)
        | CompositionError::Shell(ShellServiceError::OwnerMismatch) => {
            ErrorKind::ERROR_KIND_UNAUTHORIZED
        }
        CompositionError::Runtime(RuntimeServiceError::GenerationMismatch)
        | CompositionError::Shell(ShellServiceError::GenerationMismatch) => {
            ErrorKind::ERROR_KIND_GENERATION_MISMATCH
        }
        CompositionError::Runtime(RuntimeServiceError::DeadlineExpired)
        | CompositionError::Shell(ShellServiceError::DeadlineExpired) => {
            ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED
        }
        CompositionError::Runtime(RuntimeServiceError::NotFound)
        | CompositionError::Shell(ShellServiceError::NotFound) => ErrorKind::ERROR_KIND_NOT_FOUND,
        CompositionError::Runtime(RuntimeServiceError::Conflict)
        | CompositionError::Shell(
            ShellServiceError::AlreadyExists | ShellServiceError::AlreadyAttached,
        ) => ErrorKind::ERROR_KIND_CONFLICT,
        CompositionError::Runtime(
            RuntimeServiceError::Unavailable | RuntimeServiceError::WaylandUnavailable,
        )
        | CompositionError::Shell(ShellServiceError::RuntimeUnavailable)
        | CompositionError::Tty(TtyOneShotError::RuntimeUnavailable)
        | CompositionError::SessionUnavailable => ErrorKind::ERROR_KIND_UNAVAILABLE,
        CompositionError::Shell(ShellServiceError::ReservationExhausted)
        | CompositionError::Tty(TtyOneShotError::CapacityExceeded) => {
            ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED
        }
        CompositionError::RecoveryMismatch
        | CompositionError::TeardownFailed
        | CompositionError::InvalidLifecycle
        | CompositionError::ReconnectLimit
        | CompositionError::Runtime(RuntimeServiceError::BackendInvariant)
        | CompositionError::Tty(TtyOneShotError::TeardownFailed) => {
            ErrorKind::ERROR_KIND_INVARIANT_VIOLATION
        }
        _ => ErrorKind::ERROR_KIND_INVALID_REQUEST,
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

fn rpc_internal() -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(
        ttrpc::Code::INTERNAL,
        "runtime-service-failed".to_owned(),
    ))
}

#[derive(Clone, Copy)]
struct UnavailableResolver;

impl ConfiguredProcessResolver for UnavailableResolver {
    fn resolve(
        &mut self,
        _owner: &RuntimeOwner,
        _resource_id: &str,
        _request_digest: &[u8; 32],
    ) -> Result<ResolvedProcess, RuntimeServiceError> {
        Err(RuntimeServiceError::Unavailable)
    }
}

#[derive(Clone, Copy)]
struct UnavailableWayland;

impl WaylandControlPort for UnavailableWayland {
    fn open_display(
        &mut self,
        _owner: &RuntimeOwner,
        _process: &ResolvedProcess,
        _operation_id: &str,
    ) -> Result<WaylandDisplayLease, RuntimeServiceError> {
        Err(RuntimeServiceError::WaylandUnavailable)
    }

    fn close_display(&mut self, _lease: WaylandDisplayLease) {}
}

#[derive(Clone, Copy)]
struct UnavailableBackend;

impl SystemdUserRuntimePort for UnavailableBackend {
    fn ensure_scope(
        &mut self,
        _owner: &RuntimeOwner,
        _resource_id: &str,
        _operation_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        Err(RuntimeServiceError::Unavailable)
    }

    fn start_process(
        &mut self,
        _owner: &RuntimeOwner,
        _operation_id: &str,
        _process: &ResolvedProcess,
        _display: Option<&WaylandDisplayLease>,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        Err(RuntimeServiceError::Unavailable)
    }

    fn inspect_process(
        &mut self,
        _owner: &RuntimeOwner,
        _resource_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        Err(RuntimeServiceError::Unavailable)
    }

    fn adopt_process(
        &mut self,
        _owner: &RuntimeOwner,
        _resource_id: &str,
        _operation_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        Err(RuntimeServiceError::Unavailable)
    }

    fn stop_process(
        &mut self,
        _owner: &RuntimeOwner,
        _resource_id: &str,
        _operation_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        Err(RuntimeServiceError::Unavailable)
    }

    fn open_terminal(
        &mut self,
        _owner: &RuntimeOwner,
        _resource_id: &str,
        _stream_id: &str,
        _attachment: &AuthenticatedTerminalAttachment,
    ) -> Result<RuntimeResource, RuntimeServiceError> {
        Err(RuntimeServiceError::Unavailable)
    }

    fn cancel(&mut self, _owner: &RuntimeOwner, _request_id: [u8; 16]) -> CancelResult {
        CancelResult::UnknownRequest
    }
}

impl AuthenticatedSystemdUserRuntime for UnavailableBackend {
    fn create_shell_scope(
        &mut self,
        _owner: &ShellOwner,
        _resource_id: &str,
        _operation_id: &str,
    ) -> Result<VerifiedTransientScope, ShellServiceError> {
        Err(ShellServiceError::RuntimeUnavailable)
    }

    fn inspect_shell_scope(
        &mut self,
        _owner: &ShellOwner,
        _scope: &VerifiedTransientScope,
    ) -> Result<crate::shell_runtime::ScopeInspection, ShellServiceError> {
        Err(ShellServiceError::RuntimeUnavailable)
    }

    fn adopt_shell_scope(
        &mut self,
        _owner: &ShellOwner,
        _scope: &VerifiedTransientScope,
        _operation_id: &str,
    ) -> Result<crate::shell_runtime::ScopeInspection, ShellServiceError> {
        Err(ShellServiceError::RuntimeUnavailable)
    }

    fn kill_shell_scope(
        &mut self,
        _owner: &ShellOwner,
        _scope: &VerifiedTransientScope,
        _operation_id: &str,
    ) -> Result<crate::shell_runtime::ScopeInspection, ShellServiceError> {
        Err(ShellServiceError::RuntimeUnavailable)
    }

    fn cancel(&mut self, _owner: &ShellOwner, _request_id: [u8; 16]) -> ShellCancelOutcome {
        ShellCancelOutcome::UnknownRequest
    }
}

impl TtyOneShotRuntime for UnavailableBackend {
    fn start_transient_user_scope(
        &mut self,
        _owner: &RuntimeOwner,
        _request: &TtyOneShotRequest,
        _spec: &TtyOneShotSpec,
        _terminal: ValidatedTerminal,
    ) -> Result<TransientUserScope, TtyOneShotError> {
        Err(TtyOneShotError::RuntimeUnavailable)
    }

    fn teardown_transient_user_scope(
        &mut self,
        _owner: &RuntimeOwner,
        _scope: &TransientUserScope,
    ) -> Result<(), TtyOneShotError> {
        Err(TtyOneShotError::RuntimeUnavailable)
    }
}

impl fmt::Debug for RuntimeAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeAdapter(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v2_services::common::{IdentityScope, RequestMetadata};
    use d2b_session_unix::prearmed_seqpacket_pair;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::oneshot;

    fn request(generation: u64, desired: WireDesiredState) -> ServiceRequest {
        let now = now_unix_ms();
        let mut metadata = RequestMetadata::new();
        metadata.request_id = vec![1; 16];
        metadata.idempotency_key = vec![2; 32];
        metadata.issued_at_unix_ms = now;
        metadata.expires_at_unix_ms = now + 1_000;
        metadata.session_generation = generation;
        let mut scope = IdentityScope::new();
        scope.realm_id = "work".to_owned();
        scope.workload_id = "shell".to_owned();
        let mut request = ServiceRequest::new();
        request.metadata = MessageField::some(metadata);
        request.scope = MessageField::some(scope);
        request.resource_id = "session".to_owned();
        request.operation_id = "operation".to_owned();
        request.request_digest = vec![3; 32];
        request.desired_state = EnumOrUnknown::new(desired);
        request
    }

    fn response_error_kind(response: &ServiceResponse) -> ErrorKind {
        response.error.as_ref().unwrap().kind.enum_value().unwrap()
    }

    #[test]
    fn runtime_decoder_projects_a_complete_valid_request_without_debug_disclosure() {
        let mut wire = request(17, WireDesiredState::DESIRED_STATE_RUNNING);
        wire.stream_id = "terminal".to_owned();
        wire.attachment_indexes = vec![0];
        let decoded = decode_runtime_request(&wire).unwrap();

        assert_eq!(decoded.request_id, [1; 16]);
        assert_eq!(decoded.idempotency_key, Some([2; 32]));
        assert_eq!(decoded.session_generation, 17);
        assert!(decoded.realm_id == "work");
        assert!(decoded.workload_id == "shell");
        assert!(decoded.resource_id == "session");
        assert!(decoded.operation_id == "operation");
        assert_eq!(decoded.request_digest, Some([3; 32]));
        assert!(decoded.stream_id == "terminal");
        assert_eq!(decoded.attachment_indexes, [0]);
        assert_eq!(decoded.desired_state, DesiredState::Running);
        let debug = format!("{decoded:?}");
        for value in ["work", "shell", "session", "operation", "terminal"] {
            assert!(!debug.contains(value));
        }
    }

    #[test]
    fn shell_decoder_projects_every_method_and_action_without_debug_disclosure() {
        let cases = [
            (ShellMethod::Create, true),
            (ShellMethod::Attach, true),
            (ShellMethod::Detach, true),
            (ShellMethod::List, false),
            (ShellMethod::Inspect, false),
            (ShellMethod::Kill, true),
            (ShellMethod::Cancel, false),
        ];
        for (method, mutating) in cases {
            let mut wire = request(19, WireDesiredState::DESIRED_STATE_UNSPECIFIED);
            wire.stream_id = "terminal".to_owned();
            wire.attachment_indexes = vec![0];
            wire.page_size = 4096;
            let decoded = decode_shell_request(method, &wire).unwrap();

            assert_eq!(decoded.method, method);
            assert_eq!(decoded.method.mutating(), mutating);
            assert_eq!(decoded.request_id, [1; 16]);
            assert_eq!(decoded.idempotency_key, Some([2; 32]));
            assert_eq!(decoded.session_generation, 19);
            assert!(decoded.realm_id == "work");
            assert!(decoded.workload_id == "shell");
            assert!(decoded.resource_id == "session");
            assert!(decoded.operation_id == "operation");
            assert!(decoded.stream_id == "terminal");
            assert_eq!(decoded.attachment_indexes, [0]);
            assert_eq!(decoded.output_ring_bytes, 4096);
            let debug = format!("{decoded:?}");
            for value in ["work", "shell", "session", "operation", "terminal"] {
                assert!(!debug.contains(value));
            }
        }
    }

    #[test]
    fn decoders_reject_missing_metadata_scope_and_malformed_request_identity() {
        let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        wire.metadata = MessageField::none();
        assert_eq!(
            decode_runtime_request(&wire).unwrap_err(),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );
        assert_eq!(
            decode_shell_request(ShellMethod::Create, &wire).unwrap_err(),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );

        let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        wire.scope = MessageField::none();
        assert_eq!(
            decode_runtime_request(&wire).unwrap_err(),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );
        assert_eq!(
            decode_shell_request(ShellMethod::Create, &wire).unwrap_err(),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );

        for invalid in [Vec::new(), vec![0; 16], vec![1; 15], vec![1; 17]] {
            let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
            wire.metadata.as_mut().unwrap().request_id = invalid;
            assert_eq!(
                decode_runtime_request(&wire).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
            assert_eq!(
                decode_shell_request(ShellMethod::Create, &wire).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
        }
    }

    #[test]
    fn decoders_reject_malformed_optional_keys_and_runtime_digests() {
        for invalid in [vec![0; 32], vec![2; 31], vec![2; 33]] {
            let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
            wire.metadata.as_mut().unwrap().idempotency_key = invalid;
            assert_eq!(
                decode_runtime_request(&wire).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
            assert_eq!(
                decode_shell_request(ShellMethod::Create, &wire).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
        }

        for invalid in [vec![0; 32], vec![3; 31], vec![3; 33]] {
            let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
            wire.request_digest = invalid;
            assert_eq!(
                decode_runtime_request(&wire).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
        }

        let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        wire.metadata.as_mut().unwrap().idempotency_key.clear();
        wire.request_digest.clear();
        let decoded = decode_runtime_request(&wire).unwrap();
        assert_eq!(decoded.idempotency_key, None);
        assert_eq!(decoded.request_digest, None);
        assert_eq!(
            decode_shell_request(ShellMethod::List, &wire)
                .unwrap()
                .idempotency_key,
            None
        );
    }

    #[test]
    fn runtime_decoder_accepts_only_the_closed_desired_state_set() {
        let accepted = [
            (
                WireDesiredState::DESIRED_STATE_UNSPECIFIED,
                DesiredState::Unspecified,
            ),
            (
                WireDesiredState::DESIRED_STATE_PRESENT,
                DesiredState::Present,
            ),
            (
                WireDesiredState::DESIRED_STATE_RUNNING,
                DesiredState::Running,
            ),
            (
                WireDesiredState::DESIRED_STATE_STOPPED,
                DesiredState::Stopped,
            ),
            (
                WireDesiredState::DESIRED_STATE_ATTACHED,
                DesiredState::Attached,
            ),
        ];
        for (wire_state, decoded_state) in accepted {
            assert_eq!(
                decode_runtime_request(&request(7, wire_state))
                    .unwrap()
                    .desired_state,
                decoded_state
            );
        }

        for rejected in [
            WireDesiredState::DESIRED_STATE_ABSENT,
            WireDesiredState::DESIRED_STATE_ENABLED,
            WireDesiredState::DESIRED_STATE_DISABLED,
            WireDesiredState::DESIRED_STATE_OPEN,
            WireDesiredState::DESIRED_STATE_CLOSED,
            WireDesiredState::DESIRED_STATE_DETACHED,
        ] {
            assert_eq!(
                decode_runtime_request(&request(7, rejected)).unwrap_err(),
                ErrorKind::ERROR_KIND_INVALID_REQUEST
            );
        }
        let mut wire = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        wire.desired_state = EnumOrUnknown::from_i32(i32::MAX);
        assert_eq!(
            decode_runtime_request(&wire).unwrap_err(),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );
    }

    #[test]
    fn decoders_preserve_generation_and_attachment_bindings_for_service_validation() {
        let mut wire = request(0, WireDesiredState::DESIRED_STATE_ATTACHED);
        wire.attachment_indexes = vec![0, 0, u32::MAX];

        let runtime = decode_runtime_request(&wire).unwrap();
        assert_eq!(runtime.session_generation, 0);
        assert_eq!(runtime.attachment_indexes, [0, 0, u32::MAX]);

        let shell = decode_shell_request(ShellMethod::Attach, &wire).unwrap();
        assert_eq!(shell.session_generation, 0);
        assert_eq!(shell.attachment_indexes, [0, 0, u32::MAX]);
    }

    #[test]
    fn runtime_methods_have_closed_names_and_mutation_actions() {
        let cases = [
            (RuntimeMethod::EnsureScope, "EnsureScope", true),
            (RuntimeMethod::StartProcess, "StartProcess", true),
            (RuntimeMethod::InspectProcess, "InspectProcess", false),
            (RuntimeMethod::AdoptProcess, "AdoptProcess", true),
            (RuntimeMethod::StopProcess, "StopProcess", true),
            (RuntimeMethod::OpenTerminal, "OpenTerminal", true),
        ];
        for (method, name, mutating) in cases {
            assert_eq!(method.name(), name);
            assert_eq!(method.mutating(), mutating);
        }
    }

    #[test]
    fn adapters_close_malformed_identity_generation_and_attachment_requests() {
        if geteuid().is_root() {
            return;
        }
        let dispatch_runtime = |wire| {
            RuntimeAdapter(Arc::new(SessionServices::new(7)))
                .dispatch(RuntimeMethod::EnsureScope, wire)
                .unwrap()
        };
        let dispatch_shell = |wire| {
            ShellAdapter(Arc::new(SessionServices::new(7)))
                .dispatch(ShellMethod::Attach, wire)
                .unwrap()
        };

        let mut malformed_identity = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        malformed_identity.scope.as_mut().unwrap().realm_id.clear();
        assert_eq!(
            response_error_kind(&dispatch_runtime(malformed_identity)),
            ErrorKind::ERROR_KIND_UNAUTHORIZED
        );

        let zero_generation = request(0, WireDesiredState::DESIRED_STATE_PRESENT);
        assert_eq!(
            response_error_kind(&dispatch_runtime(zero_generation)),
            ErrorKind::ERROR_KIND_GENERATION_MISMATCH
        );

        let mut invalid_attachment = request(7, WireDesiredState::DESIRED_STATE_PRESENT);
        invalid_attachment.attachment_indexes = vec![0];
        assert_eq!(
            response_error_kind(&dispatch_runtime(invalid_attachment)),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );

        let mut shell_attachment = request(7, WireDesiredState::DESIRED_STATE_UNSPECIFIED);
        shell_attachment.attachment_indexes = vec![0, 1];
        assert_eq!(
            response_error_kind(&dispatch_shell(shell_attachment)),
            ErrorKind::ERROR_KIND_INVALID_REQUEST
        );
    }

    #[test]
    fn composition_errors_map_to_closed_wire_kinds() {
        let cases = [
            (
                CompositionError::OwnerMismatch,
                ErrorKind::ERROR_KIND_UNAUTHORIZED,
            ),
            (
                CompositionError::SessionUnavailable,
                ErrorKind::ERROR_KIND_UNAVAILABLE,
            ),
            (
                CompositionError::InvalidLifecycle,
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
            (
                CompositionError::ReconnectLimit,
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
            (
                CompositionError::RecoveryMismatch,
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
            (
                CompositionError::TeardownFailed,
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::Unauthenticated),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::ContractMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::OwnerMismatch),
                ErrorKind::ERROR_KIND_UNAUTHORIZED,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::GenerationMismatch),
                ErrorKind::ERROR_KIND_GENERATION_MISMATCH,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::DeadlineExpired),
                ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::InvalidRequest),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::InvalidResolvedProcess),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::ResolverMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::AttachmentMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::WaylandUnavailable),
                ErrorKind::ERROR_KIND_UNAVAILABLE,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::Unavailable),
                ErrorKind::ERROR_KIND_UNAVAILABLE,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::Conflict),
                ErrorKind::ERROR_KIND_CONFLICT,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::NotFound),
                ErrorKind::ERROR_KIND_NOT_FOUND,
            ),
            (
                CompositionError::Runtime(RuntimeServiceError::BackendInvariant),
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
            (
                CompositionError::Shell(ShellServiceError::Unauthenticated),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Shell(ShellServiceError::ContractMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Shell(ShellServiceError::OwnerMismatch),
                ErrorKind::ERROR_KIND_UNAUTHORIZED,
            ),
            (
                CompositionError::Shell(ShellServiceError::GenerationMismatch),
                ErrorKind::ERROR_KIND_GENERATION_MISMATCH,
            ),
            (
                CompositionError::Shell(ShellServiceError::DeadlineExpired),
                ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED,
            ),
            (
                CompositionError::Shell(ShellServiceError::InvalidRequest),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Shell(ShellServiceError::AttachmentMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Shell(ShellServiceError::ScopeOwnershipMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Shell(ShellServiceError::ReservationExhausted),
                ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED,
            ),
            (
                CompositionError::Shell(ShellServiceError::AlreadyExists),
                ErrorKind::ERROR_KIND_CONFLICT,
            ),
            (
                CompositionError::Shell(ShellServiceError::AlreadyAttached),
                ErrorKind::ERROR_KIND_CONFLICT,
            ),
            (
                CompositionError::Shell(ShellServiceError::NotFound),
                ErrorKind::ERROR_KIND_NOT_FOUND,
            ),
            (
                CompositionError::Shell(ShellServiceError::RuntimeUnavailable),
                ErrorKind::ERROR_KIND_UNAVAILABLE,
            ),
            (
                CompositionError::Tty(TtyOneShotError::InvalidPolicy),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::InvalidRequest),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::OwnerMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::AttachmentMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::ScopeOwnershipMismatch),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::CapacityExceeded),
                ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED,
            ),
            (
                CompositionError::Tty(TtyOneShotError::RequestConflict),
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
            ),
            (
                CompositionError::Tty(TtyOneShotError::RuntimeUnavailable),
                ErrorKind::ERROR_KIND_UNAVAILABLE,
            ),
            (
                CompositionError::Tty(TtyOneShotError::TeardownFailed),
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(composition_error_kind(error), expected);
        }
    }

    #[test]
    fn error_responses_use_closed_outcomes_and_retry_classes() {
        let cases = [
            (
                ErrorKind::ERROR_KIND_UNAVAILABLE,
                RetryClass::RETRY_CLASS_AFTER_OBSERVATION,
            ),
            (
                ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED,
                RetryClass::RETRY_CLASS_SAME_OPERATION,
            ),
            (
                ErrorKind::ERROR_KIND_INVALID_REQUEST,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_UNAUTHENTICATED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_UNAUTHORIZED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_NOT_FOUND,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_CONFLICT,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_CAPABILITY_DENIED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_CANCELLED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_GENERATION_MISMATCH,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_INVARIANT_VIOLATION,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_INTERNAL,
                RetryClass::RETRY_CLASS_NEVER,
            ),
            (
                ErrorKind::ERROR_KIND_UNSPECIFIED,
                RetryClass::RETRY_CLASS_NEVER,
            ),
        ];
        for (kind, retry) in cases {
            let response = error_response(kind);
            assert_eq!(
                response.outcome.enum_value().unwrap(),
                Outcome::OUTCOME_FAILED
            );
            let error = response.error.as_ref().unwrap();
            assert_eq!(error.kind.enum_value().unwrap(), kind);
            assert_eq!(error.retry.enum_value().unwrap(), retry);
            assert!(error.correlation_id.is_empty());
        }
    }

    #[test]
    fn endpoint_policy_is_same_uid_and_runtime_specific() {
        assert!(endpoint_policy(0, 0, 1).is_none());
        let uid = geteuid().as_raw();
        if uid == 0 {
            return;
        }
        let policy = endpoint_policy(uid, nix::unistd::getegid().as_raw(), 9).unwrap();
        assert_eq!(policy.purpose, EndpointPurpose::RuntimeSystemdUser);
        assert_eq!(policy.responder_role, EndpointRole::RuntimeSystemdUserAgent);
        assert_eq!(policy.reconnect_generation, 9);
        assert_eq!(policy.attachment_policy.max_per_packet, 1);
    }

    #[tokio::test]
    async fn socket_peer_admission_requires_current_non_root_uid() {
        let (left, _right) = prearmed_seqpacket_pair().unwrap();
        let socket = SeqpacketSocket::from_owned(left).unwrap();
        let peer = socket.acceptor_peer_credentials().unwrap();
        assert_eq!(peer.uid().as_raw(), geteuid().as_raw());
        assert_eq!(
            peer_is_authorized(
                peer.uid().as_raw(),
                geteuid().as_raw(),
                &ControllerAllowlist::empty()
            ),
            geteuid().as_raw() != 0
        );
    }

    #[test]
    fn peer_admission_accepts_the_exact_allowlisted_controller() {
        let allowlist = ControllerAllowlist::resolve(
            controller_allowlist_document(&[("alice", &[1234])]),
            "alice",
        )
        .unwrap();
        assert!(peer_is_authorized(1234, 1000, &allowlist));
    }

    #[test]
    fn peer_admission_denies_an_unrelated_controller() {
        let allowlist = ControllerAllowlist::resolve(
            controller_allowlist_document(&[("alice", &[1234])]),
            "alice",
        )
        .unwrap();
        // 1300 is a real, distinct controller uid but was never granted to
        // this requester.
        assert!(!peer_is_authorized(1300, 1000, &allowlist));
    }

    #[test]
    fn peer_admission_denies_an_unrelated_users_controller() {
        // The document authorizes 1234 only for bob, not for alice.
        let allowlist = ControllerAllowlist::resolve(
            controller_allowlist_document(&[("bob", &[1234])]),
            "alice",
        )
        .unwrap();
        assert!(!peer_is_authorized(1234, 1000, &allowlist));
    }

    #[test]
    fn peer_admission_denies_root_regardless_of_allowlist_or_own_uid() {
        // A document that ever tried to authorize uid 0 fails closed at
        // parse time (see controller_allowlist::tests), so uid 0 can never
        // legitimately appear in a resolved allowlist. `peer_is_authorized`
        // additionally defends in depth against uid 0 on either side.
        let empty = ControllerAllowlist::empty();
        assert!(!peer_is_authorized(0, 1000, &empty));
        assert!(!peer_is_authorized(1000, 0, &empty));
    }

    #[test]
    fn peer_admission_still_accepts_the_same_uid_direct_path() {
        let allowlist = ControllerAllowlist::empty();
        assert!(peer_is_authorized(1000, 1000, &allowlist));
    }

    #[test]
    fn malformed_allowlist_document_never_authorizes_a_foreign_uid() {
        for bytes in [
            &b"not-json"[..],
            br#"{"schemaVersion":1,"entries":[{"user":"alice","controllerUids":[0]}]}"#,
            br#"{"schemaVersion":1,"entries":[{"user":"alice","controllerUids":[1300,1234]}]}"#,
        ] {
            assert!(ControllerAllowlist::resolve(bytes, "alice").is_err());
        }
    }

    #[test]
    fn admission_never_selects_or_changes_the_execution_uid() {
        // `peer_is_authorized` is a pure predicate: it takes both uids and
        // an allowlist and returns a bool. There is no code path by which an
        // authorized peer uid can become the uid the helper executes
        // requests as -- that identity is fixed once, in `run`, from the
        // process's own real/effective uid, before any peer is ever
        // accepted.
        let allowlist = ControllerAllowlist::resolve(
            controller_allowlist_document(&[("alice", &[1234])]),
            "alice",
        )
        .unwrap();
        assert!(peer_is_authorized(1234, 1000, &allowlist));
        // Swapping which side is "own" vs "peer" must not also authorize --
        // authorization is not symmetric execution-identity selection.
        assert!(!peer_is_authorized(1000, 1234, &allowlist));
    }

    fn controller_allowlist_document(entries: &[(&str, &[u32])]) -> &'static [u8] {
        // Only literal fixtures are exercised through this helper; leaking a
        // small boxed buffer keeps call sites terse without unsafe code.
        let entries: Vec<serde_json::Value> = entries
            .iter()
            .map(|(user, uids)| serde_json::json!({ "user": user, "controllerUids": uids }))
            .collect();
        let document = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "entries": entries,
        }))
        .unwrap();
        Box::leak(document.into_boxed_slice())
    }

    #[test]
    fn registry_dispatches_other_services_when_tty_is_unavailable() {
        if geteuid().is_root() {
            return;
        }
        let generation = 7;
        let registry = service_registry(generation);
        assert!(registry.contains_key("d2b.runtime.systemd-user.v2.RuntimeSystemdUserService"));
        assert!(registry.contains_key("d2b.shell.v2.ShellService"));
        assert!(registry.contains_key("d2b.tty.v2.TtyService"));

        let runtime = RuntimeAdapter(Arc::new(SessionServices::new(generation)));
        let unavailable = runtime
            .dispatch(
                RuntimeMethod::EnsureScope,
                request(generation, WireDesiredState::DESIRED_STATE_PRESENT),
            )
            .unwrap();
        assert_eq!(
            unavailable
                .error
                .as_ref()
                .unwrap()
                .kind
                .enum_value()
                .unwrap(),
            ErrorKind::ERROR_KIND_UNAVAILABLE
        );

        let shell = ShellAdapter(Arc::clone(&runtime.0));
        let listed = shell
            .dispatch(
                ShellMethod::List,
                request(generation, WireDesiredState::DESIRED_STATE_UNSPECIFIED),
            )
            .unwrap();
        assert_eq!(
            listed.outcome.enum_value().unwrap(),
            Outcome::OUTCOME_SUCCEEDED
        );
    }

    #[tokio::test]
    async fn unavailable_tty_method_returns_a_typed_result() {
        let context = TtrpcContext {
            mh: ttrpc::MessageHeader::default(),
            metadata: HashMap::new(),
            timeout_nano: 0,
        };
        let response = TtyTtrpc::enter_raw_mode(&TtyUnavailable, &context, ServiceRequest::new())
            .await
            .unwrap();
        assert_eq!(
            response.error.as_ref().unwrap().kind.enum_value().unwrap(),
            ErrorKind::ERROR_KIND_UNAVAILABLE
        );
    }

    struct FakeActivatedListener {
        accepted: Mutex<Option<SeqpacketSocket>>,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ActivatedListener for FakeActivatedListener {
        async fn accept(&self) -> Result<SeqpacketSocket, ServerError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let accepted = self
                .accepted
                .lock()
                .map_err(|_| ServerError::Activation)?
                .take();
            match accepted {
                Some(socket) => Ok(socket),
                None => std::future::pending().await,
            }
        }
    }

    #[tokio::test]
    async fn activation_loop_stops_and_closes_active_sessions() {
        let (left, right) = prearmed_seqpacket_pair().unwrap();
        let listener = FakeActivatedListener {
            accepted: Mutex::new(Some(SeqpacketSocket::from_owned(left).unwrap())),
            calls: AtomicUsize::new(0),
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let shutdown = async {
            shutdown_rx.await.map_err(|_| ServerError::Signal)?;
            Ok(())
        };
        let task = tokio::spawn(async move {
            serve_until_shutdown(&listener, 1, &ControllerAllowlist::empty(), shutdown)
                .await
                .map(|()| listener.calls.load(Ordering::Relaxed))
        });
        tokio::task::yield_now().await;
        shutdown_tx.send(()).unwrap();
        assert!(task.await.unwrap().unwrap() >= 1);
        drop(right);
    }

    #[test]
    fn debug_and_errors_do_not_expose_request_identity() {
        let canary = "private-request-canary";
        let adapter = RuntimeAdapter(Arc::new(SessionServices::new(1)));
        let mut wire = request(1, WireDesiredState::DESIRED_STATE_PRESENT);
        wire.resource_id = canary.to_owned();
        let response = adapter.dispatch(RuntimeMethod::EnsureScope, wire).unwrap();
        assert!(!format!("{response:?}").contains(canary));
        assert!(!format!("{adapter:?}").contains(canary));
    }
}
