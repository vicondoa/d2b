//! Typed `d2b.guest.v2` terminal client adapter.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{
        GuestSessionCredentialV1, LimitProfile, MAX_NAMED_STREAM_QUEUE_BYTES, RequestId,
    },
    v2_services::{
        RedactedTerminalFrame, ServerStreamLease, StrictWireMessage, TerminalFrameDirection,
        TerminalStreamValidator,
        activation_ttrpc::ActivationServiceClient,
        common, decode_strict,
        guest::{
            GuestCancelExecRequest, GuestCancelExecResponse, GuestExecRequest,
            GuestFileTransferFrame, GuestFileTransferRequest, GuestInspectExecRequest,
            GuestInspectExecResponse, GuestOpenExecRetainedLogRequest, GuestOpenShellRequest,
            guest_file_transfer_frame,
        },
        guest_contract::{
            FileTransferStreamValidator, GuestStreamDirection, MAX_GUEST_FILE_CHUNK_BYTES,
            retained_log_stream_validator, validate_guest_cancel_response_for_request,
            validate_guest_exec_response_for_request, validate_guest_inspect_response_for_request,
            validate_guest_open_exec_retained_log_response_for_request,
            validate_guest_open_shell_response_for_request,
            validate_terminal_open_response_for_guest_context,
        },
        guest_ttrpc::GuestServiceClient,
        terminal::{self, terminal_stream_frame},
    },
};
use d2b_session::{ComponentSessionDriver, StreamEvent, StreamId};
use protobuf::{Message, MessageField};
use sha2::{Digest, Sha256};
use tokio::{io::AsyncWriteExt, sync::mpsc};
use ttrpc::r#async::transport::Socket;
use ttrpc::proto::{
    MESSAGE_HEADER_LENGTH, MESSAGE_TYPE_REQUEST, MESSAGE_TYPE_RESPONSE, MessageHeader,
};

use crate::daemon_terminal::{
    TerminalCommand, TerminalFailure, TerminalFinish, TerminalOpenResult, TerminalOutputStream,
    TerminalOwner, TerminalOwnerEvent,
};

const GUEST_TERMINAL_QUEUE: usize = 32;
const GUEST_TERMINAL_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

#[async_trait]
pub trait GuestTerminalConnector: Send + Sync + fmt::Debug {
    async fn acquire_material(
        &self,
        workload: &str,
    ) -> Result<GuestSessionCredentialV1, TerminalFailure>;

    async fn connect_with_material(
        &self,
        workload: &str,
        material: GuestSessionCredentialV1,
    ) -> Result<Arc<GuestTerminalSession>, TerminalFailure>;

    async fn connect(&self, workload: &str) -> Result<Arc<GuestTerminalSession>, TerminalFailure> {
        let material = self.acquire_material(workload).await?;
        self.connect_with_material(workload, material).await
    }

    async fn connect_proxy(
        &self,
        workload: &str,
    ) -> Result<Arc<dyn GuestProxySession>, TerminalFailure> {
        let session: Arc<dyn GuestProxySession> = self.connect(workload).await?;
        Ok(session)
    }
}

#[derive(Debug, Default)]
pub struct UnavailableGuestTerminalConnector;

#[async_trait]
impl GuestTerminalConnector for UnavailableGuestTerminalConnector {
    async fn acquire_material(&self, _: &str) -> Result<GuestSessionCredentialV1, TerminalFailure> {
        Err(TerminalFailure::Unavailable)
    }

    async fn connect_with_material(
        &self,
        _: &str,
        _: GuestSessionCredentialV1,
    ) -> Result<Arc<GuestTerminalSession>, TerminalFailure> {
        Err(TerminalFailure::Unavailable)
    }
}

pub struct GuestTerminalSession {
    driver: Arc<dyn ComponentSessionDriver>,
    client: GuestServiceClient,
    activation_client: ActivationServiceClient,
    routes: Arc<GuestStreamRoutes>,
}

#[async_trait]
pub trait GuestProxySession: Send + Sync + fmt::Debug {
    fn generation(&self) -> u64;

    async fn cancel_exec(
        &self,
        request: GuestCancelExecRequest,
        timeout: Duration,
    ) -> Result<GuestCancelExecResponse, TerminalFailure>;

    async fn inspect_exec(
        &self,
        request: GuestInspectExecRequest,
        timeout: Duration,
    ) -> Result<GuestInspectExecResponse, TerminalFailure>;

    async fn prepare_retained_log(
        &self,
        request: GuestOpenExecRetainedLogRequest,
        timeout: Duration,
    ) -> Result<terminal::TerminalOpenResponse, TerminalFailure>;

    async fn open_prepared_retained_log(
        &self,
        request: &GuestOpenExecRetainedLogRequest,
        response: terminal::TerminalOpenResponse,
        selection: terminal::TerminalSelection,
    ) -> Result<TerminalOpenResult, TerminalFailure>;

    async fn abandon_retained_log(&self, stream_id: &str);

    async fn cancel_request(&self, request_id: &[u8]);
}

impl fmt::Debug for GuestTerminalSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestTerminalSession")
            .field("generation", &"<redacted>")
            .field("routes", &self.routes.active())
            .finish_non_exhaustive()
    }
}

impl GuestTerminalSession {
    pub fn from_driver(driver: Arc<dyn ComponentSessionDriver>) -> Arc<Self> {
        let (client_transport, bridge_transport) = tokio::io::duplex(2 * 1024 * 1024);
        let ttrpc_client = ttrpc::r#async::Client::new(Socket::new(client_transport));
        let client = GuestServiceClient::new(ttrpc_client.clone());
        let activation_client = ActivationServiceClient::new(ttrpc_client);
        let routes = GuestStreamRoutes::new(Arc::clone(&driver));
        tokio::spawn(pump_guest_ttrpc(bridge_transport, Arc::clone(&driver)));
        Arc::new(Self {
            driver,
            client,
            activation_client,
            routes,
        })
    }

    pub fn generation(&self) -> u64 {
        self.driver.generation()
    }

    pub(crate) async fn probe_live(&self, timeout: Duration) -> bool {
        tokio::time::timeout(
            timeout,
            self.driver.drive_keepalive(std::time::Instant::now()),
        )
        .await
        .is_ok_and(|result| result.is_ok())
    }

    pub(crate) async fn close_session(&self) {
        let _ = self
            .driver
            .close(
                d2b_contracts::v2_component_session::CloseReason::Normal,
                d2b_contracts::v2_component_session::Remediation::None,
            )
            .await;
    }

    pub(crate) async fn activation_activate(
        &self,
        request: common::ServiceRequest,
        timeout: Duration,
    ) -> Result<common::ServiceResponse, TerminalFailure> {
        request
            .validate_wire(true)
            .map_err(|_| TerminalFailure::Protocol)?;
        let response = self
            .activation_client
            .activate(ttrpc_context(timeout), &request)
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        validate_activation_response(&request, &response)?;
        Ok(response)
    }

    pub(crate) async fn activation_inspect(
        &self,
        request: common::ServiceRequest,
        timeout: Duration,
    ) -> Result<common::ServiceResponse, TerminalFailure> {
        request
            .validate_wire(false)
            .map_err(|_| TerminalFailure::Protocol)?;
        let response = self
            .activation_client
            .inspect(ttrpc_context(timeout), &request)
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        validate_activation_response(&request, &response)?;
        Ok(response)
    }

    pub(crate) async fn activation_cancel(
        &self,
        request: common::CancelRequest,
        timeout: Duration,
    ) -> Result<common::CancelResponse, TerminalFailure> {
        request
            .validate_wire(false)
            .map_err(|_| TerminalFailure::Protocol)?;
        let response = self
            .activation_client
            .cancel(ttrpc_context(timeout), &request)
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        response
            .validate_wire(false)
            .map_err(|_| TerminalFailure::Protocol)?;
        Ok(response)
    }

