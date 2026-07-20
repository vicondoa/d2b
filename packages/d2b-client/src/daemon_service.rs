//! Typed client for the authenticated `d2b.daemon.v2` service.

use std::{
    fmt,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use d2b_contracts::v2_identity::WorkloadName;
use d2b_contracts::v2_services::{
    StrictWireMessage, TerminalFrameDirection, TerminalStreamValidator,
    common::{self, ServiceRequest},
    daemon, encode_strict,
    terminal::{self, TerminalOpenRequest, TerminalOpenResponse, TerminalSelection},
    validate_terminal_open_response_for_request,
};
use protobuf::{EnumOrUnknown, Message};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::{
    CallOptions, CancellationToken, ClientError, ConnectedClient, MetadataInput, NamedStream,
    RemoteErrorKind, Response, RetryClass, RetryPolicy, ServiceKind,
};

const CALL_LIFETIME: Duration = Duration::from_secs(30);
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum DaemonMethod {
    Resolve = 0,
    ListRealms = 1,
    ListWorkloads = 2,
    Inspect = 3,
    Apply = 4,
    Start = 5,
    Stop = 6,
    Restart = 7,
    Exec = 8,
    Shell = 9,
    OpenConsole = 10,
    ExportAudit = 11,
    Cancel = 12,
}

impl DaemonMethod {
    const fn terminal_kind(self) -> Option<terminal::TerminalKind> {
        match self {
            Self::Exec => Some(terminal::TerminalKind::TERMINAL_KIND_EXEC),
            Self::Shell => Some(terminal::TerminalKind::TERMINAL_KIND_SHELL),
            Self::OpenConsole => Some(terminal::TerminalKind::TERMINAL_KIND_CONSOLE),
            _ => None,
        }
    }
}

pub struct DaemonClient {
    inner: ConnectedClient,
}

#[derive(Debug, Clone)]
pub struct DaemonLifecycleRequest<'a> {
    pub method: DaemonMethod,
    pub resource_id: &'a str,
    pub desired_state: common::DesiredState,
    pub operation_id: &'a str,
    pub request_digest: [u8; 32],
}

impl fmt::Debug for DaemonClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonClient")
            .field("session", &"<authenticated>")
            .finish()
    }
}

impl DaemonClient {
    pub fn new(inner: ConnectedClient) -> Result<Self, ClientError> {
        if inner.service().kind() != ServiceKind::Daemon {
            return Err(ClientError::InvalidService);
        }
        Ok(Self { inner })
    }

    pub const fn session_generation(&self) -> u64 {
        self.inner.session_generation()
    }

    pub fn connected(&self) -> &ConnectedClient {
        &self.inner
    }

    pub fn guest_proxy(&self, workload: &WorkloadName) -> Result<crate::GuestClient, ClientError> {
        crate::GuestClient::new(self.inner.local_daemon_guest_proxy(workload)?)
    }

