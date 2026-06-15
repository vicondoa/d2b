//! One-shot detached guest-control exec routing.
//!
//! Attached `vm exec` sessions keep an authenticated guest-control client in the
//! owner FSM. Detached create/list/logs/status/kill are deliberately one-shot:
//! connect, authenticate, issue exactly one management RPC (two for logs/kill
//! where the guest protocol requires it), return a redacted public DTO, then
//! drop the client.

use std::path::PathBuf;
use std::time::Duration;

use nixling_core::base64_codec;
use nixling_ipc::guest_proto as pb;
use nixling_ipc::guest_wire::ExecState as PublicExecState;
use nixling_ipc::public_wire::{
    self, ExecDetachedCreateResult, ExecDetachedKillOutcome, ExecDetachedKillResult,
    ExecDetachedListEntry, ExecDetachedListResult, ExecDetachedLogsResult,
    ExecDetachedStatusResult,
};
use protobuf::{EnumOrUnknown, MessageField};

use crate::exec_session::{ExecOpDeadlines, ExecOpError, ExecStartSpec, GuestOpError};
use crate::guest_control_bridge::{
    connect_and_build_client, host_nonce, BrokerSigner, ProbeParams, GUEST_CONTROL_ATTEMPT_CAP,
    VMADDR_CID_HOST,
};
use crate::guest_control_health::{
    probe_guest_control_health, AttemptBudget, TtrpcGuestControlClient,
};
use crate::typed_error::TypedError;
use crate::{
    broker_socket_path, exec_session_real, load_bundle_resolver,
    resolve_guest_control_probe_params, ServerState,
};

const DETACHED_CREATE_DEADLINE: Duration = Duration::from_secs(12);
const DETACHED_CANCEL_DEADLINE: Duration = Duration::from_secs(30);
#[cfg(test)]
const DETACHED_CREATE_GUEST_WINDOW: Duration = Duration::from_millis(10_000);
#[cfg(test)]
const DETACHED_CANCEL_GUEST_WINDOW: Duration = Duration::from_millis(15_000);

#[cfg(test)]
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug)]
enum DetachedRealRequest {
    Create(ExecStartSpec),
    List,
    Status { exec_id: String },
    Logs { exec_id: String },
    Kill { exec_id: String },
}

#[derive(Debug)]
enum DetachedRealResponse {
    Create(ExecDetachedCreateResult),
    List(ExecDetachedListResult),
    Status(ExecDetachedStatusResult),
    Logs(ExecDetachedLogsResult),
    Kill(ExecDetachedKillResult),
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DetachedTestRequest {
    Create {
        vm: String,
        argv_len: usize,
        env_len: usize,
        has_cwd: bool,
    },
    List {
        vm: String,
    },
    Status {
        vm: String,
        exec_id: String,
    },
    Logs {
        vm: String,
        exec_id: String,
    },
    Kill {
        vm: String,
        exec_id: String,
    },
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) enum DetachedTestResponse {
    Create(ExecDetachedCreateResult),
    List(ExecDetachedListResult),
    Status(ExecDetachedStatusResult),
    Logs(ExecDetachedLogsResult),
    Kill(ExecDetachedKillResult),
}

#[cfg(test)]
pub(crate) type DetachedTestHook =
    Arc<dyn Fn(DetachedTestRequest) -> Result<DetachedTestResponse, TypedError> + Send + Sync>;

#[cfg(test)]
fn hook_slot() -> &'static Mutex<Option<DetachedTestHook>> {
    static HOOK: OnceLock<Mutex<Option<DetachedTestHook>>> = OnceLock::new();
    HOOK.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
pub(crate) struct DetachedTestHookGuard;

#[cfg(test)]
impl Drop for DetachedTestHookGuard {
    fn drop(&mut self) {
        *hook_slot().lock().expect("detached exec test hook lock") = None;
    }
}

#[cfg(test)]
pub(crate) fn set_test_hook(hook: DetachedTestHook) -> DetachedTestHookGuard {
    *hook_slot().lock().expect("detached exec test hook lock") = Some(hook);
    DetachedTestHookGuard
}

#[cfg(test)]
fn test_hook(request: DetachedTestRequest) -> Option<Result<DetachedTestResponse, TypedError>> {
    hook_slot()
        .lock()
        .expect("detached exec test hook lock")
        .clone()
        .map(|hook| hook(request))
}

pub(crate) fn create(
    state: &ServerState,
    start: &public_wire::ExecStartArgs,
) -> Result<ExecDetachedCreateResult, TypedError> {
    #[cfg(test)]
    if let Some(result) = test_hook(DetachedTestRequest::Create {
        vm: start.vm.clone(),
        argv_len: start.argv.len(),
        env_len: start.env.as_ref().map_or(0, Vec::len),
        has_cwd: start.cwd.is_some(),
    }) {
        return match result? {
            DetachedTestResponse::Create(response) => Ok(response),
            _ => Err(internal_error(
                "detached create test hook returned wrong variant",
            )),
        };
    }

    let spec = ExecStartSpec {
        vm: start.vm.clone(),
        argv: start.argv.clone(),
        tty: start.tty,
        detached: true,
        env: start
            .env
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|var| (var.key, var.value))
            .collect(),
        cwd: start.cwd.clone(),
        term_size: start.term_size.map(|size| (size.rows, size.cols)),
    };
    match run_real(state, &start.vm, DetachedRealRequest::Create(spec))? {
        DetachedRealResponse::Create(response) => Ok(response),
        _ => Err(internal_error("detached create returned wrong variant")),
    }
}