    pub(crate) async fn upload_activation_payload(
        &self,
        request: GuestFileTransferRequest,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<(), TerminalFailure> {
        request
            .validate_wire(true)
            .map_err(|_| TerminalFailure::Protocol)?;
        if payload.is_empty()
            || payload.len() > MAX_GUEST_FILE_CHUNK_BYTES
            || request.offset != 0
            || request.declared_size != payload.len() as u64
            || request.expected_digest != Sha256::digest(payload).as_slice()
        {
            return Err(TerminalFailure::Protocol);
        }
        let response = self
            .client
            .file_transfer(ttrpc_context(timeout), &request)
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        let context = request.context.as_ref().ok_or(TerminalFailure::Protocol)?;
        validate_terminal_open_response_for_guest_context(context, &response)
            .map_err(|_| TerminalFailure::Protocol)?;
        if response.outcome.enum_value().ok() != Some(common::Outcome::OUTCOME_ACCEPTED) {
            return Err(map_open_failure(&response));
        }
        let mut lease = ServerStreamLease::reserve(
            d2b_contracts::v2_services::parse_server_stream_name(&response.stream_id)
                .map_err(|_| TerminalFailure::Protocol)?,
        )
        .map_err(|_| TerminalFailure::Protocol)?;
        let channel = lease
            .open_by_client(&response.stream_id)
            .map_err(|_| TerminalFailure::Protocol)?;
        let stream = StreamId::new(channel).map_err(|_| TerminalFailure::Protocol)?;
        let mut receiver = self.routes.register(stream)?;
        if self
            .driver
            .open_named_stream(
                stream,
                MAX_NAMED_STREAM_QUEUE_BYTES,
                MAX_NAMED_STREAM_QUEUE_BYTES,
            )
            .await
            .is_err()
        {
            self.routes.unregister(stream);
            return Err(TerminalFailure::Unavailable);
        }
        let result = self
            .drive_activation_payload_upload(
                stream,
                &mut receiver,
                &request,
                &response,
                payload,
                timeout,
            )
            .await;
        match result {
            Ok(()) => {
                self.routes.unregister(stream);
                self.driver
                    .close_named_stream(stream)
                    .await
                    .map_err(|_| TerminalFailure::Unavailable)
            }
            Err(error) => {
                self.routes.unregister(stream);
                let _ = self.driver.reset_named_stream(stream).await;
                Err(error)
            }
        }
    }

    async fn drive_activation_payload_upload(
        &self,
        stream: StreamId,
        receiver: &mut mpsc::Receiver<StreamEvent>,
        request: &GuestFileTransferRequest,
        response: &terminal::TerminalOpenResponse,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<(), TerminalFailure> {
        let context = request.context.as_ref().ok_or(TerminalFailure::Protocol)?;
        let metadata = context.metadata.as_ref().ok_or(TerminalFailure::Protocol)?;
        let mut validator = FileTransferStreamValidator::new(request, response)
            .map_err(|_| TerminalFailure::Protocol)?;
        let binding = FileTransferBinding {
            session_generation: metadata.session_generation,
            request_id: metadata.request_id.clone(),
            operation_id: context.operation_id.clone(),
            resource_handle: response.resource_handle.clone(),
        };
        send_file_transfer_frame(
            &self.driver,
            stream,
            &mut validator,
            &binding,
            0,
            guest_file_transfer_frame::Frame::Start(
                d2b_contracts::v2_services::guest::GuestFileTransferStart {
                    artifact: request.artifact,
                    configured_intent_id: request.configured_intent_id.clone(),
                    direction: request.direction,
                    offset: request.offset,
                    declared_size: request.declared_size,
                    expected_digest: request.expected_digest.clone(),
                    ..Default::default()
                },
            ),
        )
        .await?;
        let credit =
            receive_file_transfer_frame(&self.driver, stream, receiver, &mut validator, timeout)
                .await?;
        let Some(guest_file_transfer_frame::Frame::Credit(credit)) = credit.frame else {
            return Err(TerminalFailure::Protocol);
        };
        if credit.next_offset != 0 || credit.bytes < payload.len() as u32 {
            return Err(TerminalFailure::Protocol);
        }
        let digest = Sha256::digest(payload).to_vec();
        send_file_transfer_frame(
            &self.driver,
            stream,
            &mut validator,
            &binding,
            1,
            guest_file_transfer_frame::Frame::Chunk(
                d2b_contracts::v2_services::guest::GuestFileTransferChunk {
                    offset: 0,
                    data: payload.to_vec(),
                    eof: true,
                    total_size: payload.len() as u64,
                    final_digest: digest.clone(),
                    ..Default::default()
                },
            ),
        )
        .await?;
        let completed =
            receive_file_transfer_frame(&self.driver, stream, receiver, &mut validator, timeout)
                .await?;
        let Some(guest_file_transfer_frame::Frame::Complete(completed)) = completed.frame else {
            return Err(TerminalFailure::Protocol);
        };
        if completed.total_size != payload.len() as u64 || completed.digest != digest {
            return Err(TerminalFailure::Protocol);
        }
        validator
            .accept_transport_close()
            .map_err(|_| TerminalFailure::Protocol)
    }

    pub async fn open_exec(
        self: &Arc<Self>,
        request: GuestExecRequest,
        selection: terminal::TerminalSelection,
        timeout: Duration,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        request
            .validate_wire(true)
            .map_err(|_| TerminalFailure::Protocol)?;
        let response = self
            .client
            .exec(ttrpc_context(timeout), &request)
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        validate_guest_exec_response_for_request(&request, &response)
            .map_err(|_| TerminalFailure::Protocol)?;
        self.open_terminal(
            terminal::TerminalKind::TERMINAL_KIND_EXEC,
            request.terminal.as_ref().ok_or(TerminalFailure::Protocol)?,
            response,
            selection,
            timeout,
        )
        .await
    }

    pub async fn open_shell(
        self: &Arc<Self>,
        request: GuestOpenShellRequest,
        selection: terminal::TerminalSelection,
        timeout: Duration,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        request
            .validate_wire(true)
            .map_err(|_| TerminalFailure::Protocol)?;
        let response = self
            .client
            .open_shell(ttrpc_context(timeout), &request)
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        validate_guest_open_shell_response_for_request(&request, &response)
            .map_err(|_| TerminalFailure::Protocol)?;
        self.open_terminal(
            terminal::TerminalKind::TERMINAL_KIND_SHELL,
            request.terminal.as_ref().ok_or(TerminalFailure::Protocol)?,
            response,
            selection,
            timeout,
        )
        .await
    }

    async fn proxy_cancel_exec(
        &self,
        request: GuestCancelExecRequest,
        timeout: Duration,
    ) -> Result<GuestCancelExecResponse, TerminalFailure> {
        request
            .validate_wire(true)
            .map_err(|_| TerminalFailure::Protocol)?;
        let response = self
            .client
            .cancel_exec(ttrpc_context(timeout), &request)
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        validate_guest_cancel_response_for_request(&request, &response)
            .map_err(|_| TerminalFailure::Protocol)?;
        Ok(response)
    }

    async fn proxy_inspect_exec(
        &self,
        request: GuestInspectExecRequest,
        timeout: Duration,
    ) -> Result<GuestInspectExecResponse, TerminalFailure> {
        request
            .validate_wire(false)
            .map_err(|_| TerminalFailure::Protocol)?;
        let response = self
            .client
            .inspect_exec(ttrpc_context(timeout), &request)
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        validate_guest_inspect_response_for_request(&request, &response)
            .map_err(|_| TerminalFailure::Protocol)?;
        Ok(response)
    }

    async fn proxy_prepare_retained_log(
        &self,
        request: GuestOpenExecRetainedLogRequest,
        timeout: Duration,
    ) -> Result<terminal::TerminalOpenResponse, TerminalFailure> {
        request
            .validate_wire(true)
            .map_err(|_| TerminalFailure::Protocol)?;
        let response = self
            .client
            .open_exec_retained_log(ttrpc_context(timeout), &request)
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        validate_guest_open_exec_retained_log_response_for_request(&request, &response)
            .map_err(|_| TerminalFailure::Protocol)?;
        Ok(response)
    }

    async fn proxy_open_prepared_retained_log(
        &self,
        request: &GuestOpenExecRetainedLogRequest,
        response: terminal::TerminalOpenResponse,
        selection: terminal::TerminalSelection,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        if response.outcome.enum_value().ok() != Some(common::Outcome::OUTCOME_ACCEPTED) {
            return Err(map_open_failure(&response));
        }
        if response.session_generation != self.generation() {
            return Err(TerminalFailure::GenerationMismatch);
        }
        let mut lease = ServerStreamLease::reserve(
            d2b_contracts::v2_services::parse_server_stream_name(&response.stream_id)
                .map_err(|_| TerminalFailure::Protocol)?,
        )
        .map_err(|_| TerminalFailure::Protocol)?;
        let channel = lease
            .open_by_client(&response.stream_id)
            .map_err(|_| TerminalFailure::Protocol)?;
        let stream = StreamId::new(channel).map_err(|_| TerminalFailure::Protocol)?;
        let receiver = self.routes.register(stream)?;
        if self
            .driver
            .open_named_stream(
                stream,
                MAX_NAMED_STREAM_QUEUE_BYTES,
                MAX_NAMED_STREAM_QUEUE_BYTES,
            )
            .await
            .is_err()
        {
            self.routes.unregister(stream);
            return Err(TerminalFailure::Unavailable);
        }
        let context = request.context.as_ref().ok_or(TerminalFailure::Protocol)?;
        let metadata = context.metadata.as_ref().ok_or(TerminalFailure::Protocol)?;
        let request_id: [u8; 16] = metadata
            .request_id
            .as_slice()
            .try_into()
            .map_err(|_| TerminalFailure::Protocol)?;
        let mut owner = GuestTerminalOwner {
            driver: Arc::clone(&self.driver),
            routes: Arc::clone(&self.routes),
            stream,
            receiver,
            validator: retained_log_stream_validator(request, &response)
                .map_err(|_| TerminalFailure::Protocol)?,
            session_generation: response.session_generation,
            request_id,
            operation_id: response.operation_id,
            resource_handle: response.resource_handle,
            next_client_sequence: 0,
            terminal: false,
        };
        if let Err(error) = owner
            .send_payload(terminal_stream_frame::Frame::Select(selection))
            .await
        {
            owner.routes.unregister(owner.stream);
            let _ = owner.driver.reset_named_stream(owner.stream).await;
            return Err(error);
        }
        Ok(TerminalOpenResult::ActiveWithoutStarted {
            owner: Box::new(owner),
        })
    }

    async fn proxy_abandon_retained_log(&self, stream_id: &str) {
        if let Ok(channel) = d2b_contracts::v2_services::parse_server_stream_name(stream_id)
            && let Ok(stream) = StreamId::new(channel)
        {
            self.routes.unregister(stream);
            let _ = self.driver.reset_named_stream(stream).await;
        }
    }

    async fn proxy_cancel_request(&self, request_id: &[u8]) {
        if let Ok(request_id) = RequestId::new(request_id.to_vec()) {
            let _ = self.driver.cancel(self.generation(), request_id).await;
        }
    }

    async fn open_terminal(
        self: &Arc<Self>,
        kind: terminal::TerminalKind,
        request: &terminal::TerminalOpenRequest,
        response: terminal::TerminalOpenResponse,
        selection: terminal::TerminalSelection,
        timeout: Duration,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        if response.outcome.enum_value().ok() != Some(common::Outcome::OUTCOME_ACCEPTED) {
            return Err(map_open_failure(&response));
        }
        if response.session_generation != self.generation() {
            return Err(TerminalFailure::GenerationMismatch);
        }
        let metadata = request.metadata.as_ref().ok_or(TerminalFailure::Protocol)?;
        let request_id: [u8; 16] = metadata
            .request_id
            .as_slice()
            .try_into()
            .map_err(|_| TerminalFailure::Protocol)?;
        let mut lease = ServerStreamLease::reserve(
            d2b_contracts::v2_services::parse_server_stream_name(&response.stream_id)
                .map_err(|_| TerminalFailure::Protocol)?,
        )
        .map_err(|_| TerminalFailure::Protocol)?;
        let channel = lease
            .open_by_client(&response.stream_id)
            .map_err(|_| TerminalFailure::Protocol)?;
        let stream = StreamId::new(channel).map_err(|_| TerminalFailure::Protocol)?;
        let receiver = self.routes.register(stream)?;
        if self
            .driver
            .open_named_stream(
                stream,
                MAX_NAMED_STREAM_QUEUE_BYTES,
                MAX_NAMED_STREAM_QUEUE_BYTES,
            )
            .await
            .is_err()
        {
            self.routes.unregister(stream);
            return Err(TerminalFailure::Unavailable);
        }
        let mut owner = GuestTerminalOwner {
            driver: Arc::clone(&self.driver),
            routes: Arc::clone(&self.routes),
            stream,
            receiver,
            validator: TerminalStreamValidator::new(
                kind,
                response.session_generation,
                request_id,
                response.operation_id.clone(),
                response.resource_handle.clone(),
            )
            .map_err(|_| TerminalFailure::Protocol)?,
            session_generation: response.session_generation,
            request_id,
            operation_id: response.operation_id,
            resource_handle: response.resource_handle,
            next_client_sequence: 0,
            terminal: false,
        };
        owner
            .send_payload(terminal_stream_frame::Frame::Select(selection))
            .await?;
        let first = owner.receive_frame(timeout).await?;
        match first.frame {
            Some(terminal_stream_frame::Frame::Started(started)) => {
                Ok(TerminalOpenResult::Active {
                    started,
                    owner: Box::new(owner),
                })
            }
            Some(terminal_stream_frame::Frame::ShellResult(result)) => {
                let outcome = owner.receive_frame(timeout).await?;
                let Some(terminal_stream_frame::Frame::Outcome(outcome)) = outcome.frame else {
                    return Err(TerminalFailure::Protocol);
                };
                owner.close_after_terminal().await?;
                Ok(TerminalOpenResult::Immediate(vec![
                    TerminalOwnerEvent::ShellResult(result),
                    TerminalOwnerEvent::Outcome(outcome),
                ]))
            }
            Some(terminal_stream_frame::Frame::Outcome(outcome)) => {
                owner.close_after_terminal().await?;
                Ok(TerminalOpenResult::Terminal(outcome))
            }
            _ => Err(TerminalFailure::Protocol),
        }
    }
}

struct FileTransferBinding {
    session_generation: u64,
    request_id: Vec<u8>,
    operation_id: String,
    resource_handle: String,
}

async fn send_file_transfer_frame(
    driver: &Arc<dyn ComponentSessionDriver>,
    stream: StreamId,
    validator: &mut FileTransferStreamValidator,
    binding: &FileTransferBinding,
    sequence: u64,
    frame: guest_file_transfer_frame::Frame,
) -> Result<(), TerminalFailure> {
    let message = GuestFileTransferFrame {
        session_generation: binding.session_generation,
        request_id: binding.request_id.clone(),
        sequence,
        operation_id: binding.operation_id.clone(),
        resource_handle: binding.resource_handle.clone(),
        frame: Some(frame),
        ..Default::default()
    };
    validator
        .accept(GuestStreamDirection::ClientToServer, &message)
        .map_err(|_| TerminalFailure::Protocol)?;
    driver
        .send_named_stream(
            stream,
            message
                .write_to_bytes()
                .map_err(|_| TerminalFailure::Protocol)?,
        )
        .await
        .map_err(|_| TerminalFailure::Unavailable)
}

async fn receive_file_transfer_frame(
    driver: &Arc<dyn ComponentSessionDriver>,
    stream: StreamId,
    receiver: &mut mpsc::Receiver<StreamEvent>,
    validator: &mut FileTransferStreamValidator,
    timeout: Duration,
) -> Result<GuestFileTransferFrame, TerminalFailure> {
    let event = tokio::time::timeout(timeout, receiver.recv())
        .await
        .map_err(|_| TerminalFailure::Unavailable)?
        .ok_or(TerminalFailure::Unavailable)?;
    let StreamEvent::Data {
        stream: actual,
        bytes,
    } = event
    else {
        return Err(TerminalFailure::Unavailable);
    };
    if actual != stream {
        return Err(TerminalFailure::Protocol);
    }
    let frame =
        GuestFileTransferFrame::parse_from_bytes(&bytes).map_err(|_| TerminalFailure::Protocol)?;
    let consumed = u32::try_from(bytes.len()).map_err(|_| TerminalFailure::ResourceExhausted)?;
    validator
        .accept_transport_credit(consumed)
        .and_then(|()| validator.accept(GuestStreamDirection::ServerToClient, &frame))
        .map_err(|_| TerminalFailure::Protocol)?;
    driver
        .grant_named_stream_credit(stream, consumed)
        .await
        .map_err(|_| TerminalFailure::Unavailable)?;
    Ok(frame)
}

fn validate_activation_response(
    request: &common::ServiceRequest,
    response: &common::ServiceResponse,
) -> Result<(), TerminalFailure> {
    response
        .validate_wire(false)
        .map_err(|_| TerminalFailure::Protocol)?;
    if response.operation_id != request.operation_id {
        return Err(TerminalFailure::Protocol);
    }
    if response.outcome.enum_value().ok() != Some(common::Outcome::OUTCOME_FAILED)
        && response.resource_handle != request.operation_id
    {
        return Err(TerminalFailure::Protocol);
    }
    if response
        .observations
        .iter()
        .any(|observation| observation.resource_id != request.resource_id)
    {
        return Err(TerminalFailure::Protocol);
    }
    Ok(())
}

#[async_trait]
impl GuestProxySession for GuestTerminalSession {
    fn generation(&self) -> u64 {
        GuestTerminalSession::generation(self)
    }

    async fn cancel_exec(
        &self,
        request: GuestCancelExecRequest,
        timeout: Duration,
    ) -> Result<GuestCancelExecResponse, TerminalFailure> {
        self.proxy_cancel_exec(request, timeout).await
    }

    async fn inspect_exec(
        &self,
        request: GuestInspectExecRequest,
        timeout: Duration,
    ) -> Result<GuestInspectExecResponse, TerminalFailure> {
        self.proxy_inspect_exec(request, timeout).await
    }

    async fn prepare_retained_log(
        &self,
        request: GuestOpenExecRetainedLogRequest,
        timeout: Duration,
    ) -> Result<terminal::TerminalOpenResponse, TerminalFailure> {
        self.proxy_prepare_retained_log(request, timeout).await
    }

    async fn open_prepared_retained_log(
        &self,
        request: &GuestOpenExecRetainedLogRequest,
        response: terminal::TerminalOpenResponse,
        selection: terminal::TerminalSelection,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        self.proxy_open_prepared_retained_log(request, response, selection)
            .await
    }

    async fn abandon_retained_log(&self, stream_id: &str) {
        self.proxy_abandon_retained_log(stream_id).await;
    }

    async fn cancel_request(&self, request_id: &[u8]) {
        self.proxy_cancel_request(request_id).await;
    }
}

pub struct GuestTerminalOwner {
    driver: Arc<dyn ComponentSessionDriver>,
    routes: Arc<GuestStreamRoutes>,
    stream: StreamId,
    receiver: mpsc::Receiver<StreamEvent>,
    validator: TerminalStreamValidator,
    session_generation: u64,
    request_id: [u8; 16],
    operation_id: String,
    resource_handle: String,
    next_client_sequence: u64,
    terminal: bool,
}

impl fmt::Debug for GuestTerminalOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestTerminalOwner")
            .field("binding", &"<redacted>")
            .field("terminal", &self.terminal)
            .finish()
    }
}

