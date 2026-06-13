//! Production exec connector + authenticated guest-control exec client.
//!
//! Bridges the in-process [`crate::exec_session`] machinery to the real
//! per-VM vsock transport: connect, run the authenticated handshake (reusing
//! the W15 [`crate::guest_control_bridge`] connect/probe path), gate on the
//! guest's advertised exec capabilities, then issue `ExecCreate`. The
//! returned [`RealExecClient`] proxies each subsequent exec op with a FRESH
//! per-op deadline (never the exhausted one-shot establishment budget).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nixling_ipc::guest_proto as pb;
use protobuf::{EnumOrUnknown, MessageField};

use crate::exec_session::{
    Established, ExecEstablishError, ExecGuestClient, ExecGuestConnector, ExecOpDeadlines,
    ExecOpError, ExecSessionInfo, ExecStartSpec, GuestOpError, NegotiatedCaps, OutputStreamSel,
    ReadOutputOutcome, TerminalKind, WaitOutcome, WriteStdinOutcome,
};
use crate::guest_control_bridge::{
    connect_and_build_client, host_nonce, ProbeParams, BrokerSigner, GUEST_CONTROL_ATTEMPT_CAP,
    VMADDR_CID_HOST,
};
use crate::guest_control_health::{
    probe_guest_control_health, AttemptBudget, GuestControlHealthError, TtrpcGuestControlClient,
};
use nixling_ipc::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;

/// Generous absolute deadline for the whole establish (connect + auth +
/// `ExecCreate`). Per-op deadlines are separate and fresh.
const ESTABLISH_TIMEOUT: Duration = Duration::from_secs(12);

/// Production exec connector. Owns the resolved probe params + broker socket
/// path so it is `Send + Sync` and can move into the worker thread.
pub struct RealExecConnector {
    params: ProbeParams,
    broker_socket_path: PathBuf,
    deadlines: ExecOpDeadlines,
}

impl RealExecConnector {
    pub fn new(
        params: ProbeParams,
        broker_socket_path: PathBuf,
        deadlines: ExecOpDeadlines,
    ) -> Self {
        Self {
            params,
            broker_socket_path,
            deadlines,
        }
    }
}

#[async_trait]
impl ExecGuestConnector for RealExecConnector {
    async fn establish(&self, spec: &ExecStartSpec) -> Result<Established, ExecEstablishError> {
        let budget = AttemptBudget::from_now(ESTABLISH_TIMEOUT, GUEST_CONTROL_ATTEMPT_CAP);
        let signer = BrokerSigner::new(self.broker_socket_path.clone(), budget);
        let nonce = host_nonce().map_err(|_| ExecEstablishError::Transport)?;
        let client =
            connect_and_build_client(&self.params, budget).map_err(map_establish_health_error)?;
        let evidence = probe_guest_control_health(
            &self.params.vm_id,
            Some(VMADDR_CID_HOST),
            nonce,
            &client,
            &signer,
        )
        .await
        .map_err(map_establish_health_error)?;

        let caps = gate_capabilities(&evidence.health.capabilities, spec.tty)?;

        let op_timeout = self.deadlines.control;
        let request = build_exec_create_request(&self.params.vm_id, spec);
        let response: pb::ExecCreateResponse = client
            .unary_with_timeout("ExecCreate", request, op_timeout)
            .await
            .map_err(map_op_health_error_for_establish)?;

        if let Some(error) = response.error.as_ref() {
            if !is_unspecified(error.kind) {
                return Err(op_to_establish(map_guest_control_error(error)));
            }
        }
        let exec_id = response
            .exec_id
            .clone()
            .ok_or(ExecEstablishError::Protocol)?;

        let real_client = RealExecClient {
            client: Arc::new(client),
            vm_id: self.params.vm_id.clone(),
            guest_boot_id: evidence.guest_boot_id.clone(),
            exec_id,
        };
        Ok(Established {
            client: Arc::new(real_client),
            info: ExecSessionInfo {
                tty: spec.tty,
                stdout_offset: response.stdout_cursor,
                stderr_offset: response.stderr_cursor,
            },
            control_seq: response.control_seq,
            caps,
        })
    }
}