pub(crate) fn list(
    state: &ServerState,
    args: &public_wire::ExecDetachedListArgs,
) -> Result<ExecDetachedListResult, TypedError> {
    #[cfg(test)]
    if let Some(result) = test_hook(DetachedTestRequest::List {
        vm: args.vm.clone(),
    }) {
        return match result? {
            DetachedTestResponse::List(response) => Ok(response),
            _ => Err(internal_error(
                "detached list test hook returned wrong variant",
            )),
        };
    }

    match run_real(state, &args.vm, DetachedRealRequest::List)? {
        DetachedRealResponse::List(response) => Ok(response),
        _ => Err(internal_error("detached list returned wrong variant")),
    }
}

pub(crate) fn status(
    state: &ServerState,
    args: &public_wire::ExecDetachedStatusArgs,
) -> Result<ExecDetachedStatusResult, TypedError> {
    #[cfg(test)]
    if let Some(result) = test_hook(DetachedTestRequest::Status {
        vm: args.vm.clone(),
        exec_id: args.exec_id.clone(),
    }) {
        return match result? {
            DetachedTestResponse::Status(response) => Ok(response),
            _ => Err(internal_error(
                "detached status test hook returned wrong variant",
            )),
        };
    }

    match run_real(
        state,
        &args.vm,
        DetachedRealRequest::Status {
            exec_id: args.exec_id.clone(),
        },
    )? {
        DetachedRealResponse::Status(response) => Ok(response),
        _ => Err(internal_error("detached status returned wrong variant")),
    }
}

pub(crate) fn logs(
    state: &ServerState,
    args: &public_wire::ExecDetachedLogsArgs,
) -> Result<ExecDetachedLogsResult, TypedError> {
    #[cfg(test)]
    if let Some(result) = test_hook(DetachedTestRequest::Logs {
        vm: args.vm.clone(),
        exec_id: args.exec_id.clone(),
    }) {
        return match result? {
            DetachedTestResponse::Logs(response) => Ok(response),
            _ => Err(internal_error(
                "detached logs test hook returned wrong variant",
            )),
        };
    }

    match run_real(
        state,
        &args.vm,
        DetachedRealRequest::Logs {
            exec_id: args.exec_id.clone(),
        },
    )? {
        DetachedRealResponse::Logs(response) => Ok(response),
        _ => Err(internal_error("detached logs returned wrong variant")),
    }
}

pub(crate) fn kill(
    state: &ServerState,
    args: &public_wire::ExecDetachedKillArgs,
) -> Result<ExecDetachedKillResult, TypedError> {
    #[cfg(test)]
    if let Some(result) = test_hook(DetachedTestRequest::Kill {
        vm: args.vm.clone(),
        exec_id: args.exec_id.clone(),
    }) {
        return match result? {
            DetachedTestResponse::Kill(response) => Ok(response),
            _ => Err(internal_error(
                "detached kill test hook returned wrong variant",
            )),
        };
    }

    match run_real(
        state,
        &args.vm,
        DetachedRealRequest::Kill {
            exec_id: args.exec_id.clone(),
        },
    )? {
        DetachedRealResponse::Kill(response) => Ok(response),
        _ => Err(internal_error("detached kill returned wrong variant")),
    }
}

