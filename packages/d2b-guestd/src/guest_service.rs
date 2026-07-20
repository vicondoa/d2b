//! Typed `d2b.guest.v2` service and named-stream ownership.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU16, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{LimitProfile, RequestId as SessionRequestId},
    v2_services::{
        MIN_NAMED_STREAM_ID, StrictWireMessage,
        activation_ttrpc::create_activation_service,
        common::{self, Outcome},
        guest,
        guest_contract::{
            validate_guest_cancel_response_for_request, validate_guest_exec_response_for_request,
            validate_guest_inspect_response_for_request,
            validate_guest_open_exec_retained_log_response_for_request,
            validate_guest_open_shell_response_for_request,
            validate_guest_session_response_for_bootstrap,
            validate_guest_session_response_for_reconnect,
            validate_guest_shutdown_response_for_request,
            validate_terminal_open_response_for_guest_context,
        },
        guest_ttrpc::{GuestService, create_guest_service},
        server_stream_name, terminal,
    },
};
use d2b_session::{Cancellation, ComponentSessionDriver, StreamEvent, StreamId};
use futures::stream;
use protobuf::{Enum, EnumOrUnknown, Message, MessageField};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

use crate::{
    activation::ActivationRuntime,
    activation_service::ActivationServiceV2,
    request_tracker::{
        GuestRequestAdmission, GuestRequestTicket, GuestRequestTracker, RequestAdmissionError,
    },
    service_v2::{EstablishedGuestSession, GuestSessionError, GuestSessionPhase},
};

const STREAM_ROUTE_DEPTH: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestStreamMethod {
    Exec,
    RetainedLog,
    Shell,
    FileTransfer,
    SecurityKey,
}

#[derive(Clone)]
pub struct GuestStreamBinding {
    pub generation: u64,
    pub request_id: Vec<u8>,
    pub operation_id: String,
    pub resource_handle: String,
    pub owner_key: Vec<u8>,
    pub scope: common::IdentityScope,
    pub cancellation: Cancellation,
}

impl fmt::Debug for GuestStreamBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestStreamBinding")
            .field("generation", &"<redacted>")
            .field("request_id", &"<redacted>")
            .field("operation_id", &"<redacted>")
            .field("resource_handle", &"<redacted>")
            .field("owner_key", &"<redacted>")
            .field("scope", &"<redacted>")
            .field("cancellation", &self.cancellation)
            .finish()
    }
}

pub enum GuestStreamInput {
    Message(Vec<u8>),
    RemoteClosed,
    Reset,
    Disconnected,
}

impl fmt::Debug for GuestStreamInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(bytes) => formatter
                .debug_struct("GuestStreamInput::Message")
                .field("bytes", &"<redacted>")
                .field("len", &bytes.len())
                .finish(),
            Self::RemoteClosed => formatter.write_str("GuestStreamInput::RemoteClosed"),
            Self::Reset => formatter.write_str("GuestStreamInput::Reset"),
            Self::Disconnected => formatter.write_str("GuestStreamInput::Disconnected"),
        }
    }
}

pub struct GuestStream {
    stream: StreamId,
    session: Arc<dyn ComponentSessionDriver>,
    routes: Arc<Mutex<BTreeMap<StreamId, mpsc::Sender<GuestStreamInput>>>>,
    receiver: mpsc::Receiver<GuestStreamInput>,
    finished: bool,
    request: Option<RequestLease>,
}

struct RequestLease {
    tracker: Arc<GuestRequestTracker>,
    ticket: GuestRequestTicket,
}

impl Drop for RequestLease {
    fn drop(&mut self) {
        let tracker = Arc::clone(&self.tracker);
        let ticket = self.ticket.clone();
        tokio::spawn(async move {
            tracker.finish(&ticket).await;
        });
    }
}

impl fmt::Debug for GuestStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GuestStream(<redacted>)")
    }
}

impl GuestStream {
    fn attach_request(&mut self, tracker: Arc<GuestRequestTracker>, ticket: GuestRequestTicket) {
        self.request = Some(RequestLease { tracker, ticket });
    }
    pub async fn receive(&mut self) -> GuestStreamInput {
        self.receiver
            .recv()
            .await
            .unwrap_or(GuestStreamInput::Disconnected)
    }

    pub async fn consume(&self, bytes: usize) -> Result<(), GuestSessionError> {
        let bytes = u32::try_from(bytes).map_err(|_| GuestSessionError::Service)?;
        self.session
            .grant_named_stream_credit(self.stream, bytes)
            .await
            .map_err(GuestSessionError::from)
    }

    pub async fn send<M: Message>(&self, message: &M) -> Result<(), GuestSessionError> {
        let bytes = message
            .write_to_bytes()
            .map_err(|_| GuestSessionError::Service)?;
        self.session
            .send_named_stream(self.stream, bytes)
            .await
            .map_err(GuestSessionError::from)
    }

    pub async fn close(&mut self) -> Result<(), GuestSessionError> {
        self.unregister();
        let result = self
            .session
            .close_named_stream(self.stream)
            .await
            .map_err(GuestSessionError::from);
        self.finished = true;
        result
    }

    pub async fn reset(&mut self) -> Result<(), GuestSessionError> {
        self.unregister();
        let result = self
            .session
            .reset_named_stream(self.stream)
            .await
            .map_err(GuestSessionError::from);
        self.finished = true;
        result
    }

    fn unregister(&self) {
        self.routes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.stream);
    }
}

impl Drop for GuestStream {
    fn drop(&mut self) {
        self.unregister();
        if !self.finished {
            let session = Arc::clone(&self.session);
            let stream = self.stream;
            tokio::spawn(async move {
                let _ = session.reset_named_stream(stream).await;
            });
        }
    }
}

struct GuestStreamHub {
    session: Arc<dyn ComponentSessionDriver>,
    routes: Arc<Mutex<BTreeMap<StreamId, mpsc::Sender<GuestStreamInput>>>>,
    next_stream: AtomicU16,
}