#[async_trait]
impl TerminalOwner for GuestTerminalOwner {
    async fn command(
        &mut self,
        command: TerminalCommand,
    ) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
        let payload = terminal_command_payload(command);
        self.send_payload(payload).await?;
        self.drain_ready().await
    }

    async fn poll(&mut self) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
        self.drain_ready().await
    }

    async fn finish(
        &mut self,
        finish: TerminalFinish,
    ) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
        if self.terminal {
            return Err(TerminalFailure::Conflict);
        }
        let payload = match finish {
            TerminalFinish::Detach => {
                terminal_stream_frame::Frame::Detach(terminal::TerminalDetach::default())
            }
            TerminalFinish::Close => {
                terminal_stream_frame::Frame::Close(terminal::TerminalClose::default())
            }
            TerminalFinish::Cancel | TerminalFinish::Disconnect => {
                terminal_stream_frame::Frame::Cancel(terminal::TerminalCancel::default())
            }
        };
        self.send_payload(payload).await?;
        if finish == TerminalFinish::Disconnect {
            let _ = self.driver.reset_named_stream(self.stream).await;
            self.routes.unregister(self.stream);
            self.terminal = true;
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        loop {
            let frame = self.receive_frame(GUEST_TERMINAL_CLOSE_TIMEOUT).await?;
            let terminal = matches!(frame.frame, Some(terminal_stream_frame::Frame::Outcome(_)));
            if let Some(event) = owner_event(frame)? {
                events.push(event);
            }
            if terminal {
                self.close_after_terminal().await?;
                return Ok(events);
            }
        }
    }
}