fn run_real(
    state: &ServerState,
    vm: &str,
    request: DetachedRealRequest,
) -> Result<DetachedRealResponse, TypedError> {
    let resolver =
        load_bundle_resolver(state).map_err(|_| exec_typed_error(ExecOpError::Transport))?;
    let params = resolve_guest_control_probe_params(state, &resolver, vm)
        .map_err(|_| exec_typed_error(ExecOpError::OldGeneration))?;
    let broker_socket = broker_socket_path(state);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| TypedError::InternalIo {
            context: "build detached exec runtime".to_owned(),
            detail: err.to_string(),
        })?;

    runtime
        .block_on(async move {
            let client = DetachedClient::connect(params, broker_socket).await?;
            match request {
                DetachedRealRequest::Create(spec) => client
                    .exec_create(&spec)
                    .await
                    .map(DetachedRealResponse::Create),
                DetachedRealRequest::List => {
                    client.exec_list().await.map(DetachedRealResponse::List)
                }
                DetachedRealRequest::Status { exec_id } => client
                    .exec_status(&exec_id)
                    .await
                    .map(DetachedRealResponse::Status),
                DetachedRealRequest::Logs { exec_id } => client
                    .exec_logs(&exec_id)
                    .await
                    .map(DetachedRealResponse::Logs),
                DetachedRealRequest::Kill { exec_id } => client
                    .exec_kill(&exec_id)
                    .await
                    .map(DetachedRealResponse::Kill),
            }
        })
        .map_err(exec_typed_error)
}

fn exec_typed_error(error: ExecOpError) -> TypedError {
    crate::map_exec_op_error(error)
}

fn internal_error(detail: impl Into<String>) -> TypedError {
    TypedError::InternalConfig {
        detail: detail.into(),
    }
}

struct DetachedClient {
    client: TtrpcGuestControlClient,
    vm_id: String,
    guest_boot_id: String,
    deadlines: ExecOpDeadlines,
}

impl DetachedClient {
    async fn connect(params: ProbeParams, broker_socket: PathBuf) -> Result<Self, ExecOpError> {
        let budget = AttemptBudget::from_now(
            exec_session_real::ESTABLISH_TIMEOUT,
            GUEST_CONTROL_ATTEMPT_CAP,
        );
        let signer = BrokerSigner::new(broker_socket, budget);
        let nonce = host_nonce().map_err(|_| ExecOpError::Transport)?;
        let vm_id = params.vm_id.clone();
        let client = connect_and_build_client(&params, budget)
            .map_err(exec_session_real::map_op_health_error)?;
        let evidence =
            probe_guest_control_health(&vm_id, Some(VMADDR_CID_HOST), nonce, &client, &signer)
                .await
                .map_err(exec_session_real::map_op_health_error)?;

        gate_detached_capabilities(&evidence.health.capabilities)?;

        Ok(Self {
            client,
            vm_id,
            guest_boot_id: evidence.guest_boot_id,
            deadlines: ExecOpDeadlines::default(),
        })
    }

    async fn exec_create(
        &self,
        spec: &ExecStartSpec,
    ) -> Result<ExecDetachedCreateResult, ExecOpError> {
        let request = exec_session_real::build_exec_create_request(&self.vm_id, spec);
        let response: pb::ExecCreateResponse = self
            .client
            .unary_with_timeout("ExecCreate", request, DETACHED_CREATE_DEADLINE)
            .await
            .map_err(exec_session_real::map_op_health_error)?;
        check_response_error(response.error.as_ref())?;
        let exec_id = response.exec_id.clone().ok_or(ExecOpError::Protocol)?;
        let state = map_exec_state(response.state)?;
        Ok(ExecDetachedCreateResult { exec_id, state })
    }

    async fn exec_list(&self) -> Result<ExecDetachedListResult, ExecOpError> {
        let mut request = pb::ExecListRequest::new();
        request.metadata =
            MessageField::some(common_metadata(&self.vm_id, "guest-control-exec-list"));
        request.guest_boot_id = self.guest_boot_id.clone();
        let response: pb::ExecListResponse = self
            .client
            .unary_with_timeout("ExecList", request, self.deadlines.control)
            .await
            .map_err(exec_session_real::map_op_health_error)?;
        check_response_error(response.error.as_ref())?;
        let mut execs = Vec::with_capacity(response.entries.len());
        for entry in &response.entries {
            let inspect = self.inspect(&entry.exec_id).await.ok();
            execs.push(map_list_entry(entry, inspect.as_ref())?);
        }
        Ok(ExecDetachedListResult { execs })
    }

    async fn exec_status(&self, exec_id: &str) -> Result<ExecDetachedStatusResult, ExecOpError> {
        let response = self.inspect(exec_id).await?;
        map_status_response(exec_id, &response)
    }