impl GuestStreamHub {
    fn new(session: Arc<dyn ComponentSessionDriver>) -> Self {
        let routes = Arc::new(Mutex::new(BTreeMap::<
            StreamId,
            mpsc::Sender<GuestStreamInput>,
        >::new()));
        let receive_session = Arc::clone(&session);
        let receive_routes = Arc::clone(&routes);
        tokio::spawn(async move {
            loop {
                let event = match receive_session.receive_named_stream().await {
                    Ok(event) => event,
                    Err(_) => {
                        let senders = {
                            let mut routes = receive_routes
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            std::mem::take(&mut *routes)
                                .into_values()
                                .collect::<Vec<_>>()
                        };
                        for sender in senders {
                            let _ = sender.try_send(GuestStreamInput::Disconnected);
                        }
                        return;
                    }
                };
                let (stream, input) = match event {
                    StreamEvent::Data { stream, bytes } => {
                        (stream, GuestStreamInput::Message(bytes))
                    }
                    StreamEvent::RemoteClosed { stream } => {
                        (stream, GuestStreamInput::RemoteClosed)
                    }
                    StreamEvent::Reset { stream } => (stream, GuestStreamInput::Reset),
                };
                let sender = receive_routes
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&stream)
                    .cloned();
                let Some(sender) = sender else {
                    let _ = receive_session.reset_named_stream(stream).await;
                    continue;
                };
                if sender.try_send(input).is_err() {
                    receive_routes
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .remove(&stream);
                    let _ = receive_session.reset_named_stream(stream).await;
                }
            }
        });
        Self {
            session,
            routes,
            next_stream: AtomicU16::new(MIN_NAMED_STREAM_ID),
        }
    }

    async fn reserve(&self) -> Result<(String, GuestStream), GuestSessionError> {
        let stream_number = self
            .next_stream
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .map_err(|_| GuestSessionError::Service)?;
        let stream = StreamId::new(stream_number).map_err(GuestSessionError::from)?;
        let (sender, receiver) = mpsc::channel(STREAM_ROUTE_DEPTH);
        {
            let mut routes = self
                .routes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if routes.insert(stream, sender).is_some() {
                return Err(GuestSessionError::Service);
            }
        }
        let credit = LimitProfile::local_default().named_stream_queue_bytes;
        if let Err(error) = self.session.open_named_stream(stream, credit, credit).await {
            self.routes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&stream);
            return Err(GuestSessionError::from(error));
        }
        Ok((
            server_stream_name(stream_number).map_err(|_| GuestSessionError::Service)?,
            GuestStream {
                stream,
                session: Arc::clone(&self.session),
                routes: Arc::clone(&self.routes),
                receiver,
                finished: false,
                request: None,
            },
        ))
    }
}

#[async_trait]
pub trait GuestOperationHandler: Send + Sync {
    fn capabilities(&self) -> Vec<guest::GuestCapability>;
    fn scope_authorized(&self, scope: &common::IdentityScope) -> bool;
    fn stream_ready(&self, method: GuestStreamMethod) -> bool;

    async fn serve_exec(
        &self,
        _request: guest::GuestExecRequest,
        _response: terminal::TerminalOpenResponse,
        _binding: GuestStreamBinding,
        _stream: GuestStream,
    ) -> Result<(), GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn cancel_exec(
        &self,
        _request: guest::GuestCancelExecRequest,
        _binding: GuestStreamBinding,
    ) -> Result<guest::GuestCancelExecResponse, GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn inspect_exec(
        &self,
        _request: guest::GuestInspectExecRequest,
        _binding: GuestStreamBinding,
    ) -> Result<guest::GuestInspectExecResponse, GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn serve_retained_log(
        &self,
        _request: guest::GuestOpenExecRetainedLogRequest,
        _response: terminal::TerminalOpenResponse,
        _binding: GuestStreamBinding,
        _stream: GuestStream,
    ) -> Result<terminal::TerminalRetainedLogRange, GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn serve_shell(
        &self,
        _request: guest::GuestOpenShellRequest,
        _response: terminal::TerminalOpenResponse,
        _binding: GuestStreamBinding,
        _stream: GuestStream,
    ) -> Result<(), GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn serve_file_transfer(
        &self,
        _request: guest::GuestFileTransferRequest,
        _response: terminal::TerminalOpenResponse,
        _binding: GuestStreamBinding,
        _stream: GuestStream,
    ) -> Result<(), GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn serve_security_key(
        &self,
        _request: guest::GuestSecurityKeyRequest,
        _response: terminal::TerminalOpenResponse,
        _binding: GuestStreamBinding,
        _stream: GuestStream,
    ) -> Result<(), GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn shutdown(
        &self,
        _request: guest::GuestShutdownRequest,
    ) -> Result<guest::GuestShutdownResponse, GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    fn disconnect(&self, _: &[u8]) {}
}

pub struct RejectingGuestOperations;

#[async_trait]
impl GuestOperationHandler for RejectingGuestOperations {
    fn capabilities(&self) -> Vec<guest::GuestCapability> {
        Vec::new()
    }

    fn scope_authorized(&self, _: &common::IdentityScope) -> bool {
        false
    }

    fn stream_ready(&self, _: GuestStreamMethod) -> bool {
        false
    }

    async fn serve_exec(
        &self,
        _: guest::GuestExecRequest,
        _: terminal::TerminalOpenResponse,
        _: GuestStreamBinding,
        _: GuestStream,
    ) -> Result<(), GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn cancel_exec(
        &self,
        _: guest::GuestCancelExecRequest,
        _: GuestStreamBinding,
    ) -> Result<guest::GuestCancelExecResponse, GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn inspect_exec(
        &self,
        _: guest::GuestInspectExecRequest,
        _: GuestStreamBinding,
    ) -> Result<guest::GuestInspectExecResponse, GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn serve_retained_log(
        &self,
        _: guest::GuestOpenExecRetainedLogRequest,
        _: terminal::TerminalOpenResponse,
        _: GuestStreamBinding,
        _: GuestStream,
    ) -> Result<terminal::TerminalRetainedLogRange, GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn serve_shell(
        &self,
        _: guest::GuestOpenShellRequest,
        _: terminal::TerminalOpenResponse,
        _: GuestStreamBinding,
        _: GuestStream,
    ) -> Result<(), GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn serve_file_transfer(
        &self,
        _: guest::GuestFileTransferRequest,
        _: terminal::TerminalOpenResponse,
        _: GuestStreamBinding,
        _: GuestStream,
    ) -> Result<(), GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn serve_security_key(
        &self,
        _: guest::GuestSecurityKeyRequest,
        _: terminal::TerminalOpenResponse,
        _: GuestStreamBinding,
        _: GuestStream,
    ) -> Result<(), GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    async fn shutdown(
        &self,
        _: guest::GuestShutdownRequest,
    ) -> Result<guest::GuestShutdownResponse, GuestSessionError> {
        Err(GuestSessionError::Service)
    }