    pub async fn resolve(
        &self,
        resource_id: Option<&str>,
        options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<Response, ClientError> {
        let mut request = ServiceRequest::new();
        request.resource_id = resource_id.unwrap_or_default().to_owned();
        self.inner
            .invoke(
                self.method(DaemonMethod::Resolve)?,
                request,
                options,
                cancellation,
            )
            .await
    }

    pub async fn list_realms(
        &self,
        page_size: u32,
        page_cursor: Option<&str>,
        options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<daemon::ListRealmsResponse, ClientError> {
        let request = page_request(None, page_size, page_cursor);
        let method = self.method(DaemonMethod::ListRealms)?;
        let (request, context) = self
            .inner
            .prepare_typed_request(method, request, &options)?;
        let response = self
            .call_with_cancellation(
                self.inner
                    .service()
                    .generated()
                    .daemon()?
                    .list_realms(context, &request),
                &options.metadata,
                cancellation,
            )
            .await?;
        response
            .validate_wire(false)
            .map_err(ClientError::ServiceContract)?;
        ensure_daemon_outcome(&response.outcome, response.error.as_ref())?;
        Ok(response)
    }

    pub async fn list_workloads(
        &self,
        resource_id: Option<&str>,
        page_size: u32,
        page_cursor: Option<&str>,
        options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<daemon::ListWorkloadsResponse, ClientError> {
        let request = page_request(resource_id, page_size, page_cursor);
        let method = self.method(DaemonMethod::ListWorkloads)?;
        let (request, context) = self
            .inner
            .prepare_typed_request(method, request, &options)?;
        let response = self
            .call_with_cancellation(
                self.inner
                    .service()
                    .generated()
                    .daemon()?
                    .list_workloads(context, &request),
                &options.metadata,
                cancellation,
            )
            .await?;
        response
            .validate_wire(false)
            .map_err(ClientError::ServiceContract)?;
        ensure_daemon_outcome(&response.outcome, response.error.as_ref())?;
        Ok(response)
    }

    pub async fn inspect(
        &self,
        resource_id: Option<&str>,
        page_size: u32,
        page_cursor: Option<&str>,
        options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<daemon::InspectResponse, ClientError> {
        let request = page_request(resource_id, page_size, page_cursor);
        let method = self.method(DaemonMethod::Inspect)?;
        let (request, context) = self
            .inner
            .prepare_typed_request(method, request, &options)?;
        let response = self
            .call_with_cancellation(
                self.inner
                    .service()
                    .generated()
                    .daemon()?
                    .inspect(context, &request),
                &options.metadata,
                cancellation,
            )
            .await?;
        response
            .validate_wire(false)
            .map_err(ClientError::ServiceContract)?;
        ensure_daemon_outcome(&response.outcome, response.error.as_ref())?;
        Ok(response)
    }

    pub async fn lifecycle(
        &self,
        call: DaemonLifecycleRequest<'_>,
        options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<Response, ClientError> {
        if !matches!(
            call.method,
            DaemonMethod::Apply | DaemonMethod::Start | DaemonMethod::Stop | DaemonMethod::Restart
        ) || call.resource_id.is_empty()
            || call.operation_id.is_empty()
        {
            return Err(ClientError::InvalidMethod);
        }
        let mut request = ServiceRequest::new();
        request.resource_id = call.resource_id.to_owned();
        request.operation_id = call.operation_id.to_owned();
        request.request_digest = call.request_digest.to_vec();
        request.desired_state = EnumOrUnknown::new(call.desired_state);
        self.inner
            .invoke(self.method(call.method)?, request, options, cancellation)
            .await
    }

    pub async fn open_terminal(
        &self,
        method: DaemonMethod,
        resource_id: &str,
        operation_id: &str,
        selection: TerminalSelection,
        options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<DaemonTerminal, ClientError> {
        let kind = method.terminal_kind().ok_or(ClientError::InvalidMethod)?;
        if resource_id.is_empty() || operation_id.is_empty() {
            return Err(ClientError::InvalidMetadata);
        }
        let request_id = options.metadata.request_id_bytes();
        let generation = self.session_generation();
        terminal::TerminalStreamFrame {
            session_generation: generation,
            request_id: request_id.to_vec(),
            operation_id: operation_id.to_owned(),
            resource_handle: "preopen-selection".to_owned(),
            frame: Some(terminal::terminal_stream_frame::Frame::Select(
                selection.clone(),
            )),
            ..Default::default()
        }
        .validate_wire(false)
        .map_err(ClientError::ServiceContract)?;
        let selection_digest = selection
            .write_to_bytes()
            .map_err(|_| ClientError::ContractViolation)?;
        let mut request = TerminalOpenRequest::new();
        request.resource_id = resource_id.to_owned();
        request.operation_id = operation_id.to_owned();
        request.request_digest = Sha256::digest(&selection_digest).to_vec();
        let method_handle = self.method(method)?;
        let (request, context) =
            self.inner
                .prepare_terminal_open(method_handle, request, &options)?;
        let generated = self.inner.service().generated().daemon()?;
        let response = match method {
            DaemonMethod::Exec => {
                self.call_with_cancellation(
                    generated.exec(context, &request),
                    &options.metadata,
                    cancellation,
                )
                .await?
            }
            DaemonMethod::Shell => {
                self.call_with_cancellation(
                    generated.shell(context, &request),
                    &options.metadata,
                    cancellation,
                )
                .await?
            }
            DaemonMethod::OpenConsole => {
                self.call_with_cancellation(
                    generated.open_console(context, &request),
                    &options.metadata,
                    cancellation,
                )
                .await?
            }
            _ => return Err(ClientError::InvalidMethod),
        };
        validate_terminal_open_response_for_request(&request, &response)
            .map_err(ClientError::ServiceContract)?;
        ensure_terminal_open_outcome(&response)?;
        Self::terminal_from_open_response(
            &self.inner,
            kind,
            generation,
            request_id,
            operation_id,
            response,
            selection,
        )
        .await
    }

    pub(crate) async fn terminal_from_open_response(
        client: &ConnectedClient,
        kind: terminal::TerminalKind,
        generation: u64,
        request_id: [u8; 16],
        operation_id: &str,
        response: TerminalOpenResponse,
        selection: TerminalSelection,
    ) -> Result<DaemonTerminal, ClientError> {
        let resource_handle = response.resource_handle.clone();
        let retained_log = response.retained_log.as_ref().cloned();
        let mut validator = TerminalStreamValidator::new(
            kind,
            generation,
            request_id,
            operation_id,
            &resource_handle,
        )
        .map_err(ClientError::ServiceContract)?;
        if let Some(retained_log) = response.retained_log.as_ref() {
            validator
                .bind_retained_log_range(retained_log)
                .map_err(ClientError::ServiceContract)?;
        }
        let selection_frame = terminal::TerminalStreamFrame {
            session_generation: generation,
            request_id: request_id.to_vec(),
            sequence: 0,
            operation_id: operation_id.to_owned(),
            resource_handle: resource_handle.clone(),
            frame: Some(terminal::terminal_stream_frame::Frame::Select(selection)),
            ..Default::default()
        };
        validator
            .accept(TerminalFrameDirection::ClientToServer, &selection_frame)
            .map_err(ClientError::ServiceContract)?;
        let selection_bytes =
            encode_strict(&selection_frame, false).map_err(ClientError::ServiceContract)?;
        let stream = client.open_server_stream(&response.stream_id).await?;
        stream.send(&selection_bytes).await?;
        Ok(DaemonTerminal {
            stream,
            state: Mutex::new(TerminalState {
                validator,
                next_client_sequence: 1,
            }),
            generation,
            request_id,
            operation_id: operation_id.to_owned(),
            resource_handle,
            retained_log,
            logical_message_bytes: client.session_limits().logical_named_stream_bytes,
            terminal: AtomicBool::new(false),
        })
    }

    fn method(&self, method: DaemonMethod) -> Result<crate::MethodHandle, ClientError> {
        self.inner.service().method(method as u16)
    }

    async fn call_with_cancellation<T>(
        &self,
        call: impl std::future::Future<Output = ttrpc::Result<T>>,
        metadata: &MetadataInput,
        cancellation: &CancellationToken,
    ) -> Result<T, ClientError> {
        if cancellation.is_cancelled() {
            self.inner.cancel_request(metadata).await;
            return Err(ClientError::Cancelled);
        }
        tokio::select! {
            response = call => response.map_err(map_ttrpc_error),
            () = cancellation.cancelled() => {
                self.inner.cancel_request(metadata).await;
                Err(ClientError::Cancelled)
            }
        }
    }
}

fn page_request(
    resource_id: Option<&str>,
    page_size: u32,
    page_cursor: Option<&str>,
) -> ServiceRequest {
    let mut request = ServiceRequest::new();
    request.resource_id = resource_id.unwrap_or_default().to_owned();
    request.page_size = page_size;
    request.page_cursor = page_cursor.unwrap_or_default().to_owned();
    request
}

struct TerminalState {
    validator: TerminalStreamValidator,
    next_client_sequence: u64,
}

pub struct DaemonTerminal {
    stream: NamedStream,
    state: Mutex<TerminalState>,
    generation: u64,
    request_id: [u8; 16],
    operation_id: String,
    resource_handle: String,
    retained_log: Option<terminal::TerminalRetainedLogRange>,
    logical_message_bytes: u32,
    terminal: AtomicBool,
}

impl fmt::Debug for DaemonTerminal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DaemonTerminal([authenticated, redacted])")
    }
}

impl DaemonTerminal {
    pub const fn session_generation(&self) -> u64 {
        self.generation
    }

    pub fn resource_handle(&self) -> &str {
        &self.resource_handle
    }

    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub fn retained_log_range(&self) -> Option<&terminal::TerminalRetainedLogRange> {
        self.retained_log.as_ref()
    }

    pub async fn send(
        &self,
        frame: terminal::terminal_stream_frame::Frame,
    ) -> Result<(), ClientError> {
        if self.terminal.load(Ordering::Acquire) {
            return Err(ClientError::StreamClosed);
        }
        let mut state = self.state.lock().await;
        let message = terminal::TerminalStreamFrame {
            session_generation: self.generation,
            request_id: self.request_id.to_vec(),
            sequence: state.next_client_sequence,
            operation_id: self.operation_id.clone(),
            resource_handle: self.resource_handle.clone(),
            frame: Some(frame),
            ..Default::default()
        };
        state
            .validator
            .accept(TerminalFrameDirection::ClientToServer, &message)
            .map_err(ClientError::ServiceContract)?;
        let encoded = encode_strict(&message, false).map_err(ClientError::ServiceContract)?;
        if encoded.len() > self.logical_message_bytes as usize {
            return Err(ClientError::StreamLimitExceeded);
        }
        state.next_client_sequence = state
            .next_client_sequence
            .checked_add(1)
            .ok_or(ClientError::ContractViolation)?;
        drop(state);
        self.stream.send(&encoded).await
    }

    pub async fn receive(&self) -> Result<terminal::TerminalStreamFrame, ClientError> {
        if self.terminal.load(Ordering::Acquire) {
            return Err(ClientError::StreamClosed);
        }
        let encoded = self.stream.receive().await?;
        if encoded.len() > self.logical_message_bytes as usize {
            return Err(ClientError::ContractViolation);
        }
        let frame = d2b_contracts::v2_services::decode_strict::<terminal::TerminalStreamFrame>(
            &encoded, false,
        )
        .map_err(ClientError::ServiceContract)?;
        let mut state = self.state.lock().await;
        state
            .validator
            .accept_transport_credit(
                u32::try_from(encoded.len()).map_err(|_| ClientError::ContractViolation)?,
            )
            .map_err(ClientError::ServiceContract)?;
        state
            .validator
            .accept(TerminalFrameDirection::ServerToClient, &frame)
            .map_err(ClientError::ServiceContract)?;
        if state.validator.is_terminal() {
            self.terminal.store(true, Ordering::Release);
        }
        Ok(frame)
    }

    pub async fn close_transport(&self) -> Result<(), ClientError> {
        self.state
            .lock()
            .await
            .validator
            .accept_transport_close()
            .map_err(ClientError::ServiceContract)?;
        self.stream.close().await
    }

    pub async fn reset_transport(&self) -> Result<(), ClientError> {
        self.state
            .lock()
            .await
            .validator
            .accept_transport_reset()
            .map_err(ClientError::ServiceContract)?;
        self.stream.cancel().await
    }

    pub fn is_terminal(&self) -> bool {
        self.terminal.load(Ordering::Acquire) || self.stream.is_terminal()
    }
}

pub fn daemon_call_options(mutating: bool) -> Result<CallOptions, ClientError> {
    let issued: u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .try_into()
        .map_err(|_| ClientError::InvalidMetadata)?;
    let expires = issued
        .checked_add(
            CALL_LIFETIME
                .as_millis()
                .try_into()
                .map_err(|_| ClientError::InvalidMetadata)?,
        )
        .ok_or(ClientError::InvalidMetadata)?;
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut digest = Sha256::new();
    digest.update(b"d2b-client-request-v2\0");
    digest.update(std::process::id().to_be_bytes());
    digest.update(issued.to_be_bytes());
    digest.update(sequence.to_be_bytes());
    let request_id: [u8; 16] = digest.finalize()[..16]
        .try_into()
        .expect("digest prefix has fixed length");
    let metadata = MetadataInput::new(request_id, issued, expires)?;
    Ok(CallOptions {
        metadata: if mutating {
            let mut key = Vec::with_capacity(24);
            key.extend_from_slice(&request_id);
            key.extend_from_slice(&sequence.to_be_bytes());
            metadata.with_idempotency(key)?
        } else {
            metadata
        },
        retry: RetryPolicy::no_retry(),
    })
}

fn ensure_daemon_outcome(
    outcome: &EnumOrUnknown<common::Outcome>,
    error: Option<&common::ErrorEnvelope>,
) -> Result<(), ClientError> {
    match outcome
        .enum_value()
        .map_err(|_| ClientError::ContractViolation)?
    {
        common::Outcome::OUTCOME_SUCCEEDED | common::Outcome::OUTCOME_DEGRADED => Ok(()),
        common::Outcome::OUTCOME_DENIED
        | common::Outcome::OUTCOME_CANCELLED
        | common::Outcome::OUTCOME_FAILED => {
            Err(remote_error(error.ok_or(ClientError::ContractViolation)?))
        }
        _ => Err(ClientError::ContractViolation),
    }
}

pub(crate) fn ensure_terminal_open_outcome(
    response: &TerminalOpenResponse,
) -> Result<(), ClientError> {
    match response
        .outcome
        .enum_value()
        .map_err(|_| ClientError::ContractViolation)?
    {
        common::Outcome::OUTCOME_ACCEPTED => Ok(()),
        common::Outcome::OUTCOME_DENIED
        | common::Outcome::OUTCOME_CANCELLED
        | common::Outcome::OUTCOME_FAILED => Err(remote_error(
            response
                .error
                .as_ref()
                .ok_or(ClientError::ContractViolation)?,
        )),
        _ => Err(ClientError::ContractViolation),
    }
}

pub(crate) fn remote_error(error: &common::ErrorEnvelope) -> ClientError {
    let kind = match error.kind.enum_value_or_default() {
        common::ErrorKind::ERROR_KIND_INVALID_REQUEST => RemoteErrorKind::InvalidRequest,
        common::ErrorKind::ERROR_KIND_UNAUTHENTICATED => RemoteErrorKind::Unauthorized,
        common::ErrorKind::ERROR_KIND_UNAUTHORIZED
        | common::ErrorKind::ERROR_KIND_CAPABILITY_DENIED => RemoteErrorKind::Forbidden,
        common::ErrorKind::ERROR_KIND_NOT_FOUND => RemoteErrorKind::NotFound,
        common::ErrorKind::ERROR_KIND_CONFLICT => RemoteErrorKind::Conflict,
        common::ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED => RemoteErrorKind::ResourceExhausted,
        common::ErrorKind::ERROR_KIND_UNAVAILABLE => RemoteErrorKind::Unavailable,
        common::ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED => RemoteErrorKind::DeadlineExceeded,
        common::ErrorKind::ERROR_KIND_CANCELLED => RemoteErrorKind::Cancelled,
        common::ErrorKind::ERROR_KIND_GENERATION_MISMATCH => RemoteErrorKind::GenerationMismatch,
        common::ErrorKind::ERROR_KIND_INVARIANT_VIOLATION => RemoteErrorKind::FailedPrecondition,
        _ => RemoteErrorKind::Internal,
    };
    let retry = match error.retry.enum_value_or_default() {
        common::RetryClass::RETRY_CLASS_SAME_OPERATION => RetryClass::Safe,
        common::RetryClass::RETRY_CLASS_AFTER_OBSERVATION
        | common::RetryClass::RETRY_CLASS_AFTER_INTERACTION => RetryClass::Observe,
        _ => RetryClass::Never,
    };
    ClientError::Remote { kind, retry }
}

pub(crate) fn map_ttrpc_error(error: ttrpc::Error) -> ClientError {
    match error {
        ttrpc::Error::RpcStatus(status) => {
            let kind = match status.code() {
                ttrpc::Code::INVALID_ARGUMENT => RemoteErrorKind::InvalidRequest,
                ttrpc::Code::UNAUTHENTICATED => RemoteErrorKind::Unauthorized,
                ttrpc::Code::PERMISSION_DENIED => RemoteErrorKind::Forbidden,
                ttrpc::Code::NOT_FOUND => RemoteErrorKind::NotFound,
                ttrpc::Code::ALREADY_EXISTS | ttrpc::Code::ABORTED => RemoteErrorKind::Conflict,
                ttrpc::Code::RESOURCE_EXHAUSTED => RemoteErrorKind::ResourceExhausted,
                ttrpc::Code::UNAVAILABLE => RemoteErrorKind::Unavailable,
                ttrpc::Code::DEADLINE_EXCEEDED => RemoteErrorKind::DeadlineExceeded,
                ttrpc::Code::CANCELLED => RemoteErrorKind::Cancelled,
                ttrpc::Code::FAILED_PRECONDITION => RemoteErrorKind::FailedPrecondition,
                _ => RemoteErrorKind::Internal,
            };
            ClientError::Remote {
                kind,
                retry: RetryClass::Never,
            }
        }
        _ => ClientError::TransportFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v2_services::RedactedTerminalFrame;

    #[test]
    fn terminal_debug_never_contains_frame_payloads() {
        let frame = terminal::TerminalStreamFrame {
            session_generation: 7,
            request_id: vec![9; 16],
            sequence: 4,
            operation_id: "op-redacted".to_owned(),
            resource_handle: "resource-redacted".to_owned(),
            frame: Some(terminal::terminal_stream_frame::Frame::Stdin(
                terminal::TerminalStdin {
                    offset: 0,
                    data: b"do-not-log".to_vec(),
                    eof: false,
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let rendered = format!("{:?}", RedactedTerminalFrame(&frame));
        assert!(rendered.contains("stdin"));
        assert!(!rendered.contains("do-not-log"));
        assert!(!rendered.contains(&"09".repeat(16)));
    }
}