    async fn exec_logs(&self, exec_id: &str) -> Result<ExecDetachedLogsResult, ExecOpError> {
        let inspect = self.inspect(exec_id).await?;
        let stdout_window =
            stream_window_from_inspect(&inspect, pb::OutputStream::OUTPUT_STREAM_STDOUT);
        let stderr_window =
            stream_window_from_inspect(&inspect, pb::OutputStream::OUTPUT_STREAM_STDERR);
        let stdout = self
            .read_retained_log_stream(
                exec_id,
                pb::OutputStream::OUTPUT_STREAM_STDOUT,
                stdout_window,
            )
            .await?;
        let stderr = self
            .read_retained_log_stream(
                exec_id,
                pb::OutputStream::OUTPUT_STREAM_STDERR,
                stderr_window,
            )
            .await?;
        Ok(map_logs_result(exec_id, &stdout, &stderr))
    }

    async fn exec_kill(&self, exec_id: &str) -> Result<ExecDetachedKillResult, ExecOpError> {
        let before = self.inspect(exec_id).await?;
        let before_state = map_exec_state(before.state)?;
        if is_terminal_state(before_state) {
            return Ok(ExecDetachedKillResult {
                exec_id: exec_id.to_owned(),
                result: ExecDetachedKillOutcome::AlreadyTerminal,
                state: before_state,
            });
        }

        let mut request = pb::ExecCancelRequest::new();
        request.metadata =
            MessageField::some(self.exec_metadata(exec_id, "guest-control-exec-kill"));
        request.control_seq = before.last_control_seq.saturating_add(1);
        request.reason =
            EnumOrUnknown::new(pb::ExecCancelReason::EXEC_CANCEL_REASON_USER_REQUESTED);
        let response: pb::ControlAck = self
            .client
            .unary_with_timeout("ExecCancel", request, DETACHED_CANCEL_DEADLINE)
            .await
            .map_err(exec_session_real::map_op_health_error)?;
        if let Some(error) = response.error.as_ref() {
            if !exec_session_real::is_unspecified(error.kind) {
                let mapped = exec_session_real::map_guest_control_error(error);
                if matches!(mapped, ExecOpError::Guest(GuestOpError::ExecAlreadyExited)) {
                    let terminal = self.inspect(exec_id).await?;
                    let terminal_state = map_exec_state(terminal.state)?;
                    if !is_terminal_state(terminal_state) {
                        return Err(ExecOpError::Protocol);
                    }
                    return Ok(ExecDetachedKillResult {
                        exec_id: exec_id.to_owned(),
                        result: ExecDetachedKillOutcome::AlreadyTerminal,
                        state: terminal_state,
                    });
                }
                return Err(mapped);
            }
        }
        let after = self.inspect(exec_id).await?;
        let after_state = map_exec_state(after.state)?;
        if response.duplicate {
            if !is_terminal_state(after_state) {
                return Err(ExecOpError::Protocol);
            }
            return Ok(ExecDetachedKillResult {
                exec_id: exec_id.to_owned(),
                result: ExecDetachedKillOutcome::AlreadyTerminal,
                state: after_state,
            });
        }
        Ok(ExecDetachedKillResult {
            exec_id: exec_id.to_owned(),
            result: ExecDetachedKillOutcome::Cancelling,
            state: after_state,
        })
    }

    async fn inspect(&self, exec_id: &str) -> Result<pb::ExecInspectResponse, ExecOpError> {
        let mut request = pb::ExecInspectRequest::new();
        request.metadata =
            MessageField::some(self.exec_metadata(exec_id, "guest-control-exec-status"));
        let response: pb::ExecInspectResponse = self
            .client
            .unary_with_timeout("ExecInspect", request, self.deadlines.control)
            .await
            .map_err(exec_session_real::map_op_health_error)?;
        check_response_error(response.error.as_ref())?;
        Ok(response)
    }

    async fn read_log_stream(
        &self,
        exec_id: &str,
        stream: pb::OutputStream,
        offset: u64,
    ) -> Result<pb::ExecLogsResponse, LogReadError> {
        let mut request = pb::ExecLogsRequest::new();
        request.metadata =
            MessageField::some(self.exec_metadata(exec_id, "guest-control-exec-logs"));
        request.stream = EnumOrUnknown::new(stream);
        request.offset = offset;
        request.max_len = public_wire::EXEC_MAX_CHUNK_BYTES;
        let response: pb::ExecLogsResponse = self
            .client
            .unary_with_timeout("ExecLogs", request, self.deadlines.control)
            .await
            .map_err(exec_session_real::map_op_health_error)
            .map_err(LogReadError::Op)?;
        if let Some(error) = response.error.as_ref() {
            if is_guest_error_kind(
                error,
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_OFFSET_EXPIRED,
            ) {
                return Err(LogReadError::OffsetExpired);
            }
            check_response_error(Some(error)).map_err(LogReadError::Op)?;
        }
        Ok(response)
    }