    fn disconnect(&self, _: &[u8]) {}
}

pub struct GuestServiceV2 {
    handler: Arc<dyn GuestOperationHandler>,
    session: Arc<dyn ComponentSessionDriver>,
    bootstrap_commit: Option<Arc<crate::service_v2::BootstrapCommit>>,
    identity: crate::service_v2::SessionIdentityEvidence,
    owner_key: Vec<u8>,
    hub: GuestStreamHub,
    next_resource: AtomicU64,
    authorization: Arc<AtomicU8>,
    requests: Arc<GuestRequestTracker>,
    clock: Arc<dyn GuestWallClock>,
    authorization_gate: Arc<dyn GuestAuthorizationGate>,
}

pub trait GuestWallClock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
}

#[derive(Clone)]
pub(crate) struct GuestServiceAccess {
    pub(crate) session: Arc<dyn ComponentSessionDriver>,
    pub(crate) authorization: Arc<AtomicU8>,
    pub(crate) requests: Arc<GuestRequestTracker>,
    pub(crate) clock: Arc<dyn GuestWallClock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestAuthorizationPhase {
    Bootstrap,
    Reconnect,
}

#[async_trait]
pub trait GuestAuthorizationGate: Send + Sync {
    async fn before_point_of_no_return(&self, phase: GuestAuthorizationPhase);
}

struct ImmediateAuthorizationGate;

#[async_trait]
impl GuestAuthorizationGate for ImmediateAuthorizationGate {
    async fn before_point_of_no_return(&self, _: GuestAuthorizationPhase) {}
}

struct SystemGuestWallClock;

impl GuestWallClock for SystemGuestWallClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

enum TypedAdmission<T> {
    New(GuestRequestTicket),
    Replay(T),
}

impl fmt::Debug for GuestServiceV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestServiceV2")
            .field("generation", &"<redacted>")
            .field("identity", &"<redacted>")
            .field("owner", &"<redacted>")
            .finish()
    }
}

impl GuestServiceV2 {
    pub fn with_handler(
        handler: Arc<dyn GuestOperationHandler>,
        session: EstablishedGuestSession,
    ) -> Self {
        Self::with_handler_clock_and_gate(
            handler,
            session,
            Arc::new(SystemGuestWallClock),
            Arc::new(ImmediateAuthorizationGate),
        )
    }

    pub fn with_handler_and_clock(
        handler: Arc<dyn GuestOperationHandler>,
        session: EstablishedGuestSession,
        clock: Arc<dyn GuestWallClock>,
    ) -> Self {
        Self::with_handler_clock_and_gate(
            handler,
            session,
            clock,
            Arc::new(ImmediateAuthorizationGate),
        )
    }

    pub fn with_handler_clock_and_gate(
        handler: Arc<dyn GuestOperationHandler>,
        session: EstablishedGuestSession,
        clock: Arc<dyn GuestWallClock>,
        authorization_gate: Arc<dyn GuestAuthorizationGate>,
    ) -> Self {
        let hub = GuestStreamHub::new(Arc::clone(&session.driver));
        let requests = Arc::new(
            GuestRequestTracker::new(session.driver.generation(), Arc::clone(&session.driver))
                .expect("established session generation"),
        );
        Self {
            handler,
            session: session.driver,
            bootstrap_commit: session.bootstrap_commit,
            identity: session.identity,
            owner_key: session.owner_key.to_vec(),
            hub,
            next_resource: AtomicU64::new(1),
            authorization: Arc::new(AtomicU8::new(0)),
            requests,
            clock,
            authorization_gate,
        }
    }

    pub fn fail_closed(session: EstablishedGuestSession) -> Self {
        Self::with_handler(Arc::new(RejectingGuestOperations), session)
    }

    pub(crate) fn access(&self) -> GuestServiceAccess {
        GuestServiceAccess {
            session: Arc::clone(&self.session),
            authorization: Arc::clone(&self.authorization),
            requests: Arc::clone(&self.requests),
            clock: Arc::clone(&self.clock),
        }
    }

    fn capabilities(&self) -> Vec<guest::GuestCapability> {
        let mut capabilities = self.handler.capabilities();
        capabilities.sort_by_key(|capability| capability.value());
        capabilities.dedup();
        capabilities
    }

    fn require_authorized(&self) -> ttrpc::Result<()> {
        if self.authorization.load(Ordering::Acquire) == 1 {
            Ok(())
        } else {
            Err(unauthorized_rpc_error())
        }
    }