fn terminal_command_payload(command: TerminalCommand) -> terminal_stream_frame::Frame {
    match command {
        TerminalCommand::Stdin { offset, data, eof } => {
            terminal_stream_frame::Frame::Stdin(terminal::TerminalStdin {
                offset,
                data,
                eof,
                ..Default::default()
            })
        }
        TerminalCommand::Resize {
            operation_sequence,
            rows,
            columns,
        } => terminal_stream_frame::Frame::Resize(terminal::TerminalResize {
            operation_sequence,
            size: MessageField::some(terminal::TerminalSize {
                rows,
                columns,
                ..Default::default()
            }),
            ..Default::default()
        }),
        TerminalCommand::Signal {
            operation_sequence,
            signal,
        } => terminal_stream_frame::Frame::Signal(terminal::TerminalSignal {
            operation_sequence,
            signal: signal.into(),
            ..Default::default()
        }),
        TerminalCommand::CloseStdin => {
            terminal_stream_frame::Frame::CloseStdin(terminal::TerminalCloseStdin::default())
        }
    }
}

impl GuestTerminalOwner {
    async fn send_payload(
        &mut self,
        payload: terminal_stream_frame::Frame,
    ) -> Result<(), TerminalFailure> {
        let frame = terminal::TerminalStreamFrame {
            session_generation: self.session_generation,
            request_id: self.request_id.to_vec(),
            sequence: self.next_client_sequence,
            operation_id: self.operation_id.clone(),
            resource_handle: self.resource_handle.clone(),
            frame: Some(payload),
            ..Default::default()
        };
        self.validator
            .accept(TerminalFrameDirection::ClientToServer, &frame)
            .map_err(|_| TerminalFailure::Protocol)?;
        self.driver
            .send_named_stream(
                self.stream,
                frame
                    .write_to_bytes()
                    .map_err(|_| TerminalFailure::Protocol)?,
            )
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        self.next_client_sequence = self
            .next_client_sequence
            .checked_add(1)
            .ok_or(TerminalFailure::ResourceExhausted)?;
        Ok(())
    }

    async fn receive_frame(
        &mut self,
        timeout: Duration,
    ) -> Result<terminal::TerminalStreamFrame, TerminalFailure> {
        let event = tokio::time::timeout(timeout, self.receiver.recv())
            .await
            .map_err(|_| TerminalFailure::Unavailable)?
            .ok_or(TerminalFailure::Unavailable)?;
        match event {
            StreamEvent::Data { stream, bytes } if stream == self.stream => {
                let frame = terminal::TerminalStreamFrame::parse_from_bytes(&bytes)
                    .map_err(|_| TerminalFailure::Protocol)?;
                let consumed =
                    u32::try_from(bytes.len()).map_err(|_| TerminalFailure::ResourceExhausted)?;
                self.validator
                    .accept_transport_credit(consumed)
                    .map_err(|_| TerminalFailure::Protocol)?;
                if self
                    .validator
                    .accept(TerminalFrameDirection::ServerToClient, &frame)
                    .is_err()
                {
                    tracing::debug!(
                        frame = ?RedactedTerminalFrame(&frame),
                        "guest terminal frame rejected"
                    );
                    return Err(TerminalFailure::Protocol);
                }
                self.driver
                    .grant_named_stream_credit(self.stream, consumed)
                    .await
                    .map_err(|_| TerminalFailure::Unavailable)?;
                Ok(frame)
            }
            StreamEvent::RemoteClosed { stream } | StreamEvent::Reset { stream }
                if stream == self.stream =>
            {
                Err(TerminalFailure::Unavailable)
            }
            _ => Err(TerminalFailure::Protocol),
        }
    }

    async fn drain_ready(&mut self) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
        let mut events = Vec::new();
        for _ in 0..GUEST_TERMINAL_QUEUE {
            let event = match self.receiver.try_recv() {
                Ok(event) => event,
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(TerminalFailure::Unavailable);
                }
            };
            let StreamEvent::Data { stream, bytes } = event else {
                return Err(TerminalFailure::Unavailable);
            };
            if stream != self.stream {
                return Err(TerminalFailure::Protocol);
            }
            let frame = terminal::TerminalStreamFrame::parse_from_bytes(&bytes)
                .map_err(|_| TerminalFailure::Protocol)?;
            let consumed =
                u32::try_from(bytes.len()).map_err(|_| TerminalFailure::ResourceExhausted)?;
            self.validator
                .accept_transport_credit(consumed)
                .map_err(|_| TerminalFailure::Protocol)?;
            if self
                .validator
                .accept(TerminalFrameDirection::ServerToClient, &frame)
                .is_err()
            {
                tracing::debug!(
                    frame = ?RedactedTerminalFrame(&frame),
                    "guest terminal frame rejected"
                );
                return Err(TerminalFailure::Protocol);
            }
            self.driver
                .grant_named_stream_credit(self.stream, consumed)
                .await
                .map_err(|_| TerminalFailure::Unavailable)?;
            if let Some(event) = owner_event(frame)? {
                if matches!(event, TerminalOwnerEvent::Outcome(_)) {
                    self.terminal = true;
                }
                events.push(event);
            }
        }
        Ok(events)
    }

    async fn close_after_terminal(&mut self) -> Result<(), TerminalFailure> {
        if !self.validator.is_terminal() {
            return Err(TerminalFailure::Protocol);
        }
        self.validator
            .accept_transport_close()
            .map_err(|_| TerminalFailure::Protocol)?;
        self.driver
            .close_named_stream(self.stream)
            .await
            .map_err(|_| TerminalFailure::Unavailable)?;
        self.routes.unregister(self.stream);
        self.terminal = true;
        Ok(())
    }
}

impl Drop for GuestTerminalOwner {
    fn drop(&mut self) {
        self.routes.unregister(self.stream);
        if !self.terminal
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let driver = Arc::clone(&self.driver);
            let stream = self.stream;
            runtime.spawn(async move {
                let _ = driver.reset_named_stream(stream).await;
            });
        }
    }
}

