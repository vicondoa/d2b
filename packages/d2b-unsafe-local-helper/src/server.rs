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
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Activation => "socket-activation-failed",
            Self::InvalidIdentity => "runtime-identity-invalid",
            Self::Generation => "runtime-generation-unavailable",
            Self::Signal => "shutdown-signal-unavailable",
        })
    }
}

impl std::error::Error for ServerError {}

pub async fn run() -> Result<(), ServerError> {
    let uid = geteuid().as_raw();
    if uid == 0 || uid != nix::unistd::getuid().as_raw() {
        return Err(ServerError::InvalidIdentity);
    }
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
    serve_until_shutdown(&listeners, generation, shutdown).await
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
                sessions.spawn(async move {
                    let _ = serve_socket(socket, generation).await;
                });
            }
            completed = sessions.join_next(), if !sessions.is_empty() => {
                let _ = completed;
            }
        }
    }
}

async fn serve_socket(socket: SeqpacketSocket, generation: u64) -> Result<(), ()> {
    let peer = socket.acceptor_peer_credentials().map_err(|_| ())?;
    let uid = geteuid().as_raw();
    if uid == 0 || peer.uid().as_raw() != uid {
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
            peer.uid().as_raw() != 0 && peer.uid().as_raw() == geteuid().as_raw(),
            geteuid().as_raw() != 0
        );
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
            serve_until_shutdown(&listener, 1, shutdown)
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