    fn confirm_authorized(&self) -> ttrpc::Result<()> {
        self.authorization
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| service_rpc_error_for("guest-session-already-confirmed"))
    }

    async fn admit_typed<T: Message>(
        &self,
        context: &ttrpc::r#async::TtrpcContext,
        metadata: &common::RequestMetadata,
        method: &'static str,
        request: &impl Message,
        request_digest: &[u8],
        requires_idempotency: bool,
    ) -> ttrpc::Result<TypedAdmission<T>> {
        let peer_timeout = u64::try_from(context.timeout_nano)
            .ok()
            .filter(|timeout| *timeout != 0);
        let encoded_request = request
            .write_to_bytes()
            .map_err(|_| invalid_request_rpc_error())?;
        let mut binding = Sha256::new();
        binding.update(b"d2b-guest-request-replay-v2\0");
        binding.update(method.as_bytes());
        binding.update(request_digest);
        binding.update(encoded_request);
        let replay_binding = binding.finalize();
        match self
            .requests
            .admit(
                metadata,
                method,
                &replay_binding,
                requires_idempotency,
                self.clock.now_unix_ms(),
                peer_timeout,
            )
            .await
            .map_err(map_admission_error)?
        {
            GuestRequestAdmission::New(ticket) => Ok(TypedAdmission::New(ticket)),
            GuestRequestAdmission::Replay(bytes) => T::parse_from_bytes(&bytes)
                .map(TypedAdmission::Replay)
                .map_err(|_| service_rpc_error_for("guest-service-replay-invalid")),
        }
    }

    async fn complete_typed<T: Message>(
        &self,
        ticket: &GuestRequestTicket,
        response: &T,
        keep_active: bool,
    ) -> ttrpc::Result<()> {
        let bytes = response
            .write_to_bytes()
            .map_err(|_| service_rpc_error_for("guest-service-response-encode-failed"))?;
        self.requests
            .complete_response(ticket, bytes, keep_active)
            .await;
        Ok(())
    }

    async fn fail_ticket(&self, ticket: &GuestRequestTicket) {
        self.requests.fail(ticket).await;
    }

    fn authorize_scope(&self, scope: &common::IdentityScope) -> ttrpc::Result<()> {
        if self.handler.scope_authorized(scope) {
            Ok(())
        } else {
            Err(unauthorized_rpc_error())
        }
    }

    fn require_generation(&self, generation: u64) -> ttrpc::Result<()> {
        if generation == self.session.generation() {
            Ok(())
        } else {
            Err(invalid_request_rpc_error())
        }
    }

    fn require_capability(&self, capability: guest::GuestCapability) -> ttrpc::Result<()> {
        if self.capabilities().contains(&capability) {
            Ok(())
        } else {
            Err(service_rpc_error())
        }
    }

    fn resource_handle(&self, request_id: &[u8]) -> String {
        let ordinal = self.next_resource.fetch_add(1, Ordering::AcqRel);
        let mut digest = Sha256::new();
        digest.update(b"d2b-guest-resource-v2\0");
        digest.update(&self.owner_key);
        digest.update(request_id);
        digest.update(ordinal.to_be_bytes());
        let encoded = crate::service_v2::hex(&digest.finalize()[..16]);
        format!("a{}", &encoded[1..])
    }

    fn terminal_binding(
        &self,
        request: &terminal::TerminalOpenRequest,
        resource_handle: String,
        ticket: &GuestRequestTicket,
    ) -> ttrpc::Result<GuestStreamBinding> {
        let metadata = request
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let scope = request
            .scope
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        self.require_generation(metadata.session_generation)?;
        self.authorize_scope(scope)?;
        Ok(GuestStreamBinding {
            generation: metadata.session_generation,
            request_id: metadata.request_id.clone(),
            operation_id: request.operation_id.clone(),
            resource_handle,
            owner_key: self.owner_key.clone(),
            scope: scope.clone(),
            cancellation: ticket.cancellation.clone(),
        })
    }

    fn context_binding(
        &self,
        context: &guest::GuestOperationContext,
        resource_handle: String,
        ticket: &GuestRequestTicket,
    ) -> ttrpc::Result<GuestStreamBinding> {
        let metadata = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let scope = context
            .scope
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        self.require_generation(metadata.session_generation)?;
        self.authorize_scope(scope)?;
        Ok(GuestStreamBinding {
            generation: metadata.session_generation,
            request_id: metadata.request_id.clone(),
            operation_id: context.operation_id.clone(),
            resource_handle,
            owner_key: self.owner_key.clone(),
            scope: scope.clone(),
            cancellation: ticket.cancellation.clone(),
        })
    }

    fn accepted_response(
        binding: &GuestStreamBinding,
        stream_id: String,
    ) -> terminal::TerminalOpenResponse {
        terminal::TerminalOpenResponse {
            outcome: EnumOrUnknown::new(Outcome::OUTCOME_ACCEPTED),
            operation_id: binding.operation_id.clone(),
            stream_id,
            session_generation: binding.generation,
            request_id: binding.request_id.clone(),
            resource_handle: binding.resource_handle.clone(),
            ..Default::default()
        }
    }
}