fn owner_event(
    frame: terminal::TerminalStreamFrame,
) -> Result<Option<TerminalOwnerEvent>, TerminalFailure> {
    Ok(match frame.frame.ok_or(TerminalFailure::Protocol)? {
        terminal_stream_frame::Frame::Stdout(output) => Some(TerminalOwnerEvent::Output {
            stream: TerminalOutputStream::Stdout,
            offset: output.offset,
            data: output.data,
            eof: output.eof,
            dropped_bytes: output.dropped_bytes,
            truncated: output.truncated,
        }),
        terminal_stream_frame::Frame::Stderr(output) => Some(TerminalOwnerEvent::Output {
            stream: TerminalOutputStream::Stderr,
            offset: output.offset,
            data: output.data,
            eof: output.eof,
            dropped_bytes: output.dropped_bytes,
            truncated: output.truncated,
        }),
        terminal_stream_frame::Frame::Status(status) => Some(TerminalOwnerEvent::Status {
            status: status
                .status
                .enum_value()
                .map_err(|_| TerminalFailure::Protocol)?,
            next_stdin_offset: status.next_stdin_offset,
        }),
        terminal_stream_frame::Frame::Outcome(outcome) => {
            Some(TerminalOwnerEvent::Outcome(outcome))
        }
        terminal_stream_frame::Frame::ShellResult(result) => {
            Some(TerminalOwnerEvent::ShellResult(result))
        }
        terminal_stream_frame::Frame::Started(_) => return Err(TerminalFailure::Protocol),
        _ => return Err(TerminalFailure::Protocol),
    })
}

struct GuestStreamRoutes {
    driver: Arc<dyn ComponentSessionDriver>,
    senders: Mutex<BTreeMap<u16, mpsc::Sender<StreamEvent>>>,
}

impl GuestStreamRoutes {
    fn new(driver: Arc<dyn ComponentSessionDriver>) -> Arc<Self> {
        let routes = Arc::new(Self {
            driver,
            senders: Mutex::new(BTreeMap::new()),
        });
        tokio::spawn(run_guest_stream_router(Arc::clone(&routes)));
        routes
    }

    fn register(&self, stream: StreamId) -> Result<mpsc::Receiver<StreamEvent>, TerminalFailure> {
        let (sender, receiver) = mpsc::channel(GUEST_TERMINAL_QUEUE);
        let mut senders = self.senders.lock().map_err(|_| TerminalFailure::Internal)?;
        if senders.insert(stream.channel().value(), sender).is_some() {
            return Err(TerminalFailure::Conflict);
        }
        Ok(receiver)
    }

    fn unregister(&self, stream: StreamId) {
        if let Ok(mut senders) = self.senders.lock() {
            senders.remove(&stream.channel().value());
        }
    }

    fn active(&self) -> usize {
        self.senders.lock().map(|routes| routes.len()).unwrap_or(0)
    }
}

async fn run_guest_stream_router(routes: Arc<GuestStreamRoutes>) {
    loop {
        let event = match routes.driver.receive_named_stream().await {
            Ok(event) => event,
            Err(_) => return,
        };
        let stream = match &event {
            StreamEvent::Data { stream, .. }
            | StreamEvent::RemoteClosed { stream }
            | StreamEvent::Reset { stream } => *stream,
        };
        let sender = routes
            .senders
            .lock()
            .ok()
            .and_then(|senders| senders.get(&stream.channel().value()).cloned());
        let Some(sender) = sender else {
            let _ = routes.driver.reset_named_stream(stream).await;
            continue;
        };
        if sender.try_send(event).is_err() {
            routes.unregister(stream);
            let _ = routes.driver.reset_named_stream(stream).await;
        }
    }
}

async fn pump_guest_ttrpc(
    bridge: tokio::io::DuplexStream,
    driver: Arc<dyn ComponentSessionDriver>,
) {
    let (mut reader, mut writer) = tokio::io::split(bridge);
    let in_flight = Arc::new(tokio::sync::Mutex::new(BTreeMap::new()));
    let request_driver = Arc::clone(&driver);
    let request_map = Arc::clone(&in_flight);
    let send = async move {
        loop {
            let (header, request, frame) = read_ttrpc_request(&mut reader).await?;
            if header.type_ != MESSAGE_TYPE_REQUEST {
                return Err(());
            }
            let request_id = direct_guest_request_id(&request)?;
            if request_map
                .lock()
                .await
                .insert(header.stream_id, request_id.clone())
                .is_some()
            {
                return Err(());
            }
            match request_id {
                Some(request_id) => request_driver
                    .start_ttrpc(request_id, frame)
                    .await
                    .map_err(|_| ())?,
                None => request_driver.send_ttrpc(frame).await.map_err(|_| ())?,
            }
        }
    };
    let receive = async move {
        loop {
            let frame = driver.receive_ttrpc().await.map_err(|_| ())?;
            let header = ttrpc_frame_header(&frame)?;
            if header.type_ != MESSAGE_TYPE_RESPONSE {
                return Err(());
            }
            let request_id = in_flight.lock().await.remove(&header.stream_id).ok_or(())?;
            if let Some(request_id) = request_id
                && !driver.complete_ttrpc(request_id).await.map_err(|_| ())?
            {
                return Err(());
            }
            writer.write_all(&frame).await.map_err(|_| ())?;
            writer.flush().await.map_err(|_| ())?;
        }
    };
    let _: Result<(), ()> = tokio::select! {
        result = receive => result,
        result = send => result,
    };
}

async fn read_ttrpc_request<R>(
    reader: &mut R,
) -> Result<(MessageHeader, ttrpc::Request, Vec<u8>), ()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let (header, frame) = crate::ttrpc_frame::read_ttrpc_frame(
        reader,
        LimitProfile::local_default().logical_ttrpc_bytes,
    )
    .await
    .map_err(|_| ())?
    .ok_or(())?;
    let request =
        ttrpc::Request::parse_from_bytes(&frame[MESSAGE_HEADER_LENGTH..]).map_err(|_| ())?;
    Ok((header, request, frame))
}

fn ttrpc_frame_header(frame: &[u8]) -> Result<MessageHeader, ()> {
    let header_bytes: [u8; MESSAGE_HEADER_LENGTH] = frame
        .get(..MESSAGE_HEADER_LENGTH)
        .ok_or(())?
        .try_into()
        .map_err(|_| ())?;
    let header = MessageHeader::from(header_bytes);
    if header.length as usize != frame.len().saturating_sub(MESSAGE_HEADER_LENGTH) {
        return Err(());
    }
    Ok(header)
}

fn direct_guest_request_id(request: &ttrpc::Request) -> Result<Option<RequestId>, ()> {
    let bytes = match (request.service.as_str(), request.method.as_str()) {
        ("d2b.guest.v2.GuestService", "Exec") => {
            decode_strict::<GuestExecRequest>(&request.payload, true)
                .map_err(|_| ())?
                .terminal
                .as_ref()
                .and_then(|terminal| terminal.metadata.as_ref())
                .map(|metadata| metadata.request_id.clone())
        }
        ("d2b.guest.v2.GuestService", "OpenShell") => {
            decode_strict::<GuestOpenShellRequest>(&request.payload, true)
                .map_err(|_| ())?
                .terminal
                .as_ref()
                .and_then(|terminal| terminal.metadata.as_ref())
                .map(|metadata| metadata.request_id.clone())
        }
        ("d2b.guest.v2.GuestService", "CancelExec") => {
            decode_strict::<GuestCancelExecRequest>(&request.payload, true)
                .map_err(|_| ())?
                .context
                .as_ref()
                .and_then(|context| context.metadata.as_ref())
                .map(|metadata| metadata.request_id.clone())
        }
        ("d2b.guest.v2.GuestService", "InspectExec") => {
            decode_strict::<GuestInspectExecRequest>(&request.payload, false)
                .map_err(|_| ())?
                .context
                .as_ref()
                .and_then(|context| context.metadata.as_ref())
                .map(|metadata| metadata.request_id.clone())
        }
        ("d2b.guest.v2.GuestService", "OpenExecRetainedLog") => {
            decode_strict::<GuestOpenExecRetainedLogRequest>(&request.payload, true)
                .map_err(|_| ())?
                .context
                .as_ref()
                .and_then(|context| context.metadata.as_ref())
                .map(|metadata| metadata.request_id.clone())
        }
        ("d2b.guest.v2.GuestService", "FileTransfer") => {
            decode_strict::<GuestFileTransferRequest>(&request.payload, true)
                .map_err(|_| ())?
                .context
                .as_ref()
                .and_then(|context| context.metadata.as_ref())
                .map(|metadata| metadata.request_id.clone())
        }
        ("d2b.activation.v2.ActivationService", "Activate") => {
            decode_strict::<common::ServiceRequest>(&request.payload, true)
                .map_err(|_| ())?
                .metadata
                .as_ref()
                .map(|metadata| metadata.request_id.clone())
        }
        ("d2b.activation.v2.ActivationService", "Inspect") => {
            decode_strict::<common::ServiceRequest>(&request.payload, false)
                .map_err(|_| ())?
                .metadata
                .as_ref()
                .map(|metadata| metadata.request_id.clone())
        }
        ("d2b.activation.v2.ActivationService", "Cancel") => {
            decode_strict::<common::CancelRequest>(&request.payload, false).map_err(|_| ())?;
            return Ok(None);
        }
        _ => return Err(()),
    }
    .ok_or(())?;
    RequestId::new(bytes).map(Some).map_err(|_| ())
}