    async fn read_retained_log_stream(
        &self,
        exec_id: &str,
        stream: pb::OutputStream,
        mut window: RetainedStreamWindow,
    ) -> Result<pb::ExecLogsResponse, ExecOpError> {
        for _ in 0..2 {
            match self
                .read_log_stream(exec_id, stream, window.start_offset)
                .await
            {
                Ok(response) => return Ok(response),
                Err(LogReadError::OffsetExpired) => {
                    let inspect = self.inspect(exec_id).await?;
                    window = stream_window_from_inspect(&inspect, stream);
                }
                Err(LogReadError::Op(error)) => return Err(error),
            }
        }
        Ok(empty_retained_log_stream(stream, window))
    }

    fn exec_metadata(&self, exec_id: &str, request_id: &str) -> pb::ExecRequestMetadata {
        let mut metadata = pb::ExecRequestMetadata::new();
        metadata.common = MessageField::some(common_metadata(&self.vm_id, request_id));
        metadata.exec_id = exec_id.to_owned();
        metadata.guest_boot_id = self.guest_boot_id.clone();
        metadata
    }
}

fn common_metadata(vm_id: &str, request_id: &str) -> pb::RequestMetadata {
    let mut metadata = pb::RequestMetadata::new();
    metadata.vm_id = vm_id.to_owned();
    metadata.request_id = request_id.to_owned();
    metadata.protocol_version = nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
    metadata
}

pub(crate) fn gate_detached_capabilities(
    capabilities: &[EnumOrUnknown<pb::GuestCapability>],
) -> Result<(), ExecOpError> {
    let advertises = |cap: pb::GuestCapability| {
        capabilities
            .iter()
            .filter_map(|value| value.enum_value().ok())
            .any(|value| value == cap)
    };
    if !advertises(pb::GuestCapability::GUEST_CAPABILITY_EXEC_DETACHED) {
        return Err(ExecOpError::DetachedUnavailable);
    }
    if !advertises(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS) {
        return Err(ExecOpError::Capability);
    }
    Ok(())
}

fn check_response_error(error: Option<&pb::GuestControlError>) -> Result<(), ExecOpError> {
    if let Some(error) = error {
        if !exec_session_real::is_unspecified(error.kind) {
            return Err(exec_session_real::map_guest_control_error(error));
        }
    }
    Ok(())
}

fn is_guest_error_kind(error: &pb::GuestControlError, expected: pb::GuestControlErrorKind) -> bool {
    matches!(error.kind.enum_value(), Ok(kind) if kind == expected)
}

#[derive(Debug)]
enum LogReadError {
    OffsetExpired,
    Op(ExecOpError),
}

#[derive(Debug, Clone, Copy)]
struct RetainedStreamWindow {
    start_offset: u64,
    end_offset: u64,
    dropped_bytes: u64,
    truncated: bool,
}

fn stream_window_from_inspect(
    response: &pb::ExecInspectResponse,
    stream: pb::OutputStream,
) -> RetainedStreamWindow {
    match stream {
        pb::OutputStream::OUTPUT_STREAM_STDOUT => RetainedStreamWindow {
            start_offset: response.stdout_start_offset,
            end_offset: response.stdout_end_offset,
            dropped_bytes: response.stdout_dropped_bytes,
            truncated: response.stdout_truncated_for_retention,
        },
        pb::OutputStream::OUTPUT_STREAM_STDERR => RetainedStreamWindow {
            start_offset: response.stderr_start_offset,
            end_offset: response.stderr_end_offset,
            dropped_bytes: response.stderr_dropped_bytes,
            truncated: response.stderr_truncated_for_retention,
        },
        _ => RetainedStreamWindow {
            start_offset: 0,
            end_offset: 0,
            dropped_bytes: 0,
            truncated: false,
        },
    }
}

fn empty_retained_log_stream(
    stream: pb::OutputStream,
    window: RetainedStreamWindow,
) -> pb::ExecLogsResponse {
    let mut response = pb::ExecLogsResponse::new();
    response.stream = EnumOrUnknown::new(stream);
    response.offset = window.start_offset;
    response.start_offset = window.start_offset;
    response.end_offset = window.end_offset;
    response.next_offset = window.start_offset;
    response.eof = window.start_offset >= window.end_offset;
    response.dropped_bytes = window.dropped_bytes;
    response.truncated = true;
    response
}