#[async_trait]
impl GuestService for GuestServiceV2 {
    async fn bootstrap(
        &self,
        rpc_context: &ttrpc::r#async::TtrpcContext,
        request: guest::GuestBootstrapRequest,
    ) -> ttrpc::Result<guest::GuestSessionResponse> {
        request
            .validate_wire(true)
            .map_err(|_| invalid_request_rpc_error())?;
        let context = request
            .context
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let metadata = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let scope = context
            .scope
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let ticket = match self
            .admit_typed::<guest::GuestSessionResponse>(
                rpc_context,
                metadata,
                "Bootstrap",
                &request,
                &context.request_digest,
                true,
            )
            .await?
        {
            TypedAdmission::Replay(response) => return Ok(response),
            TypedAdmission::New(ticket) => ticket,
        };
        let result = async {
            self.require_generation(metadata.session_generation)?;
            self.authorize_scope(scope)?;
            if self.authorization.load(Ordering::Acquire) != 0
                || self.identity.phase != GuestSessionPhase::Bootstrap
                || request.expected_generation != self.session.generation()
                || request.expected_parent_static_public_key_digest
                    != self.identity.parent_static_public_key_digest
            {
                return Err(unauthorized_rpc_error());
            }
            let capabilities = self.capabilities();
            if request.requested_capabilities.iter().any(|requested| {
                requested
                    .enum_value()
                    .ok()
                    .is_none_or(|requested| !capabilities.contains(&requested))
            }) {
                return Err(service_rpc_error());
            }
            let commit = self
                .bootstrap_commit
                .as_ref()
                .ok_or_else(|| service_rpc_error_for("bootstrap-state-unavailable"))?;
            let preview_public = commit
                .preview_public_key()
                .map_err(|_| service_rpc_error_for("bootstrap-state-unavailable"))?;
            if Sha256::digest(preview_public).as_slice()
                != self.identity.guest_static_public_key_digest
            {
                return Err(service_rpc_error_for("bootstrap-identity-mismatch"));
            }
            let response = guest::GuestSessionResponse {
                outcome: EnumOrUnknown::new(Outcome::OUTCOME_SUCCEEDED),
                operation_id: context.operation_id.clone(),
                session_generation: self.session.generation(),
                request_id: metadata.request_id.clone(),
                guest_identity_handle: self.identity.guest_identity_handle.clone(),
                guest_identity_digest: self.identity.guest_identity_digest.to_vec(),
                guest_static_public_key: self.identity.guest_static_public_key.to_vec(),
                guest_static_public_key_digest: self
                    .identity
                    .guest_static_public_key_digest
                    .to_vec(),
                parent_static_public_key_digest: self
                    .identity
                    .parent_static_public_key_digest
                    .to_vec(),
                capabilities: capabilities.into_iter().map(EnumOrUnknown::new).collect(),
                ..Default::default()
            };
            validate_guest_session_response_for_bootstrap(&request, &response).map_err(
                |error| service_rpc_error_for(&format!("bootstrap-response-invalid-{error}")),
            )?;
            self.authorization_gate
                .before_point_of_no_return(GuestAuthorizationPhase::Bootstrap)
                .await;
            // Point of no return: cancellation wins by atomically moving the
            // ticket to CANCELLED, or this transition wins and the following
            // synchronous seal + authorization commit cannot be interrupted.
            if !ticket.begin_non_cancellable() {
                return Err(cancelled_rpc_error());
            }
            let committed_public = commit
                .commit()
                .map_err(|_| service_rpc_error_for("bootstrap-seal-failed"))?;
            if committed_public != preview_public {
                return Err(service_rpc_error_for("bootstrap-identity-mismatch"));
            }
            self.confirm_authorized()?;
            Ok(response)
        }
        .await;
        match result {
            Ok(response) => {
                self.complete_typed(&ticket, &response, false).await?;
                Ok(response)
            }
            Err(error) => {
                self.fail_ticket(&ticket).await;
                Err(error)
            }
        }
    }

    async fn reconnect(
        &self,
        rpc_context: &ttrpc::r#async::TtrpcContext,
        request: guest::GuestReconnectRequest,
    ) -> ttrpc::Result<guest::GuestSessionResponse> {
        request
            .validate_wire(true)
            .map_err(|_| invalid_request_rpc_error())?;
        let context = request
            .context
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let metadata = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let scope = context
            .scope
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let ticket = match self
            .admit_typed::<guest::GuestSessionResponse>(
                rpc_context,
                metadata,
                "Reconnect",
                &request,
                &context.request_digest,
                true,
            )
            .await?
        {
            TypedAdmission::Replay(response) => return Ok(response),
            TypedAdmission::New(ticket) => ticket,
        };
        let result = async {
            self.require_generation(metadata.session_generation)?;
            self.authorize_scope(scope)?;
            let capabilities = self.capabilities();
            if self.authorization.load(Ordering::Acquire) != 0
                || self.identity.phase != GuestSessionPhase::Enrolled
                || request.expected_generation != self.session.generation()
                || request.guest_identity_handle != self.identity.guest_identity_handle
                || request.expected_guest_identity_digest != self.identity.guest_identity_digest
                || request.expected_guest_static_public_key != self.identity.guest_static_public_key
                || Sha256::digest(self.identity.guest_static_public_key).as_slice()
                    != self.identity.guest_identity_digest
                || request.expected_guest_static_public_key_digest
                    != self.identity.guest_static_public_key_digest
                || request.expected_parent_static_public_key_digest
                    != self.identity.parent_static_public_key_digest
                || request.required_capabilities.iter().any(|required| {
                    required
                        .enum_value()
                        .ok()
                        .is_none_or(|required| !capabilities.contains(&required))
                })
            {
                return Err(unauthorized_rpc_error());
            }
            let response = guest::GuestSessionResponse {
                outcome: EnumOrUnknown::new(Outcome::OUTCOME_SUCCEEDED),
                operation_id: context.operation_id.clone(),
                session_generation: self.session.generation(),
                request_id: metadata.request_id.clone(),
                guest_identity_handle: self.identity.guest_identity_handle.clone(),
                guest_identity_digest: self.identity.guest_identity_digest.to_vec(),
                guest_static_public_key: self.identity.guest_static_public_key.to_vec(),
                guest_static_public_key_digest: self
                    .identity
                    .guest_static_public_key_digest
                    .to_vec(),
                parent_static_public_key_digest: self
                    .identity
                    .parent_static_public_key_digest
                    .to_vec(),
                capabilities: capabilities.into_iter().map(EnumOrUnknown::new).collect(),
                ..Default::default()
            };
            validate_guest_session_response_for_reconnect(&request, &response)
                .map_err(|_| service_rpc_error())?;
            self.authorization_gate
                .before_point_of_no_return(GuestAuthorizationPhase::Reconnect)
                .await;
            // Reconnect has no sealing side effect. This atomic transition is
            // still its point of no return immediately before authorization.
            if !ticket.begin_non_cancellable() {
                return Err(cancelled_rpc_error());
            }
            self.confirm_authorized()?;
            Ok(response)
        }
        .await;
        match result {
            Ok(response) => {
                self.complete_typed(&ticket, &response, false).await?;
                Ok(response)
            }
            Err(error) => {
                self.fail_ticket(&ticket).await;
                Err(error)
            }
        }
    }