/// Fail closed unless the guest advertises every exec capability the session
/// needs, returning the negotiated cap snapshot for per-op gating (WR8/F6).
/// Old generations that never advertised exec map to a dedicated
/// old-generation error (exit 70, no SSH fallback).
fn gate_capabilities(
    capabilities: &[EnumOrUnknown<pb::GuestCapability>],
    tty: bool,
) -> Result<NegotiatedCaps, ExecEstablishError> {
    let advertises = |cap: pb::GuestCapability| {
        capabilities
            .iter()
            .filter_map(|value| value.enum_value().ok())
            .any(|value| value == cap)
    };
    // A guest that advertises no exec capability at all is an old generation
    // (or exec-disabled build); surface the dedicated old-generation slug.
    if !advertises(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED) {
        return Err(ExecEstablishError::OldGeneration);
    }
    if !advertises(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS) {
        return Err(ExecEstablishError::Capability);
    }
    if tty
        && (!advertises(pb::GuestCapability::GUEST_CAPABILITY_EXEC_TTY)
            || !advertises(pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE))
    {
        return Err(ExecEstablishError::Capability);
    }
    Ok(NegotiatedCaps {
        tty,
        signals: advertises(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
        tty_resize: advertises(pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE),
        output: advertises(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
    })
}

fn build_exec_create_request(vm_id: &str, spec: &ExecStartSpec) -> pb::ExecCreateRequest {
    let mut metadata = pb::RequestMetadata::new();
    metadata.vm_id = vm_id.to_owned();
    metadata.request_id = "guest-control-exec".to_owned();
    metadata.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;

    let mut request = pb::ExecCreateRequest::new();
    request.metadata = MessageField::some(metadata);
    request.argv = spec.argv.clone();
    request.cwd = spec.cwd.clone();
    request.env = spec
        .env
        .iter()
        .map(|(key, value)| {
            let mut var = pb::EnvVar::new();
            var.key = key.clone();
            var.value = value.clone();
            var
        })
        .collect();
    request.tty = spec.tty;
    request.stdin_open = true;
    request.detached = spec.detached;
    if let Some((rows, cols)) = spec.term_size {
        let mut size = pb::TerminalSize::new();
        size.rows = rows;
        size.cols = cols;
        request.initial_terminal_size = MessageField::some(size);
    }
    let mut policy = pb::OutputPolicy::new();
    policy.max_chunk_bytes = nixling_ipc::public_wire::EXEC_MAX_CHUNK_BYTES;
    request.output_policy = MessageField::some(policy);
    request
}

/// Authenticated exec client bound to one `exec_id` on one guest connection.
struct RealExecClient {
    client: Arc<TtrpcGuestControlClient>,
    vm_id: String,
    guest_boot_id: String,
    exec_id: String,
}

impl RealExecClient {
    fn exec_metadata(&self) -> pb::ExecRequestMetadata {
        let mut common = pb::RequestMetadata::new();
        common.vm_id = self.vm_id.clone();
        common.request_id = "guest-control-exec".to_owned();
        common.protocol_version = GUEST_CONTROL_PROTOCOL_VERSION;
        let mut metadata = pb::ExecRequestMetadata::new();
        metadata.common = MessageField::some(common);
        metadata.exec_id = self.exec_id.clone();
        metadata.guest_boot_id = self.guest_boot_id.clone();
        metadata
    }
}

#[async_trait]
impl ExecGuestClient for RealExecClient {
    async fn write_stdin(
        &self,
        offset: u64,
        data: Vec<u8>,
        eof: bool,
        timeout: Duration,
    ) -> Result<WriteStdinOutcome, ExecOpError> {
        let mut request = pb::WriteStdinRequest::new();
        request.metadata = MessageField::some(self.exec_metadata());
        request.offset = offset;
        request.data = data;
        request.close_after = eof;
        let response: pb::WriteStdinResponse = self
            .client
            .unary_with_timeout("WriteStdin", request, timeout)
            .await
            .map_err(map_op_health_error)?;
        if let Some(error) = response.error.as_ref() {
            if !is_unspecified(error.kind) {
                return Err(map_guest_control_error(error));
            }
        }
        let stdin_closed = matches!(
            response.stdin_state.enum_value(),
            Ok(pb::StdinState::STDIN_STATE_CLOSED
                | pb::StdinState::STDIN_STATE_CLOSED_BY_PROCESS
                | pb::StdinState::STDIN_STATE_CLOSING)
        );
        Ok(WriteStdinOutcome {
            accepted_len: response.accepted_len,
            next_offset: response.next_offset,
            backpressured: response.blocked_ms > 0,
            stdin_closed,
        })
    }

    async fn read_output(
        &self,
        stream: OutputStreamSel,
        offset: u64,
        max_len: u64,
        wait: bool,
        timeout_ms: u64,
        timeout: Duration,
    ) -> Result<ReadOutputOutcome, ExecOpError> {
        let mut request = pb::ReadOutputRequest::new();
        request.metadata = MessageField::some(self.exec_metadata());
        request.stream = EnumOrUnknown::new(match stream {
            OutputStreamSel::Stdout => pb::OutputStream::OUTPUT_STREAM_STDOUT,
            OutputStreamSel::Stderr => pb::OutputStream::OUTPUT_STREAM_STDERR,
        });
        request.offset = offset;
        request.max_len = max_len;
        request.wait = wait;
        request.timeout_ms = timeout_ms;
        let response: pb::ReadOutputResponse = self
            .client
            .unary_with_timeout("ReadOutput", request, timeout)
            .await
            .map_err(map_op_health_error)?;
        if let Some(error) = response.error.as_ref() {
            if !is_unspecified(error.kind) {
                return Err(map_guest_control_error(error));
            }
        }
        Ok(ReadOutputOutcome {
            data: response.data,
            next_offset: response.next_offset,
            eof: response.eof,
            dropped_bytes: response.dropped_bytes,
            truncated: response.truncated,
            timed_out: response.timed_out,
        })
    }

    async fn signal(
        &self,
        control_seq: u64,
        signo: u32,
        timeout: Duration,
    ) -> Result<(), ExecOpError> {
        let mut request = pb::ExecSignalRequest::new();
        request.metadata = MessageField::some(self.exec_metadata());
        request.control_seq = control_seq;
        request.signal = signo;
        request.target =
            EnumOrUnknown::new(pb::SignalTarget::SIGNAL_TARGET_FOREGROUND_PROCESS_GROUP);
        let response: pb::ControlAck = self
            .client
            .unary_with_timeout("ExecSignal", request, timeout)
            .await
            .map_err(map_op_health_error)?;
        ack_result(&response)
    }

    async fn resize(
        &self,
        control_seq: u64,
        rows: u32,
        cols: u32,
        timeout: Duration,
    ) -> Result<(), ExecOpError> {
        let mut request = pb::TtyWinResizeRequest::new();
        request.metadata = MessageField::some(self.exec_metadata());
        request.control_seq = control_seq;
        request.rows = rows;
        request.cols = cols;
        let response: pb::ControlAck = self
            .client
            .unary_with_timeout("TtyWinResize", request, timeout)
            .await
            .map_err(map_op_health_error)?;
        ack_result(&response)
    }

    async fn wait(&self, timeout_ms: u64, timeout: Duration) -> Result<WaitOutcome, ExecOpError> {
        let mut request = pb::ExecWaitRequest::new();
        request.metadata = MessageField::some(self.exec_metadata());
        request.timeout_ms = timeout_ms;
        let response: pb::ExecWaitResponse = self
            .client
            .unary_with_timeout("ExecWait", request, timeout)
            .await
            .map_err(map_op_health_error)?;
        if let Some(error) = response.error.as_ref() {
            if !is_unspecified(error.kind) {
                return Err(map_guest_control_error(error));
            }
        }
        let state = response
            .state
            .enum_value()
            .unwrap_or(pb::ExecState::EXEC_STATE_UNSPECIFIED);
        let terminal = terminal_from_state(state, response.visible_terminal_status.as_ref());
        Ok(WaitOutcome {
            running: terminal.is_none(),
            terminal,
        })
    }

    async fn close_stdin(&self, offset: u64, timeout: Duration) -> Result<(), ExecOpError> {
        let mut request = pb::CloseStdinRequest::new();
        request.metadata = MessageField::some(self.exec_metadata());
        request.offset = offset;
        let response: pb::CloseStdinResponse = self
            .client
            .unary_with_timeout("CloseStdin", request, timeout)
            .await
            .map_err(map_op_health_error)?;
        if let Some(error) = response.error.as_ref() {
            if !is_unspecified(error.kind) {
                return Err(map_guest_control_error(error));
            }
        }
        Ok(())
    }
}

fn ack_result(ack: &pb::ControlAck) -> Result<(), ExecOpError> {
    if let Some(error) = ack.error.as_ref() {
        if !is_unspecified(error.kind) {
            return Err(map_guest_control_error(error));
        }
    }
    Ok(())
}

fn terminal_from_state(
    state: pb::ExecState,
    status: Option<&pb::TerminalStatus>,
) -> Option<TerminalKind> {
    match state {
        pb::ExecState::EXEC_STATE_EXITED => {
            let code = match status.and_then(|status| status.outcome.as_ref()) {
                Some(pb::terminal_status::Outcome::ExitCode(code)) => *code,
                Some(pb::terminal_status::Outcome::StatusCode(code)) => *code,
                _ => 0,
            };
            Some(TerminalKind::Exited(code))
        }
        pb::ExecState::EXEC_STATE_SIGNALED => {
            let signal = match status.and_then(|status| status.outcome.as_ref()) {
                Some(pb::terminal_status::Outcome::Signal(signal)) => *signal,
                _ => 0,
            };
            Some(TerminalKind::Signaled(signal))
        }
        pb::ExecState::EXEC_STATE_CANCELLED
        | pb::ExecState::EXEC_STATE_SLOW_CONSUMER_CANCELLED => Some(TerminalKind::Error("cancelled")),
        pb::ExecState::EXEC_STATE_LOST_GUESTD => Some(TerminalKind::Error("lost-guestd")),
        pb::ExecState::EXEC_STATE_REAPED => Some(TerminalKind::Error("reaped")),
        pb::ExecState::EXEC_STATE_PROTOCOL_ERROR => Some(TerminalKind::Error("protocol-error")),
        _ => None,
    }
}

fn is_unspecified(kind: EnumOrUnknown<pb::GuestControlErrorKind>) -> bool {
    matches!(
        kind.enum_value(),
        Ok(pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_UNSPECIFIED)
    )
}

fn map_guest_control_error(error: &pb::GuestControlError) -> ExecOpError {
    use pb::GuestControlErrorKind as K;
    match error.kind.enum_value() {
        Ok(K::GUEST_CONTROL_ERROR_KIND_AUTH_FAILED) => ExecOpError::Auth,
        Ok(K::GUEST_CONTROL_ERROR_KIND_STALE_SESSION) => ExecOpError::Auth,
        Ok(K::GUEST_CONTROL_ERROR_KIND_TRANSPORT_UNREACHABLE) => ExecOpError::Transport,
        Ok(K::GUEST_CONTROL_ERROR_KIND_GUEST_CONTROL_UNAVAILABLE_OLD_GENERATION) => {
            ExecOpError::OldGeneration
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_PROTOCOL_ERROR) => {
            ExecOpError::Guest(GuestOpError::Protocol)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_MAX_CHUNK_EXCEEDED) => {
            ExecOpError::Guest(GuestOpError::MaxChunkExceeded)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_STDIN_BACKPRESSURE) => {
            ExecOpError::Guest(GuestOpError::StdinBackpressure)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_STDIN_CLOSED
        | K::GUEST_CONTROL_ERROR_KIND_STDIN_CLOSED_BY_PROCESS) => {
            ExecOpError::Guest(GuestOpError::StdinClosed)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_STDIN_NOT_OPEN) => {
            ExecOpError::Guest(GuestOpError::StdinNotOpen)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_STDIN_OFFSET_MISMATCH
        | K::GUEST_CONTROL_ERROR_KIND_OFFSET_EXPIRED
        | K::GUEST_CONTROL_ERROR_KIND_OFFSET_IN_FUTURE
        | K::GUEST_CONTROL_ERROR_KIND_OFFSET_EXHAUSTED) => {
            ExecOpError::Guest(GuestOpError::OffsetMismatch)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_EXEC_NOT_FOUND) => {
            ExecOpError::Guest(GuestOpError::ExecNotFound)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_EXEC_ALREADY_EXITED
        | K::GUEST_CONTROL_ERROR_KIND_EXEC_EXPIRED) => {
            ExecOpError::Guest(GuestOpError::ExecAlreadyExited)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_CONTROL_SEQ_MISMATCH) => {
            ExecOpError::Guest(GuestOpError::ControlSeqMismatch)
        }
        Ok(K::GUEST_CONTROL_ERROR_KIND_RATE_LIMITED) => {
            ExecOpError::Guest(GuestOpError::RateLimited)
        }
        _ => ExecOpError::Guest(GuestOpError::Other),
    }
}

fn map_op_health_error(error: GuestControlHealthError) -> ExecOpError {
    match error {
        GuestControlHealthError::TransportIo
        | GuestControlHealthError::Ttrpc
        | GuestControlHealthError::Signer => ExecOpError::Transport,
        GuestControlHealthError::Timeout => ExecOpError::Timeout,
        GuestControlHealthError::AuthFailed | GuestControlHealthError::StaleSession => {
            ExecOpError::Auth
        }
        GuestControlHealthError::Protocol => ExecOpError::Protocol,
    }
}

fn map_op_health_error_for_establish(error: GuestControlHealthError) -> ExecEstablishError {
    op_to_establish(map_op_health_error(error))
}

fn map_establish_health_error(error: GuestControlHealthError) -> ExecEstablishError {
    match error {
        GuestControlHealthError::TransportIo
        | GuestControlHealthError::Ttrpc
        | GuestControlHealthError::Signer => ExecEstablishError::Transport,
        GuestControlHealthError::Timeout => ExecEstablishError::Timeout,
        GuestControlHealthError::AuthFailed | GuestControlHealthError::StaleSession => {
            ExecEstablishError::Auth
        }
        GuestControlHealthError::Protocol => ExecEstablishError::Protocol,
    }
}

fn op_to_establish(error: ExecOpError) -> ExecEstablishError {
    match error {
        ExecOpError::Transport => ExecEstablishError::Transport,
        ExecOpError::Auth => ExecEstablishError::Auth,
        ExecOpError::Protocol => ExecEstablishError::Protocol,
        ExecOpError::Timeout => ExecEstablishError::Timeout,
        ExecOpError::OldGeneration => ExecEstablishError::OldGeneration,
        ExecOpError::Capability => ExecEstablishError::Capability,
        ExecOpError::Guest(inner) => ExecEstablishError::Guest(inner),
    }
}

// ===========================================================================
// Tests (WR16 matrix f: per-capability fail-closed gating). `gate_capabilities`
// is a pure function over the guest's advertised capability set, so the gate is
// unit-tested directly without a live transport.
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    fn cap(value: pb::GuestCapability) -> EnumOrUnknown<pb::GuestCapability> {
        EnumOrUnknown::new(value)
    }

    /// The full capability set a TTY exec needs.
    fn full_tty_caps() -> Vec<EnumOrUnknown<pb::GuestCapability>> {
        vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_TTY),
            cap(pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE),
        ]
    }

    #[test]
    fn no_exec_capability_is_old_generation() {
        // A guest advertising only health/capabilities (no exec) is an old
        // generation: fail closed to the dedicated old-generation slug (exit
        // 70, NO SSH fallback), never a transport error.
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_HEALTH),
            cap(pb::GuestCapability::GUEST_CAPABILITY_CAPABILITIES),
        ];
        assert_eq!(
            gate_capabilities(&caps, false),
            Err(ExecEstablishError::OldGeneration)
        );
        assert_eq!(
            gate_capabilities(&caps, true),
            Err(ExecEstablishError::OldGeneration)
        );
    }

    #[test]
    fn exec_without_signals_is_capability_unavailable() {
        let caps = vec![cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED)];
        assert_eq!(
            gate_capabilities(&caps, false),
            Err(ExecEstablishError::Capability)
        );
    }

    #[test]
    fn non_tty_session_succeeds_without_tty_caps() {
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
        ];
        let negotiated = gate_capabilities(&caps, false).expect("non-tty session is allowed");
        assert_eq!(
            negotiated,
            NegotiatedCaps {
                tty: false,
                signals: true,
                tty_resize: false,
                output: false,
            }
        );
    }

    #[test]
    fn negotiated_caps_reflect_output_and_resize_advertisements() {
        // The cap snapshot used for per-op gating (F6) reflects exactly what the
        // guest advertised: ExecLogs → output, TtyResize → tty_resize.
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_TTY),
            cap(pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
        ];
        let negotiated = gate_capabilities(&caps, true).expect("full tty caps allowed");
        assert_eq!(
            negotiated,
            NegotiatedCaps {
                tty: true,
                signals: true,
                tty_resize: true,
                output: true,
            }
        );
    }

    #[test]
    fn tty_session_requires_exec_tty_and_tty_resize() {
        // Missing EXEC_TTY.
        let no_exec_tty = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE),
        ];
        assert_eq!(
            gate_capabilities(&no_exec_tty, true),
            Err(ExecEstablishError::Capability)
        );
        // Missing TTY_RESIZE.
        let no_resize = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_TTY),
        ];
        assert_eq!(
            gate_capabilities(&no_resize, true),
            Err(ExecEstablishError::Capability)
        );
        // A non-tty session does not need the tty caps even when absent.
        assert!(gate_capabilities(&no_exec_tty, false).is_ok());
    }

    #[test]
    fn full_capability_set_passes_for_tty_and_non_tty() {
        assert!(gate_capabilities(&full_tty_caps(), true).is_ok());
        assert!(gate_capabilities(&full_tty_caps(), false).is_ok());
    }
}