fn map_logs_result(
    exec_id: &str,
    stdout: &pb::ExecLogsResponse,
    stderr: &pb::ExecLogsResponse,
) -> ExecDetachedLogsResult {
    let start_offset = stdout.start_offset.min(stderr.start_offset);
    let end_offset = stdout.end_offset.max(stderr.end_offset);
    let dropped_bytes = stdout.dropped_bytes.saturating_add(stderr.dropped_bytes);
    let truncated = stdout.truncated || stderr.truncated || !stdout.eof || !stderr.eof;
    ExecDetachedLogsResult {
        exec_id: exec_id.to_owned(),
        stdout_base64: base64_codec::encode(&stdout.data),
        stderr_base64: base64_codec::encode(&stderr.data),
        start_offset,
        end_offset,
        dropped_bytes,
        truncated,
        stdout_start_offset: stdout.start_offset,
        stdout_end_offset: stdout.end_offset,
        stdout_next_offset: stdout.next_offset,
        stdout_eof: stdout.eof,
        stdout_dropped_bytes: stdout.dropped_bytes,
        stdout_truncated: stdout.truncated,
        stderr_start_offset: stderr.start_offset,
        stderr_end_offset: stderr.end_offset,
        stderr_next_offset: stderr.next_offset,
        stderr_eof: stderr.eof,
        stderr_dropped_bytes: stderr.dropped_bytes,
        stderr_truncated: stderr.truncated,
    }
}

fn map_exec_state(state: EnumOrUnknown<pb::ExecState>) -> Result<PublicExecState, ExecOpError> {
    match state.enum_value() {
        Ok(pb::ExecState::EXEC_STATE_CREATED) => Ok(PublicExecState::Created),
        Ok(pb::ExecState::EXEC_STATE_RUNNING) => Ok(PublicExecState::Running),
        Ok(pb::ExecState::EXEC_STATE_EXITED) => Ok(PublicExecState::Exited),
        Ok(pb::ExecState::EXEC_STATE_SIGNALED) => Ok(PublicExecState::Signaled),
        Ok(pb::ExecState::EXEC_STATE_CANCELLED) => Ok(PublicExecState::Cancelled),
        Ok(pb::ExecState::EXEC_STATE_SLOW_CONSUMER_CANCELLED) => {
            Ok(PublicExecState::SlowConsumerCancelled)
        }
        Ok(pb::ExecState::EXEC_STATE_PROTOCOL_ERROR) => Ok(PublicExecState::ProtocolError),
        Ok(pb::ExecState::EXEC_STATE_LOST_GUESTD) => Ok(PublicExecState::LostGuestd),
        Ok(pb::ExecState::EXEC_STATE_REAPED) => Ok(PublicExecState::Reaped),
        _ => Err(ExecOpError::Protocol),
    }
}

fn is_terminal_state(state: PublicExecState) -> bool {
    matches!(
        state,
        PublicExecState::Exited
            | PublicExecState::Signaled
            | PublicExecState::Cancelled
            | PublicExecState::SlowConsumerCancelled
            | PublicExecState::ProtocolError
            | PublicExecState::LostGuestd
            | PublicExecState::Reaped
    )
}

fn terminal_fields(
    status: Option<&pb::TerminalStatus>,
) -> (Option<i32>, Option<u32>, Option<String>) {
    match status.and_then(|status| status.outcome.as_ref()) {
        Some(pb::terminal_status::Outcome::ExitCode(code))
        | Some(pb::terminal_status::Outcome::StatusCode(code)) => (Some(*code), None, None),
        Some(pb::terminal_status::Outcome::Signal(signal)) => (None, Some(*signal), None),
        Some(pb::terminal_status::Outcome::Error(error)) => (
            None,
            None,
            Some(
                error
                    .enum_value()
                    .map(|kind| format!("{kind:?}"))
                    .unwrap_or_else(|_| "unknown-error".to_owned()),
            ),
        ),
        None | Some(_) => (None, None, None),
    }
}

fn state_reason(state: PublicExecState, terminal_reason: Option<String>) -> Option<String> {
    terminal_reason.or_else(|| match state {
        PublicExecState::Cancelled => Some("cancelled".to_owned()),
        PublicExecState::SlowConsumerCancelled => Some("slow-consumer-cancelled".to_owned()),
        PublicExecState::ProtocolError => Some("protocol-error".to_owned()),
        PublicExecState::LostGuestd => Some("lost-guestd".to_owned()),
        PublicExecState::Reaped => Some("reaped".to_owned()),
        _ => None,
    })
}