    async fn exec(
        &self,
        rpc_context: &ttrpc::r#async::TtrpcContext,
        request: guest::GuestExecRequest,
    ) -> ttrpc::Result<terminal::TerminalOpenResponse> {
        request
            .validate_wire(true)
            .map_err(|_| invalid_request_rpc_error())?;
        let terminal = request
            .terminal
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let metadata = terminal
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let ticket = match self
            .admit_typed::<terminal::TerminalOpenResponse>(
                rpc_context,
                metadata,
                "Exec",
                &request,
                &terminal.request_digest,
                true,
            )
            .await?
        {
            TypedAdmission::Replay(response) => return Ok(response),
            TypedAdmission::New(ticket) => ticket,
        };
        let result = async {
            self.require_authorized()?;
            self.require_capability(guest::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED)?;
            if !self.handler.stream_ready(GuestStreamMethod::Exec)
                || ticket.cancellation.is_cancelled()
            {
                return Err(service_rpc_error());
            }
            let binding = self.terminal_binding(
                terminal,
                self.resource_handle(&metadata.request_id),
                &ticket,
            )?;
            let (stream_id, mut stream) = self
                .hub
                .reserve()
                .await
                .map_err(|_| service_rpc_error_for("stream-reservation-failed"))?;
            stream.attach_request(Arc::clone(&self.requests), ticket.clone());
            let response = Self::accepted_response(&binding, stream_id);
            validate_guest_exec_response_for_request(&request, &response)
                .map_err(|_| service_rpc_error_for("exec-response-invalid"))?;
            self.handler
                .serve_exec(request.clone(), response.clone(), binding, stream)
                .await
                .map_err(map_service_error)?;
            if ticket.cancellation.is_cancelled() {
                return Err(cancelled_rpc_error());
            }
            Ok(response)
        }
        .await;
        match result {
            Ok(response) => {
                self.complete_typed(&ticket, &response, true).await?;
                Ok(response)
            }
            Err(error) => {
                self.fail_ticket(&ticket).await;
                Err(error)
            }
        }
    }

    async fn cancel_exec(
        &self,
        rpc_context: &ttrpc::r#async::TtrpcContext,
        request: guest::GuestCancelExecRequest,
    ) -> ttrpc::Result<guest::GuestCancelExecResponse> {
        request
            .validate_wire(true)
            .map_err(|_| invalid_request_rpc_error())?;
        let context = request
            .context
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let metadata = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let ticket = match self
            .admit_typed::<guest::GuestCancelExecResponse>(
                rpc_context,
                metadata,
                "CancelExec",
                &request,
                &context.request_digest,
                true,
            )
            .await?
        {
            TypedAdmission::Replay(response) => return Ok(response),
            TypedAdmission::New(ticket) => ticket,
        };
        let result = async {
            self.require_authorized()?;
            let binding =
                self.context_binding(context, request.resource_handle.clone(), &ticket)?;
            let response = tokio::select! {
                response = self.handler.cancel_exec(request.clone(), binding) => {
                    response.map_err(map_service_error)?
                }
                () = ticket.cancellation.cancelled() => return Err(cancelled_rpc_error()),
            };
            validate_guest_cancel_response_for_request(&request, &response)
                .map_err(|_| service_rpc_error())?;
            Ok(response)
        }
        .await;
        match result {
            Ok(response) => {
                self.complete_typed(&ticket, &response, false).await?;
                Ok(response)
            }
            Err(error) => {
                self.fail_ticket(&ticket).await;
                Err(error)
            }
        }
    }

    async fn inspect_exec(
        &self,
        rpc_context: &ttrpc::r#async::TtrpcContext,
        request: guest::GuestInspectExecRequest,
    ) -> ttrpc::Result<guest::GuestInspectExecResponse> {
        request
            .validate_wire(false)
            .map_err(|_| invalid_request_rpc_error())?;
        let context = request
            .context
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let metadata = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let ticket = match self
            .admit_typed::<guest::GuestInspectExecResponse>(
                rpc_context,
                metadata,
                "InspectExec",
                &request,
                &context.request_digest,
                false,
            )
            .await?
        {
            TypedAdmission::Replay(response) => return Ok(response),
            TypedAdmission::New(ticket) => ticket,
        };
        let result = async {
            self.require_authorized()?;
            let binding = self.context_binding(context, "inspect".to_owned(), &ticket)?;
            let response = tokio::select! {
                response = self.handler.inspect_exec(request.clone(), binding) => {
                    response.map_err(map_service_error)?
                }
                () = ticket.cancellation.cancelled() => return Err(cancelled_rpc_error()),
            };
            validate_guest_inspect_response_for_request(&request, &response)
                .map_err(|_| service_rpc_error())?;
            Ok(response)
        }
        .await;
        match result {
            Ok(response) => {
                self.complete_typed(&ticket, &response, false).await?;
                Ok(response)
            }
            Err(error) => {
                self.fail_ticket(&ticket).await;
                Err(error)
            }
        }
    }

    async fn open_exec_retained_log(
        &self,
        rpc_context: &ttrpc::r#async::TtrpcContext,
        request: guest::GuestOpenExecRetainedLogRequest,
    ) -> ttrpc::Result<terminal::TerminalOpenResponse> {
        request
            .validate_wire(true)
            .map_err(|_| invalid_request_rpc_error())?;
        let context = request
            .context
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let metadata = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let ticket = match self
            .admit_typed::<terminal::TerminalOpenResponse>(
                rpc_context,
                metadata,
                "OpenExecRetainedLog",
                &request,
                &context.request_digest,
                true,
            )
            .await?
        {
            TypedAdmission::Replay(response) => return Ok(response),
            TypedAdmission::New(ticket) => ticket,
        };
        let result = async {
            self.require_authorized()?;
            self.require_capability(guest::GuestCapability::GUEST_CAPABILITY_EXEC_RETAINED_LOGS)?;
            if !self.handler.stream_ready(GuestStreamMethod::RetainedLog) {
                return Err(service_rpc_error());
            }
            let binding =
                self.context_binding(context, request.resource_handle.clone(), &ticket)?;
            let (stream_id, mut stream) =
                self.hub.reserve().await.map_err(|_| service_rpc_error())?;
            stream.attach_request(Arc::clone(&self.requests), ticket.clone());
            let mut response = Self::accepted_response(&binding, stream_id);
            let range = self
                .handler
                .serve_retained_log(request.clone(), response.clone(), binding, stream)
                .await
                .map_err(map_service_error)?;
            response.retained_log = MessageField::some(range);
            validate_guest_open_exec_retained_log_response_for_request(&request, &response)
                .map_err(|_| service_rpc_error())?;
            Ok(response)
        }
        .await;
        match result {
            Ok(response) => {
                self.complete_typed(&ticket, &response, true).await?;
                Ok(response)
            }
            Err(error) => {
                self.fail_ticket(&ticket).await;
                Err(error)
            }
        }
    }