fn ttrpc_context(timeout: Duration) -> ttrpc::context::Context {
    ttrpc::context::with_timeout(timeout.as_nanos().try_into().unwrap_or(i64::MAX))
}

fn map_open_failure(response: &terminal::TerminalOpenResponse) -> TerminalFailure {
    let kind = response
        .error
        .as_ref()
        .and_then(|error| error.kind.enum_value().ok());
    match kind {
        Some(common::ErrorKind::ERROR_KIND_UNAUTHORIZED) => TerminalFailure::Unauthorized,
        Some(common::ErrorKind::ERROR_KIND_NOT_FOUND) => TerminalFailure::NotFound,
        Some(common::ErrorKind::ERROR_KIND_CONFLICT) => TerminalFailure::Conflict,
        Some(common::ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED) => {
            TerminalFailure::ResourceExhausted
        }
        Some(common::ErrorKind::ERROR_KIND_GENERATION_MISMATCH) => {
            TerminalFailure::GenerationMismatch
        }
        Some(common::ErrorKind::ERROR_KIND_INVALID_REQUEST) => TerminalFailure::InvalidSelection,
        _ => TerminalFailure::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use d2b_contracts::v2_component_session::{
        CloseReason, GuestIdentityBindingV1, Remediation, SessionErrorCode,
    };
    use d2b_session::{Cancellation, OwnedAttachment, RequestRegistry, SessionError, SessionEvent};
    use protobuf::EnumOrUnknown;

    struct FakeGuestDriver {
        generation: u64,
        ttrpc_send: mpsc::UnboundedSender<Vec<u8>>,
        ttrpc_receive: tokio::sync::Mutex<mpsc::UnboundedReceiver<Vec<u8>>>,
        stream_send: mpsc::UnboundedSender<StreamEvent>,
        stream_receive: tokio::sync::Mutex<mpsc::UnboundedReceiver<StreamEvent>>,
        validator: Mutex<Option<TerminalStreamValidator>>,
        file_validator: Mutex<Option<FileTransferStreamValidator>>,
        file_payload: Mutex<Vec<u8>>,
        server_sequence: Mutex<u64>,
        opened: AtomicUsize,
        credited: AtomicUsize,
        closed: AtomicUsize,
        activation_calls: AtomicUsize,
    }

    impl FakeGuestDriver {
        fn new(generation: u64) -> Arc<Self> {
            let (ttrpc_send, ttrpc_receive) = mpsc::unbounded_channel();
            let (stream_send, stream_receive) = mpsc::unbounded_channel();
            Arc::new(Self {
                generation,
                ttrpc_send,
                ttrpc_receive: tokio::sync::Mutex::new(ttrpc_receive),
                stream_send,
                stream_receive: tokio::sync::Mutex::new(stream_receive),
                validator: Mutex::new(None),
                file_validator: Mutex::new(None),
                file_payload: Mutex::new(Vec::new()),
                server_sequence: Mutex::new(0),
                opened: AtomicUsize::new(0),
                credited: AtomicUsize::new(0),
                closed: AtomicUsize::new(0),
                activation_calls: AtomicUsize::new(0),
            })
        }

        fn handle_ttrpc(&self, frame: Vec<u8>) -> d2b_session::Result<()> {
            let header = ttrpc_frame_header(&frame)
                .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
            let request = ttrpc::Request::parse_from_bytes(
                frame
                    .get(MESSAGE_HEADER_LENGTH..)
                    .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?,
            )
            .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
            if request.service == "d2b.activation.v2.ActivationService" {
                self.activation_calls.fetch_add(1, Ordering::AcqRel);
                let payload = match request.method.as_str() {
                    "Activate" | "Inspect" => {
                        let mutation = request.method == "Activate";
                        let request =
                            decode_strict::<common::ServiceRequest>(&request.payload, mutation)
                                .map_err(|_| {
                                    SessionError::new(SessionErrorCode::InternalInvariant)
                                })?;
                        let digest = Sha256::digest(request.operation_id.as_bytes()).to_vec();
                        common::ServiceResponse {
                            outcome: EnumOrUnknown::new(if mutation {
                                common::Outcome::OUTCOME_ACCEPTED
                            } else {
                                common::Outcome::OUTCOME_SUCCEEDED
                            }),
                            operation_id: request.operation_id.clone(),
                            resource_handle: request.operation_id,
                            result_digest: digest.clone(),
                            observations: vec![common::Observation {
                                resource_id: request.resource_id,
                                state: EnumOrUnknown::new(if mutation {
                                    common::ObservationState::OBSERVATION_STATE_RUNNING
                                } else {
                                    common::ObservationState::OBSERVATION_STATE_READY
                                }),
                                generation: self.generation,
                                digest,
                                ..Default::default()
                            }],
                            ..Default::default()
                        }
                        .write_to_bytes()
                        .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?
                    }
                    "Cancel" => {
                        decode_strict::<common::CancelRequest>(&request.payload, false)
                            .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
                        common::CancelResponse {
                            outcome: EnumOrUnknown::new(
                                common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED,
                            ),
                            ..Default::default()
                        }
                        .write_to_bytes()
                        .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?
                    }
                    _ => return Err(SessionError::new(SessionErrorCode::InternalInvariant)),
                };
                return self.respond(header, payload);
            }
            if request.service == "d2b.guest.v2.GuestService" && request.method == "FileTransfer" {
                let transfer = decode_strict::<GuestFileTransferRequest>(&request.payload, true)
                    .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
                let context = transfer
                    .context
                    .as_ref()
                    .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
                let metadata = context
                    .metadata
                    .as_ref()
                    .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
                let response = terminal::TerminalOpenResponse {
                    outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_ACCEPTED),
                    operation_id: context.operation_id.clone(),
                    stream_id: "stream-301".to_owned(),
                    session_generation: self.generation,
                    request_id: metadata.request_id.clone(),
                    resource_handle: "activation-payload-1".to_owned(),
                    ..Default::default()
                };
                *self.file_validator.lock().unwrap() = Some(
                    FileTransferStreamValidator::new(&transfer, &response)
                        .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?,
                );
                *self.server_sequence.lock().unwrap() = 0;
                return self.respond(
                    header,
                    response
                        .write_to_bytes()
                        .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?,
                );
            }
            let (terminal_request, kind) = match request.method.as_str() {
                "Exec" => (
                    decode_strict::<GuestExecRequest>(&request.payload, true)
                        .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?
                        .terminal
                        .into_option()
                        .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?,
                    terminal::TerminalKind::TERMINAL_KIND_EXEC,
                ),
                "OpenShell" => (
                    decode_strict::<GuestOpenShellRequest>(&request.payload, true)
                        .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?
                        .terminal
                        .into_option()
                        .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?,
                    terminal::TerminalKind::TERMINAL_KIND_SHELL,
                ),
                _ => return Err(SessionError::new(SessionErrorCode::InternalInvariant)),
            };
            let metadata = terminal_request
                .metadata
                .as_ref()
                .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
            let request_id: [u8; 16] = metadata
                .request_id
                .as_slice()
                .try_into()
                .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
            *self.validator.lock().unwrap() = Some(
                TerminalStreamValidator::new(
                    kind,
                    self.generation,
                    request_id,
                    terminal_request.operation_id.clone(),
                    "guest-resource-1",
                )
                .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?,
            );
            let response = terminal::TerminalOpenResponse {
                outcome: EnumOrUnknown::new(common::Outcome::OUTCOME_ACCEPTED),
                operation_id: terminal_request.operation_id,
                stream_id: "stream-300".to_owned(),
                session_generation: self.generation,
                request_id: metadata.request_id.clone(),
                resource_handle: "guest-resource-1".to_owned(),
                ..Default::default()
            };
            let body = response
                .write_to_bytes()
                .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
            self.respond(header, body)
        }

        fn respond(&self, header: MessageHeader, body: Vec<u8>) -> d2b_session::Result<()> {
            let response = ttrpc::Response {
                payload: body,
                ..Default::default()
            }
            .write_to_bytes()
            .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
            let mut encoded = Vec::from(MessageHeader::new_response(
                header.stream_id,
                u32::try_from(response.len())
                    .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?,
            ));
            encoded.extend_from_slice(&response);
            self.ttrpc_send
                .send(encoded)
                .map_err(|_| SessionError::new(SessionErrorCode::SessionDisconnected))
        }

        fn handle_stream(&self, stream: StreamId, bytes: Vec<u8>) -> d2b_session::Result<()> {
            if stream.channel().value() == 301 {
                return self.handle_file_stream(stream, bytes);
            }
            let frame = terminal::TerminalStreamFrame::parse_from_bytes(&bytes)
                .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
            let mut validator = self.validator.lock().unwrap();
            let validator = validator
                .as_mut()
                .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
            validator
                .accept(TerminalFrameDirection::ClientToServer, &frame)
                .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
            let payload = match frame.frame {
                Some(terminal_stream_frame::Frame::Select(selection)) => {
                    let (kind, tty) = match selection.selection {
                        Some(terminal::terminal_selection::Selection::Exec(exec)) => {
                            (terminal::TerminalKind::TERMINAL_KIND_EXEC, exec.tty)
                        }
                        Some(terminal::terminal_selection::Selection::Shell(_)) => {
                            (terminal::TerminalKind::TERMINAL_KIND_SHELL, true)
                        }
                        _ => return Err(SessionError::new(SessionErrorCode::InternalInvariant)),
                    };
                    terminal_stream_frame::Frame::Started(terminal::TerminalStarted {
                        kind: EnumOrUnknown::new(kind),
                        tty,
                        ..Default::default()
                    })
                }
                Some(terminal_stream_frame::Frame::Stdin(stdin)) => {
                    terminal_stream_frame::Frame::Status(terminal::TerminalStatus {
                        status: EnumOrUnknown::new(
                            terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_ACCEPTED,
                        ),
                        next_stdin_offset: stdin.offset + stdin.data.len() as u64,
                        ..Default::default()
                    })
                }
                Some(terminal_stream_frame::Frame::Cancel(_)) => {
                    terminal_stream_frame::Frame::Outcome(terminal::TerminalOutcome {
                        outcome: Some(terminal::terminal_outcome::Outcome::Cancelled(
                            terminal::TerminalCancelled::default(),
                        )),
                        ..Default::default()
                    })
                }
                _ => return Err(SessionError::new(SessionErrorCode::InternalInvariant)),
            };
            let mut sequence = self.server_sequence.lock().unwrap();
            let response = terminal::TerminalStreamFrame {
                session_generation: frame.session_generation,
                request_id: frame.request_id,
                sequence: *sequence,
                operation_id: frame.operation_id,
                resource_handle: frame.resource_handle,
                frame: Some(payload),
                ..Default::default()
            };
            validator
                .accept(TerminalFrameDirection::ServerToClient, &response)
                .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
            *sequence += 1;
            self.stream_send
                .send(StreamEvent::Data {
                    stream,
                    bytes: response
                        .write_to_bytes()
                        .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?,
                })
                .map_err(|_| SessionError::new(SessionErrorCode::SessionDisconnected))
        }

        fn handle_file_stream(&self, stream: StreamId, bytes: Vec<u8>) -> d2b_session::Result<()> {
            let frame = GuestFileTransferFrame::parse_from_bytes(&bytes)
                .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
            let mut validator = self.file_validator.lock().unwrap();
            let validator = validator
                .as_mut()
                .ok_or_else(|| SessionError::new(SessionErrorCode::InternalInvariant))?;
            validator
                .accept(GuestStreamDirection::ClientToServer, &frame)
                .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
            let payload = match frame.frame {
                Some(guest_file_transfer_frame::Frame::Start(start)) => {
                    guest_file_transfer_frame::Frame::Credit(
                        d2b_contracts::v2_services::guest::GuestFileTransferCredit {
                            bytes: MAX_GUEST_FILE_CHUNK_BYTES as u32,
                            next_offset: start.offset,
                            ..Default::default()
                        },
                    )
                }
                Some(guest_file_transfer_frame::Frame::Chunk(chunk)) if chunk.eof => {
                    *self.file_payload.lock().unwrap() = chunk.data;
                    guest_file_transfer_frame::Frame::Complete(
                        d2b_contracts::v2_services::guest::GuestFileTransferComplete {
                            total_size: chunk.total_size,
                            digest: chunk.final_digest,
                            ..Default::default()
                        },
                    )
                }
                _ => return Err(SessionError::new(SessionErrorCode::InternalInvariant)),
            };
            let mut sequence = self.server_sequence.lock().unwrap();
            let response = GuestFileTransferFrame {
                session_generation: frame.session_generation,
                request_id: frame.request_id,
                sequence: *sequence,
                operation_id: frame.operation_id,
                resource_handle: frame.resource_handle,
                frame: Some(payload),
                ..Default::default()
            };
            validator
                .accept(GuestStreamDirection::ServerToClient, &response)
                .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?;
            *sequence += 1;
            self.stream_send
                .send(StreamEvent::Data {
                    stream,
                    bytes: response
                        .write_to_bytes()
                        .map_err(|_| SessionError::new(SessionErrorCode::InternalInvariant))?,
                })
                .map_err(|_| SessionError::new(SessionErrorCode::SessionDisconnected))
        }
    }

    #[async_trait]
    impl ComponentSessionDriver for FakeGuestDriver {
        fn generation(&self) -> u64 {
            self.generation
        }

        async fn start_ttrpc(&self, _: RequestId, frame: Vec<u8>) -> d2b_session::Result<()> {
            self.handle_ttrpc(frame)
        }

        async fn complete_ttrpc(&self, _: RequestId) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn cancel(&self, _: u64, _: RequestId) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn send_ttrpc(&self, frame: Vec<u8>) -> d2b_session::Result<()> {
            self.handle_ttrpc(frame)
        }

        async fn receive_ttrpc(&self) -> d2b_session::Result<Vec<u8>> {
            self.ttrpc_receive
                .lock()
                .await
                .recv()
                .await
                .ok_or_else(|| SessionError::new(SessionErrorCode::SessionDisconnected))
        }

        async fn register_inbound_call(
            &self,
            request_id: RequestId,
        ) -> d2b_session::Result<Cancellation> {
            RequestRegistry::new(self.generation)?.register(request_id)
        }

        async fn complete_inbound_call(&self, _: RequestId) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn remove_inbound_call(&self, _: RequestId) -> d2b_session::Result<bool> {
            Ok(true)
        }

        async fn send_attachments(&self, _: Vec<OwnedAttachment>) -> d2b_session::Result<()> {
            Err(SessionError::new(SessionErrorCode::InternalInvariant))
        }

        async fn receive_attachments(&self) -> d2b_session::Result<Vec<OwnedAttachment>> {
            Err(SessionError::new(SessionErrorCode::InternalInvariant))
        }

        async fn open_named_stream(&self, _: StreamId, _: u32, _: u32) -> d2b_session::Result<()> {
            self.opened.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        async fn send_named_stream(
            &self,
            stream: StreamId,
            bytes: Vec<u8>,
        ) -> d2b_session::Result<()> {
            self.handle_stream(stream, bytes)
        }

        async fn receive_named_stream(&self) -> d2b_session::Result<StreamEvent> {
            self.stream_receive
                .lock()
                .await
                .recv()
                .await
                .ok_or_else(|| SessionError::new(SessionErrorCode::SessionDisconnected))
        }

        async fn grant_named_stream_credit(
            &self,
            _: StreamId,
            bytes: u32,
        ) -> d2b_session::Result<()> {
            self.credited.fetch_add(bytes as usize, Ordering::AcqRel);
            Ok(())
        }

        async fn close_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
            self.closed.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        async fn reset_named_stream(&self, _: StreamId) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn drive_keepalive(&self, _: std::time::Instant) -> d2b_session::Result<()> {
            Ok(())
        }

        async fn receive_control(&self) -> d2b_session::Result<SessionEvent> {
            std::future::pending().await
        }

        async fn close(&self, _: CloseReason, _: Remediation) -> d2b_session::Result<()> {
            Ok(())
        }
    }

    fn terminal_request(generation: u64) -> terminal::TerminalOpenRequest {
        terminal::TerminalOpenRequest {
            metadata: MessageField::some(common::RequestMetadata {
                request_id: vec![1; 16],
                correlation_id: "correlation-1".to_owned(),
                trace_id: vec![2; 16],
                idempotency_key: vec![3; 16],
                issued_at_unix_ms: 1,
                expires_at_unix_ms: 2,
                session_generation: generation,
                ..Default::default()
            }),
            scope: MessageField::some(common::IdentityScope {
                realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
                workload_id: "baaaaaaaaaaaaaaaaaaq".to_owned(),
                ..Default::default()
            }),
            resource_id: "corp-vm".to_owned(),
            operation_id: "operation-1".to_owned(),
            request_digest: vec![4; 32],
            ..Default::default()
        }
    }

    fn exec_selection() -> terminal::TerminalSelection {
        terminal::TerminalSelection {
            selection: Some(terminal::terminal_selection::Selection::Exec(
                terminal::ExecSelection {
                    authority: EnumOrUnknown::new(
                        terminal::ExecAuthority::EXEC_AUTHORITY_ADMIN_ARBITRARY,
                    ),
                    selection: Some(terminal::exec_selection::Selection::Arbitrary(
                        terminal::ArbitraryExecSelection {
                            argv: vec![b"true".to_vec()],
                            ..Default::default()
                        },
                    )),
                    tty: false,
                    ..Default::default()
                },
            )),
            ..Default::default()
        }
    }

    fn activation_request(generation: u64, mutation: bool) -> common::ServiceRequest {
        common::ServiceRequest {
            metadata: MessageField::some(common::RequestMetadata {
                request_id: vec![0x31 + u8::from(mutation); 16],
                idempotency_key: if mutation { vec![0x41; 32] } else { Vec::new() },
                issued_at_unix_ms: 1,
                expires_at_unix_ms: 2,
                session_generation: generation,
                ..Default::default()
            }),
            scope: MessageField::some(common::IdentityScope {
                realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
                workload_id: "baaaaaaaaaaaaaaaaaaq".to_owned(),
                ..Default::default()
            }),
            resource_id: "activation-baaaaaaaaaaaaaaaaaaq".to_owned(),
            operation_id: "activation-0123456789abcdef0123456789abcdef".to_owned(),
            request_digest: if mutation { vec![0x51; 32] } else { Vec::new() },
            desired_state: EnumOrUnknown::new(if mutation {
                common::DesiredState::DESIRED_STATE_RUNNING
            } else {
                common::DesiredState::DESIRED_STATE_UNSPECIFIED
            }),
            ..Default::default()
        }
    }

    fn activation_transfer_request(generation: u64, payload: &[u8]) -> GuestFileTransferRequest {
        GuestFileTransferRequest {
                context: MessageField::some(
                    d2b_contracts::v2_services::guest::GuestOperationContext {
                        metadata: MessageField::some(common::RequestMetadata {
                            request_id: vec![0x61; 16],
                            idempotency_key: vec![0x62; 32],
                            issued_at_unix_ms: 1,
                            expires_at_unix_ms: 2,
                            session_generation: generation,
                            ..Default::default()
                        }),
                        scope: MessageField::some(common::IdentityScope {
                            realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
                            workload_id: "baaaaaaaaaaaaaaaaaaq".to_owned(),
                            ..Default::default()
                        }),
                        operation_id: "activation-payload-1".to_owned(),
                        request_digest: Sha256::digest(payload).to_vec(),
                        ..Default::default()
                    },
                ),
                artifact: EnumOrUnknown::new(
                    d2b_contracts::v2_services::guest::GuestArtifactId::GUEST_ARTIFACT_ID_ACTIVATION_PAYLOAD,
                ),
                configured_intent_id: "activation-baaaaaaaaaaaaaaaaaaq".to_owned(),
                direction: EnumOrUnknown::new(
                    d2b_contracts::v2_services::guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_HOST_TO_GUEST,
                ),
                declared_size: payload.len() as u64,
                expected_digest: Sha256::digest(payload).to_vec(),
                ..Default::default()
        }
    }

    struct MaterialConnector {
        driver: Arc<FakeGuestDriver>,
        acquired: AtomicUsize,
        connected: AtomicUsize,
    }

    impl fmt::Debug for MaterialConnector {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("MaterialConnector(REDACTED)")
        }
    }

    #[async_trait]
    impl GuestTerminalConnector for MaterialConnector {
        async fn acquire_material(
            &self,
            workload: &str,
        ) -> Result<GuestSessionCredentialV1, TerminalFailure> {
            assert_eq!(workload, "corp-vm");
            self.acquired.fetch_add(1, Ordering::AcqRel);
            GuestSessionCredentialV1::new(
                9,
                [1; 32],
                [2; 32],
                GuestIdentityBindingV1::Enrolled {
                    guest_identity_digest: [3; 32],
                    guest_static_public_key: [4; 32],
                },
                None,
            )
            .map_err(|_| TerminalFailure::Internal)
        }

        async fn connect_with_material(
            &self,
            workload: &str,
            material: GuestSessionCredentialV1,
        ) -> Result<Arc<GuestTerminalSession>, TerminalFailure> {
            assert_eq!(workload, "corp-vm");
            assert_eq!(self.acquired.load(Ordering::Acquire), 1);
            assert_eq!(material.session_generation(), 9);
            assert_eq!(material.channel_binding(), &[2; 32]);
            self.connected.fetch_add(1, Ordering::AcqRel);
            let driver: Arc<dyn ComponentSessionDriver> = self.driver.clone();
            Ok(GuestTerminalSession::from_driver(driver))
        }
    }

    #[tokio::test]
    async fn typed_guest_exec_bridges_terminal_lifecycle_and_bindings() {
        let driver = FakeGuestDriver::new(9);
        let erased: Arc<dyn ComponentSessionDriver> = driver.clone();
        let session = GuestTerminalSession::from_driver(erased);
        let request = GuestExecRequest {
            terminal: MessageField::some(terminal_request(9)),
            ..Default::default()
        };
        let TerminalOpenResult::Active { started, mut owner } = session
            .open_exec(request, exec_selection(), Duration::from_secs(1))
            .await
            .expect("typed guest exec open")
        else {
            panic!("expected active guest terminal");
        };
        assert_eq!(
            started.kind.enum_value().unwrap(),
            terminal::TerminalKind::TERMINAL_KIND_EXEC
        );
        let events = owner
            .command(TerminalCommand::Stdin {
                offset: 0,
                data: b"input".to_vec(),
                eof: false,
            })
            .await
            .expect("guest stdin");
        if events.is_empty() {
            tokio::task::yield_now().await;
        }

        let mut events = events;
        events.extend(owner.poll().await.expect("guest status poll"));
        assert!(events.iter().any(|event| matches!(
            event,
            TerminalOwnerEvent::Status {
                status: terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_ACCEPTED,
                ..
            }
        )));
        let events = owner
            .finish(TerminalFinish::Cancel)
            .await
            .expect("guest cancel");
        assert!(events.iter().any(|event| matches!(
            event,
            TerminalOwnerEvent::Outcome(terminal::TerminalOutcome {
                outcome: Some(terminal::terminal_outcome::Outcome::Cancelled(_)),
                ..
            })
        )));
        assert_eq!(driver.opened.load(Ordering::Acquire), 1);
        assert!(driver.credited.load(Ordering::Acquire) > 0);
        assert_eq!(driver.closed.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn typed_activation_client_routes_on_the_verified_direct_session() {
        let driver = FakeGuestDriver::new(9);
        let erased: Arc<dyn ComponentSessionDriver> = driver.clone();
        let session = GuestTerminalSession::from_driver(erased);
        let started = session
            .activation_activate(activation_request(9, true), Duration::from_secs(1))
            .await
            .expect("typed activation start");
        assert_eq!(
            started.outcome.enum_value().unwrap(),
            common::Outcome::OUTCOME_ACCEPTED
        );
        let inspected = session
            .activation_inspect(activation_request(9, false), Duration::from_secs(1))
            .await
            .expect("typed activation inspect");
        assert_eq!(
            inspected.outcome.enum_value().unwrap(),
            common::Outcome::OUTCOME_SUCCEEDED
        );
        let cancelled = session
            .activation_cancel(
                common::CancelRequest {
                    session_generation: 9,
                    request_id: vec![0x32; 16],
                    ..Default::default()
                },
                Duration::from_secs(1),
            )
            .await
            .expect("typed activation cancel");
        assert_eq!(
            cancelled.outcome.enum_value().unwrap(),
            common::CancelOutcome::CANCEL_OUTCOME_CANCELLATION_SIGNALLED
        );
        assert_eq!(driver.activation_calls.load(Ordering::Acquire), 3);
    }

    #[tokio::test]
    async fn activation_payload_uses_the_typed_direct_file_transfer() {
        let driver = FakeGuestDriver::new(9);
        let erased: Arc<dyn ComponentSessionDriver> = driver.clone();
        let session = GuestTerminalSession::from_driver(erased);
        let payload = br#"{"schemaVersion":1}"#;
        session
            .upload_activation_payload(
                activation_transfer_request(9, payload),
                payload,
                Duration::from_secs(1),
            )
            .await
            .expect("typed activation payload transfer");
        assert_eq!(driver.file_payload.lock().unwrap().as_slice(), payload);
        assert_eq!(driver.opened.load(Ordering::Acquire), 1);
        assert_eq!(driver.closed.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn unavailable_guest_connector_fails_before_public_stream_allocation() {
        assert_eq!(
            UnavailableGuestTerminalConnector
                .connect("corp-vm")
                .await
                .unwrap_err(),
            TerminalFailure::Unavailable
        );
    }

    #[tokio::test]
    async fn connector_acquires_typed_material_before_session_connection() {
        let connector = MaterialConnector {
            driver: FakeGuestDriver::new(9),
            acquired: AtomicUsize::new(0),
            connected: AtomicUsize::new(0),
        };
        let session = connector.connect("corp-vm").await.unwrap();
        assert_eq!(session.generation(), 9);
        assert_eq!(connector.acquired.load(Ordering::Acquire), 1);
        assert_eq!(connector.connected.load(Ordering::Acquire), 1);
    }

    #[test]
    fn quit_signal_is_forwarded_without_collapsing_kind() {
        let terminal_stream_frame::Frame::Signal(signal) =
            terminal_command_payload(TerminalCommand::Signal {
                operation_sequence: 7,
                signal: terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_QUIT,
            })
        else {
            panic!("expected signal frame");
        };
        assert_eq!(signal.operation_sequence, 7);
        assert_eq!(
            signal.signal.enum_value().unwrap(),
            terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_QUIT
        );
    }
}