fn map_status_response(
    exec_id: &str,
    response: &pb::ExecInspectResponse,
) -> Result<ExecDetachedStatusResult, ExecOpError> {
    let state = map_exec_state(response.state)?;
    let (exit_code, signal, terminal_reason) =
        terminal_fields(response.visible_terminal_status.as_ref());
    Ok(ExecDetachedStatusResult {
        exec_id: exec_id.to_owned(),
        state,
        reason: state_reason(state, terminal_reason),
        exit_code,
        signal,
        start_offset: response
            .stdout_start_offset
            .min(response.stderr_start_offset),
        end_offset: response.stdout_end_offset.max(response.stderr_end_offset),
        dropped_bytes: response
            .stdout_dropped_bytes
            .saturating_add(response.stderr_dropped_bytes),
        truncated: response.stdout_truncated_for_retention
            || response.stderr_truncated_for_retention,
    })
}

fn map_list_entry(
    entry: &pb::ExecListEntry,
    inspect: Option<&pb::ExecInspectResponse>,
) -> Result<ExecDetachedListEntry, ExecOpError> {
    let state = inspect
        .map(|response| map_exec_state(response.state))
        .unwrap_or_else(|| map_exec_state(entry.state))?;
    let (exit_code, signal, stdout, stderr) = if let Some(response) = inspect {
        let (exit_code, signal, _terminal_reason) =
            terminal_fields(response.visible_terminal_status.as_ref());
        (
            exit_code,
            signal,
            stream_window_from_inspect(response, pb::OutputStream::OUTPUT_STREAM_STDOUT),
            stream_window_from_inspect(response, pb::OutputStream::OUTPUT_STREAM_STDERR),
        )
    } else {
        (
            None,
            None,
            RetainedStreamWindow {
                start_offset: 0,
                end_offset: 0,
                dropped_bytes: 0,
                truncated: entry.stdout_truncated,
            },
            RetainedStreamWindow {
                start_offset: 0,
                end_offset: 0,
                dropped_bytes: 0,
                truncated: entry.stderr_truncated,
            },
        )
    };
    let dropped_bytes = if inspect.is_some() {
        stdout.dropped_bytes.saturating_add(stderr.dropped_bytes)
    } else {
        entry.dropped_bytes
    };
    Ok(ExecDetachedListEntry {
        exec_id: entry.exec_id.clone(),
        state,
        exit_code,
        signal,
        started_at: entry.create_time_unix.to_string(),
        start_offset: stdout.start_offset.min(stderr.start_offset),
        end_offset: stdout.end_offset.max(stderr.end_offset),
        stdout_start_offset: stdout.start_offset,
        stdout_end_offset: stdout.end_offset,
        stderr_start_offset: stderr.start_offset,
        stderr_end_offset: stderr.end_offset,
        dropped_bytes,
        stdout_dropped_bytes: stdout.dropped_bytes,
        stderr_dropped_bytes: stderr.dropped_bytes,
        truncated: stdout.truncated || stderr.truncated,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(value: pb::GuestCapability) -> EnumOrUnknown<pb::GuestCapability> {
        EnumOrUnknown::new(value)
    }

    #[test]
    fn missing_exec_detached_capability_is_clear_error() {
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
        ];
        assert_eq!(
            gate_detached_capabilities(&caps),
            Err(ExecOpError::DetachedUnavailable)
        );
        let typed = crate::map_exec_op_error(ExecOpError::DetachedUnavailable);
        assert_eq!(typed.kind(), "guest-control-exec-detached-unavailable");
        assert!(typed.message().contains("detached exec"));
    }

    #[test]
    fn detached_capability_gate_accepts_detached_with_logs() {
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_DETACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
        ];
        assert_eq!(gate_detached_capabilities(&caps), Ok(()));
    }

    #[test]
    fn detached_deadlines_cover_guest_bounded_windows() {
        assert!(
            DETACHED_CREATE_DEADLINE > DETACHED_CREATE_GUEST_WINDOW,
            "create deadline must cover guestd's 10s create window plus margin"
        );
        assert!(
            DETACHED_CANCEL_DEADLINE > DETACHED_CANCEL_GUEST_WINDOW,
            "cancel deadline must cover guestd's 15s cancel window plus margin"
        );
    }

    #[test]
    fn status_translation_combines_retained_log_accounting() {
        let mut terminal = pb::TerminalStatus::new();
        terminal.set_exit_code(7);
        let mut response = pb::ExecInspectResponse::new();
        response.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_EXITED);
        response.visible_terminal_status = MessageField::some(terminal);
        response.stdout_start_offset = 2;
        response.stderr_start_offset = 4;
        response.stdout_end_offset = 10;
        response.stderr_end_offset = 9;
        response.stdout_dropped_bytes = 3;
        response.stderr_dropped_bytes = 5;
        response.stderr_truncated_for_retention = true;

        let mapped = map_status_response("exec-1", &response).expect("status maps");
        assert_eq!(mapped.exec_id, "exec-1");
        assert_eq!(mapped.state, PublicExecState::Exited);
        assert_eq!(mapped.exit_code, Some(7));
        assert_eq!(mapped.signal, None);
        assert_eq!(mapped.start_offset, 2);
        assert_eq!(mapped.end_offset, 10);
        assert_eq!(mapped.dropped_bytes, 8);
        assert!(mapped.truncated);
    }

    #[test]
    fn list_translation_maps_terminal_status_offsets_and_drops_argv_hash() {
        let mut entry = pb::ExecListEntry::new();
        entry.exec_id = "exec-1".to_owned();
        entry.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_RUNNING);
        entry.create_time_unix = 1_700_000_001;
        entry.argv_sha256 = "SENTINEL-ARGV-HASH".to_owned();
        entry.stderr_truncated = true;
        entry.dropped_bytes = 9;

        let mut terminal = pb::TerminalStatus::new();
        terminal.set_signal(15);
        let mut inspect = pb::ExecInspectResponse::new();
        inspect.state = EnumOrUnknown::new(pb::ExecState::EXEC_STATE_SIGNALED);
        inspect.visible_terminal_status = MessageField::some(terminal);
        inspect.stdout_start_offset = 4;
        inspect.stdout_end_offset = 20;
        inspect.stderr_start_offset = 7;
        inspect.stderr_end_offset = 11;
        inspect.stdout_dropped_bytes = 2;
        inspect.stderr_dropped_bytes = 3;
        inspect.stderr_truncated_for_retention = true;

        let mapped = map_list_entry(&entry, Some(&inspect)).expect("list entry maps");
        let encoded = serde_json::to_string(&mapped).expect("serialize public entry");
        assert!(!encoded.contains("argv"));
        assert!(!encoded.contains("SENTINEL-ARGV-HASH"));
        assert_eq!(mapped.state, PublicExecState::Signaled);
        assert_eq!(mapped.exit_code, None);
        assert_eq!(mapped.signal, Some(15));
        assert_eq!(mapped.started_at, "1700000001");
        assert_eq!(mapped.start_offset, 4);
        assert_eq!(mapped.end_offset, 20);
        assert_eq!(mapped.stdout_start_offset, 4);
        assert_eq!(mapped.stdout_end_offset, 20);
        assert_eq!(mapped.stderr_start_offset, 7);
        assert_eq!(mapped.stderr_end_offset, 11);
        assert!(mapped.truncated);
        assert!(!mapped.stdout_truncated);
        assert!(mapped.stderr_truncated);
        assert_eq!(mapped.dropped_bytes, 5);
        assert_eq!(mapped.stdout_dropped_bytes, 2);
        assert_eq!(mapped.stderr_dropped_bytes, 3);
    }

    #[test]
    fn logs_translation_preserves_per_stream_cursors() {
        let mut stdout = pb::ExecLogsResponse::new();
        stdout.data = b"out".to_vec();
        stdout.start_offset = 5;
        stdout.end_offset = 8;
        stdout.next_offset = 8;
        stdout.eof = true;
        stdout.dropped_bytes = 5;
        stdout.truncated = true;
        let mut stderr = pb::ExecLogsResponse::new();
        stderr.data = b"err".to_vec();
        stderr.start_offset = 2;
        stderr.end_offset = 12;
        stderr.next_offset = 6;
        stderr.eof = false;
        stderr.dropped_bytes = 2;
        stderr.truncated = false;

        let mapped = map_logs_result("exec-1", &stdout, &stderr);
        assert_eq!(mapped.stdout_base64, "b3V0");
        assert_eq!(mapped.stderr_base64, "ZXJy");
        assert_eq!(mapped.start_offset, 2);
        assert_eq!(mapped.end_offset, 12);
        assert_eq!(mapped.dropped_bytes, 7);
        assert!(mapped.truncated);
        assert_eq!(mapped.stdout_start_offset, 5);
        assert_eq!(mapped.stdout_end_offset, 8);
        assert_eq!(mapped.stdout_next_offset, 8);
        assert!(mapped.stdout_eof);
        assert_eq!(mapped.stdout_dropped_bytes, 5);
        assert!(mapped.stdout_truncated);
        assert_eq!(mapped.stderr_start_offset, 2);
        assert_eq!(mapped.stderr_end_offset, 12);
        assert_eq!(mapped.stderr_next_offset, 6);
        assert!(!mapped.stderr_eof);
        assert_eq!(mapped.stderr_dropped_bytes, 2);
        assert!(!mapped.stderr_truncated);
    }
}