    async fn open_shell(
        &self,
        rpc_context: &ttrpc::r#async::TtrpcContext,
        request: guest::GuestOpenShellRequest,
    ) -> ttrpc::Result<terminal::TerminalOpenResponse> {
        request
            .validate_wire(true)
            .map_err(|_| invalid_request_rpc_error())?;
        let terminal = request
            .terminal
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let metadata = terminal
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let ticket = match self
            .admit_typed::<terminal::TerminalOpenResponse>(
                rpc_context,
                metadata,
                "OpenShell",
                &request,
                &terminal.request_digest,
                true,
            )
            .await?
        {
            TypedAdmission::Replay(response) => return Ok(response),
            TypedAdmission::New(ticket) => ticket,
        };
        let result = async {
            self.require_authorized()?;
            if !self.capabilities().iter().any(|capability| {
                matches!(
                    capability,
                    guest::GuestCapability::GUEST_CAPABILITY_SHELL_ATTACHED
                        | guest::GuestCapability::GUEST_CAPABILITY_SHELL_MANAGEMENT
                )
            }) || !self.handler.stream_ready(GuestStreamMethod::Shell)
            {
                return Err(service_rpc_error());
            }
            let binding = self.terminal_binding(
                terminal,
                self.resource_handle(&metadata.request_id),
                &ticket,
            )?;
            let (stream_id, mut stream) =
                self.hub.reserve().await.map_err(|_| service_rpc_error())?;
            stream.attach_request(Arc::clone(&self.requests), ticket.clone());
            let response = Self::accepted_response(&binding, stream_id);
            validate_guest_open_shell_response_for_request(&request, &response)
                .map_err(|_| service_rpc_error())?;
            self.handler
                .serve_shell(request.clone(), response.clone(), binding, stream)
                .await
                .map_err(map_service_error)?;
            Ok(response)
        }
        .await;
        match result {
            Ok(response) => {
                self.complete_typed(&ticket, &response, true).await?;
                Ok(response)
            }
            Err(error) => {
                self.fail_ticket(&ticket).await;
                Err(error)
            }
        }
    }

    async fn file_transfer(
        &self,
        rpc_context: &ttrpc::r#async::TtrpcContext,
        request: guest::GuestFileTransferRequest,
    ) -> ttrpc::Result<terminal::TerminalOpenResponse> {
        request
            .validate_wire(true)
            .map_err(|_| invalid_request_rpc_error())?;
        let context = request
            .context
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let metadata = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let ticket = match self
            .admit_typed::<terminal::TerminalOpenResponse>(
                rpc_context,
                metadata,
                "FileTransfer",
                &request,
                &context.request_digest,
                true,
            )
            .await?
        {
            TypedAdmission::Replay(response) => return Ok(response),
            TypedAdmission::New(ticket) => ticket,
        };
        let result = async {
            self.require_authorized()?;
            self.require_capability(guest::GuestCapability::GUEST_CAPABILITY_FILE_TRANSFER)?;
            if !self.handler.stream_ready(GuestStreamMethod::FileTransfer) {
                return Err(service_rpc_error());
            }
            let binding =
                self.context_binding(context, self.resource_handle(&metadata.request_id), &ticket)?;
            let (stream_id, mut stream) =
                self.hub.reserve().await.map_err(|_| service_rpc_error())?;
            stream.attach_request(Arc::clone(&self.requests), ticket.clone());
            let response = Self::accepted_response(&binding, stream_id);
            validate_terminal_open_response_for_guest_context(context, &response)
                .map_err(|_| service_rpc_error())?;
            self.handler
                .serve_file_transfer(request.clone(), response.clone(), binding, stream)
                .await
                .map_err(map_service_error)?;
            Ok(response)
        }
        .await;
        match result {
            Ok(response) => {
                self.complete_typed(&ticket, &response, true).await?;
                Ok(response)
            }
            Err(error) => {
                self.fail_ticket(&ticket).await;
                Err(error)
            }
        }
    }

    async fn security_key(
        &self,
        rpc_context: &ttrpc::r#async::TtrpcContext,
        request: guest::GuestSecurityKeyRequest,
    ) -> ttrpc::Result<terminal::TerminalOpenResponse> {
        request
            .validate_wire(true)
            .map_err(|_| invalid_request_rpc_error())?;
        let context = request
            .context
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let metadata = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let ticket = match self
            .admit_typed::<terminal::TerminalOpenResponse>(
                rpc_context,
                metadata,
                "SecurityKey",
                &request,
                &context.request_digest,
                true,
            )
            .await?
        {
            TypedAdmission::Replay(response) => return Ok(response),
            TypedAdmission::New(ticket) => ticket,
        };
        let result = async {
            self.require_authorized()?;
            self.require_capability(guest::GuestCapability::GUEST_CAPABILITY_SECURITY_KEY)?;
            if !self.handler.stream_ready(GuestStreamMethod::SecurityKey) {
                return Err(service_rpc_error());
            }
            let resource_handle = if request.action.enum_value().ok()
                == Some(guest::GuestSecurityKeyAction::GUEST_SECURITY_KEY_ACTION_RESUME)
            {
                request.ceremony_handle.clone()
            } else {
                self.resource_handle(&metadata.request_id)
            };
            let binding = self.context_binding(context, resource_handle, &ticket)?;
            let (stream_id, mut stream) =
                self.hub.reserve().await.map_err(|_| service_rpc_error())?;
            stream.attach_request(Arc::clone(&self.requests), ticket.clone());
            let response = Self::accepted_response(&binding, stream_id);
            validate_terminal_open_response_for_guest_context(context, &response)
                .map_err(|_| service_rpc_error())?;
            self.handler
                .serve_security_key(request.clone(), response.clone(), binding, stream)
                .await
                .map_err(map_service_error)?;
            Ok(response)
        }
        .await;
        match result {
            Ok(response) => {
                self.complete_typed(&ticket, &response, true).await?;
                Ok(response)
            }
            Err(error) => {
                self.fail_ticket(&ticket).await;
                Err(error)
            }
        }
    }

    async fn shutdown(
        &self,
        rpc_context: &ttrpc::r#async::TtrpcContext,
        request: guest::GuestShutdownRequest,
    ) -> ttrpc::Result<guest::GuestShutdownResponse> {
        request
            .validate_wire(true)
            .map_err(|_| invalid_request_rpc_error())?;
        let context = request
            .context
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let metadata = context
            .metadata
            .as_ref()
            .ok_or_else(invalid_request_rpc_error)?;
        let ticket = match self
            .admit_typed::<guest::GuestShutdownResponse>(
                rpc_context,
                metadata,
                "Shutdown",
                &request,
                &context.request_digest,
                true,
            )
            .await?
        {
            TypedAdmission::Replay(response) => return Ok(response),
            TypedAdmission::New(ticket) => ticket,
        };
        let result = async {
            self.require_authorized()?;
            self.require_capability(guest::GuestCapability::GUEST_CAPABILITY_SHUTDOWN)?;
            self.context_binding(context, "shutdown".to_owned(), &ticket)?;
            let response = tokio::select! {
                response = self.handler.shutdown(request.clone()) => {
                    response.map_err(map_service_error)?
                }
                () = ticket.cancellation.cancelled() => return Err(cancelled_rpc_error()),
            };
            validate_guest_shutdown_response_for_request(&request, &response)
                .map_err(|_| service_rpc_error())?;
            Ok(response)
        }
        .await;
        match result {
            Ok(response) => {
                self.complete_typed(&ticket, &response, false).await?;
                Ok(response)
            }
            Err(error) => {
                self.fail_ticket(&ticket).await;
                Err(error)
            }
        }
    }

    async fn cancel(
        &self,
        _: &ttrpc::r#async::TtrpcContext,
        request: common::CancelRequest,
    ) -> ttrpc::Result<common::CancelResponse> {
        if request.session_generation != self.session.generation()
            || SessionRequestId::new(request.request_id.clone()).is_err()
        {
            return Err(invalid_request_rpc_error());
        }
        Ok(self.requests.cancel(&request).await)
    }
}

pub async fn serve_guest_session(
    session: EstablishedGuestSession,
    handler: Arc<dyn GuestOperationHandler>,
    activation: Arc<ActivationRuntime>,
) -> Result<(), GuestSessionError> {
    let driver = Arc::clone(&session.driver);
    let owner_key = session.owner_key.to_vec();
    let service = Arc::new(GuestServiceV2::with_handler(Arc::clone(&handler), session));
    let activation_service = ActivationServiceV2::new(activation, service.access());
    let (server_transport, bridge_transport) =
        tokio::io::duplex(LimitProfile::local_default().logical_ttrpc_bytes as usize);
    let listener = ttrpc::r#async::transport::Listener::new(stream::once(async move {
        Ok::<_, std::io::Error>(server_transport)
    }));
    let mut server = ttrpc::r#async::Server::new()
        .add_listener(listener)
        .register_service(create_guest_service(service))
        .register_service(create_activation_service(activation_service));
    server
        .start()
        .await
        .map_err(|_| GuestSessionError::Service)?;

    let (mut bridge_reader, mut bridge_writer) = tokio::io::split(bridge_transport);
    let receive_driver = Arc::clone(&driver);
    let receive = async move {
        loop {
            let frame = receive_driver
                .receive_ttrpc()
                .await
                .map_err(GuestSessionError::from)?;
            bridge_writer
                .write_all(&frame)
                .await
                .map_err(|_| GuestSessionError::Transport)?;
            bridge_writer
                .flush()
                .await
                .map_err(|_| GuestSessionError::Transport)?;
        }
        #[allow(unreachable_code)]
        Ok::<(), GuestSessionError>(())
    };
    let send = async move {
        let mut frame = vec![0_u8; 64 * 1024];
        loop {
            let read = bridge_reader
                .read(&mut frame)
                .await
                .map_err(|_| GuestSessionError::Transport)?;
            if read == 0 {
                return Ok::<(), GuestSessionError>(());
            }
            driver
                .send_ttrpc(frame[..read].to_vec())
                .await
                .map_err(GuestSessionError::from)?;
        }
    };
    let result = tokio::select! {
        result = receive => result,
        result = send => result,
    };
    handler.disconnect(&owner_key);
    server.disconnect().await;
    result
}

fn invalid_request_rpc_error() -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(
        ttrpc::Code::INVALID_ARGUMENT,
        "guest-service-request-invalid".to_owned(),
    ))
}

fn unauthorized_rpc_error() -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(
        ttrpc::Code::PERMISSION_DENIED,
        "guest-service-authorization-denied".to_owned(),
    ))
}

fn cancelled_rpc_error() -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(
        ttrpc::Code::CANCELLED,
        "guest-service-request-cancelled".to_owned(),
    ))
}

fn map_admission_error(error: RequestAdmissionError) -> ttrpc::Error {
    let (code, message) = match error {
        RequestAdmissionError::Deadline => (
            ttrpc::Code::DEADLINE_EXCEEDED,
            "guest-service-deadline-invalid",
        ),
        RequestAdmissionError::Duplicate | RequestAdmissionError::ReplayConflict => {
            (ttrpc::Code::ALREADY_EXISTS, "guest-service-replay-conflict")
        }
        RequestAdmissionError::Cancelled => {
            (ttrpc::Code::CANCELLED, "guest-service-request-cancelled")
        }
        RequestAdmissionError::Session => (
            ttrpc::Code::FAILED_PRECONDITION,
            "guest-service-session-invalid",
        ),
    };
    ttrpc::Error::RpcStatus(ttrpc::get_status(code, message.to_owned()))
}

fn service_rpc_error() -> ttrpc::Error {
    service_rpc_error_for("guest-service-failed-closed")
}

fn service_rpc_error_for(message: &str) -> ttrpc::Error {
    ttrpc::Error::RpcStatus(ttrpc::get_status(
        ttrpc::Code::FAILED_PRECONDITION,
        message.to_owned(),
    ))
}

fn map_service_error(_: GuestSessionError) -> ttrpc::Error {
    service_rpc_error()
}
