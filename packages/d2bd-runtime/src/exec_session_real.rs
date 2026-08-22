//! Resource-backed exec connector and named-stream helpers.
//!
//! Attached execution is admitted through `EphemeralProcess` resources and
//! ComponentSession named streams. The former direct guest-control connector
//! remains test-only characterization coverage so production composition cannot
//! reach feature-specific `ExecCreate`.

#[cfg(any(test, feature = "test-support"))]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::ResourceRef;

#[cfg(any(test, feature = "test-support"))]
use crate::exec_session::ExecOpDeadlines;
use crate::exec_session::{
    ComponentSessionExecClient, Established, ExecEstablishError, ExecGuestClient,
    ExecGuestConnector, ExecOpError, ExecSessionInfo, ExecStartSpec,
};
pub use crate::exec_session::{
    ack_result, build_exec_create_request, gate_capabilities, is_unspecified,
    map_establish_health_error, map_guest_control_error, map_op_health_error,
    map_op_health_error_for_establish, op_to_establish, terminal_from_state,
};
#[cfg(any(test, feature = "test-support"))]
use crate::guest_control_bridge::connect_and_build_client_for_tests;
#[cfg(any(test, feature = "test-support"))]
use crate::guest_control_bridge::{
    BrokerSigner, GUEST_CONTROL_ATTEMPT_CAP, ProbeParams, VMADDR_CID_HOST,
    connect_and_build_client, host_nonce,
};
#[cfg(any(test, feature = "test-support"))]
use crate::guest_control_health::{
    AttemptBudget, GuestControlHealthError, TtrpcGuestControlClient, probe_guest_control_health,
};
#[cfg(any(test, feature = "test-support"))]
use crate::terminal_session::{
    OutputStreamSel, ReadOutputOutcome, TerminalBackend, WaitOutcome, WriteStdinOutcome,
};
#[cfg(any(test, feature = "test-support"))]
use d2b_contracts_broker::broker_wire::BrokerCallerRole;
#[cfg(any(test, feature = "test-support"))]
use d2b_contracts_control::guest_proto as pb;
#[cfg(any(test, feature = "test-support"))]
use d2b_contracts_control::guest_wire::GUEST_CONTROL_PROTOCOL_VERSION;
#[cfg(any(test, feature = "test-support"))]
use protobuf::{EnumOrUnknown, MessageField};

/// Absolute deadline for the whole establish phase (connect + auth handshake:
/// `CONNECT`-ack, Hello, sign, Authenticate, sign, Health).
/// `ExecCreate` uses a separate fresh per-op deadline (`ExecOpDeadlines`).
///
/// The establish phase has six sequential operations, each capped at
/// `GUEST_CONTROL_ATTEMPT_CAP` (3 s). Under heavy guest load - for example,
/// while a GUI application (Firefox, etc.) is doing its initial virtiofs burst
/// loading hundreds of shared libraries - every operation can approach its
/// per-op cap, requiring up to 6 × 3 s = 18 s of budget. 20 s leaves 2 s of
/// headroom for scheduling jitter without changing the per-op cap or protocol.
pub const ESTABLISH_TIMEOUT: Duration = Duration::from_secs(20);

/// The resource handle returned after an EphemeralProcess Create admission.
///
/// The handle contains only the resource identity and transport-neutral
/// stream metadata. Command data, credentials, paths, and process identities
/// remain owned by the Resource API and Process Provider.
#[derive(Clone, PartialEq, Eq)]
pub struct EphemeralProcessHandle {
    resource_ref: ResourceRef,
    stdout_offset: u64,
    stderr_offset: u64,
    control_sequence: u64,
}

impl std::fmt::Debug for EphemeralProcessHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EphemeralProcessHandle")
            .field("resource_ref", &"<redacted>")
            .field("stdout_offset", &self.stdout_offset)
            .field("stderr_offset", &self.stderr_offset)
            .field("control_sequence", &self.control_sequence)
            .finish()
    }
}

impl EphemeralProcessHandle {
    /// Construct a handle from an authenticated Resource API response.
    pub fn new(
        resource_ref: ResourceRef,
        stdout_offset: u64,
        stderr_offset: u64,
        control_sequence: u64,
    ) -> Result<Self, ExecEstablishError> {
        if resource_ref.resource_type().as_str() != "EphemeralProcess" {
            return Err(ExecEstablishError::Protocol);
        }
        Ok(Self {
            resource_ref,
            stdout_offset,
            stderr_offset,
            control_sequence,
        })
    }

    /// Borrow the created EphemeralProcess reference.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Return the initial stdout cursor.
    pub const fn stdout_offset(&self) -> u64 {
        self.stdout_offset
    }

    /// Return the initial stderr cursor.
    pub const fn stderr_offset(&self) -> u64 {
        self.stderr_offset
    }

    /// Return the initial in-stream control sequence.
    pub const fn control_sequence(&self) -> u64 {
        self.control_sequence
    }
}

/// Resource API seam used by the ComponentSession exec connector.
///
/// Implementations create an EphemeralProcess through the authenticated
/// Resource API, then open its admitted named stream. The connector has no
/// broker, socket, path, or direct child-process authority.
#[async_trait]
pub trait ProcessResourcePort: Send + Sync {
    /// Create one target-local EphemeralProcess resource.
    async fn create_ephemeral_process(
        &self,
        execution_ref: &ResourceRef,
        spec: &ExecStartSpec,
    ) -> Result<EphemeralProcessHandle, ExecEstablishError>;

    /// Attach the authenticated named stream for the created resource.
    async fn attach_process(
        &self,
        process: &EphemeralProcessHandle,
        tty: bool,
        initial_size: Option<(u32, u32)>,
    ) -> Result<Arc<dyn ExecGuestClient>, ExecEstablishError>;
}

/// Exec connector backed by Process/EphemeralProcess resources and a
/// ComponentSession named stream.
pub struct ResourceExecConnector<P> {
    port: P,
    execution_ref: ResourceRef,
}

impl<P> std::fmt::Debug for ResourceExecConnector<P> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ResourceExecConnector(<authenticated-resource-port>)")
    }
}

impl<P> ResourceExecConnector<P> {
    /// Bind the connector to one exact Host or Guest execution target.
    pub fn new(port: P, execution_ref: ResourceRef) -> Result<Self, ExecEstablishError> {
        if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
            return Err(ExecEstablishError::Protocol);
        }
        Ok(Self {
            port,
            execution_ref,
        })
    }

    /// Borrow the target bound before Resource API admission.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }
}

#[async_trait]
impl<P> ExecGuestConnector for ResourceExecConnector<P>
where
    P: ProcessResourcePort,
{
    async fn establish(&self, spec: &ExecStartSpec) -> Result<Established, ExecEstablishError> {
        if spec.detached {
            return Err(ExecEstablishError::Capability);
        }
        let process = self
            .port
            .create_ephemeral_process(&self.execution_ref, spec)
            .await?;
        let client = self
            .port
            .attach_process(&process, spec.tty, spec.term_size)
            .await?;
        Ok(Established {
            client,
            info: ExecSessionInfo {
                tty: spec.tty,
                stdout_offset: process.stdout_offset(),
                stderr_offset: process.stderr_offset(),
            },
            control_seq: process.control_sequence(),
            caps: crate::exec_session::NegotiatedCaps {
                tty: spec.tty,
                signals: true,
                tty_resize: spec.tty,
                output: true,
            },
        })
    }
}

/// Open the standard Process named stream on an already authenticated
/// ComponentSession driver.
pub async fn open_component_session_process<D>(
    driver: D,
    stream_number: u16,
) -> Result<Arc<dyn ExecGuestClient>, ExecEstablishError>
where
    D: d2b_session::ComponentSessionDriver + 'static,
{
    let client = ComponentSessionExecClient::open(
        driver,
        stream_number,
        d2b_contracts_zone_session::v3::component_session::MAX_NAMED_STREAM_QUEUE_BYTES,
        d2b_contracts_zone_session::v3::component_session::MAX_NAMED_STREAM_QUEUE_BYTES,
    )
    .await
    .map_err(map_component_session_exec_error)?;
    Ok(Arc::new(client))
}

fn map_component_session_exec_error(error: ExecOpError) -> ExecEstablishError {
    match error {
        ExecOpError::Timeout => ExecEstablishError::Timeout,
        ExecOpError::Auth => ExecEstablishError::Auth,
        ExecOpError::StaleSession => ExecEstablishError::OldGeneration,
        ExecOpError::Transport => ExecEstablishError::Transport,
        ExecOpError::Protocol
        | ExecOpError::OldGeneration
        | ExecOpError::Capability
        | ExecOpError::DetachedUnavailable
        | ExecOpError::Guest(_) => ExecEstablishError::Protocol,
    }
}

/// Production exec connector. Owns the resolved probe params + broker socket
/// path so it is `Send + Sync` and can move into the worker thread.
#[cfg(any(test, feature = "test-support"))]
pub struct RealExecConnector {
    params: ProbeParams,
    broker_socket_path: PathBuf,
    caller_role: BrokerCallerRole,
    deadlines: ExecOpDeadlines,
    /// Test-only: route the connect through the relaxed-directory test policy so
    /// a hermetic test can reach the genuine socket-missing transport branch
    /// under a non-root tempdir. Always `false` for the production constructor.
    #[cfg(any(test, feature = "test-support"))]
    allow_test_dirs: bool,
}

#[cfg(any(test, feature = "test-support"))]
impl RealExecConnector {
    pub fn new(
        params: ProbeParams,
        broker_socket_path: PathBuf,
        caller_role: BrokerCallerRole,
        deadlines: ExecOpDeadlines,
    ) -> Self {
        Self {
            params,
            broker_socket_path,
            caller_role,
            deadlines,
            #[cfg(any(test, feature = "test-support"))]
            allow_test_dirs: false,
        }
    }

    /// Test constructor that drives the real `establish` path but connects
    /// through the relaxed-directory test policy.
    #[cfg(test)]
    fn new_for_tests(
        params: ProbeParams,
        broker_socket_path: PathBuf,
        deadlines: ExecOpDeadlines,
    ) -> Self {
        Self {
            params,
            broker_socket_path,
            caller_role: BrokerCallerRole::NotAuthorized,
            deadlines,
            allow_test_dirs: true,
        }
    }

    /// Connect + build the guest-control client. Production always uses the
    /// state-root-validating connect; a test connector may opt into the
    /// relaxed-directory connect so it reaches the genuine socket-missing branch
    /// rather than tripping ownership pre-validation.
    fn connect_client(
        &self,
        budget: AttemptBudget,
    ) -> Result<TtrpcGuestControlClient, GuestControlHealthError> {
        #[cfg(any(test, feature = "test-support"))]
        if self.allow_test_dirs {
            return connect_and_build_client_for_tests(&self.params, budget);
        }
        connect_and_build_client(&self.params, budget)
    }
}

#[async_trait]
#[cfg(any(test, feature = "test-support"))]
impl ExecGuestConnector for RealExecConnector {
    async fn establish(&self, spec: &ExecStartSpec) -> Result<Established, ExecEstablishError> {
        let budget = AttemptBudget::from_now(ESTABLISH_TIMEOUT, GUEST_CONTROL_ATTEMPT_CAP);
        let signer = BrokerSigner::with_caller_role(
            self.broker_socket_path.clone(),
            budget,
            self.caller_role.clone(),
        );
        let nonce = host_nonce().map_err(|_| ExecEstablishError::Transport)?;
        let client = self
            .connect_client(budget)
            .map_err(map_establish_health_error)?;
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

        if let Some(error) = response.error.as_ref()
            && !is_unspecified(error.kind)
        {
            return Err(op_to_establish(map_guest_control_error(error)));
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
/// needs, returning the negotiated cap snapshot for per-op gating.
///
/// This runs AFTER a successful authenticated handshake, so the guest is a
/// guest-control generation that is up and reachable. A guest that does not
/// advertise the exec capabilities here therefore has exec **disabled or not
/// built in** (`guest.exec.enable = false`, or a partial generation) - it is
/// NOT the genuine "no guestd / old generation" case, which is detected earlier
/// at connect/probe time. Surface the capability slug (exit 70, no SSH
/// fallback) whose remediation points at enabling guest-control exec.
/// Authenticated exec client bound to one `exec_id` on one guest connection.
#[cfg(any(test, feature = "test-support"))]
struct RealExecClient {
    client: Arc<TtrpcGuestControlClient>,
    vm_id: String,
    guest_boot_id: String,
    exec_id: String,
}

#[cfg(any(test, feature = "test-support"))]
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
#[cfg(any(test, feature = "test-support"))]
impl TerminalBackend for RealExecClient {
    type Error = ExecOpError;

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
        if let Some(error) = response.error.as_ref()
            && !is_unspecified(error.kind)
        {
            return Err(map_guest_control_error(error));
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
        if let Some(error) = response.error.as_ref()
            && !is_unspecified(error.kind)
        {
            return Err(map_guest_control_error(error));
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
        if let Some(error) = response.error.as_ref()
            && !is_unspecified(error.kind)
        {
            return Err(map_guest_control_error(error));
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
        if let Some(error) = response.error.as_ref()
            && !is_unspecified(error.kind)
        {
            return Err(map_guest_control_error(error));
        }
        Ok(())
    }

    async fn cancel(&self, control_seq: u64, timeout: Duration) -> Result<(), ExecOpError> {
        let mut request = pb::ExecCancelRequest::new();
        request.metadata = MessageField::some(self.exec_metadata());
        request.control_seq = control_seq;
        request.reason =
            EnumOrUnknown::new(pb::ExecCancelReason::EXEC_CANCEL_REASON_CLIENT_DISCONNECT);
        let response: pb::ControlAck = self
            .client
            .unary_with_timeout("ExecCancel", request, timeout)
            .await
            .map_err(map_op_health_error)?;
        ack_result(&response)
    }
}

// ===========================================================================
// Tests (matrix f: per-capability fail-closed gating). `gate_capabilities`
// is a pure function over the guest's advertised capability set, so the gate is
// unit-tested directly without a live transport.
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec_session::NegotiatedCaps;

    fn cap(value: pb::GuestCapability) -> EnumOrUnknown<pb::GuestCapability> {
        EnumOrUnknown::new(value)
    }

    /// The full capability set a TTY exec needs.
    fn full_tty_caps() -> Vec<EnumOrUnknown<pb::GuestCapability>> {
        vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_TTY),
            cap(pb::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE),
        ]
    }

    #[test]
    fn no_exec_capability_is_capability_unavailable_after_auth() {
        // A guest advertising only health/capabilities (no exec) has reached
        // this gate AFTER authenticating, so it is up and reachable - exec is
        // disabled or not built in, NOT a genuine old generation (that is a
        // connect-time failure). Fail closed to the capability slug (exit 70,
        // NO SSH fallback) whose remediation points at `guest.exec.enable`.
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_HEALTH),
            cap(pb::GuestCapability::GUEST_CAPABILITY_CAPABILITIES),
        ];
        assert_eq!(
            gate_capabilities(&caps, false),
            Err(ExecEstablishError::Capability)
        );
        assert_eq!(
            gate_capabilities(&caps, true),
            Err(ExecEstablishError::Capability)
        );
    }

    #[test]
    fn exec_without_output_capability_is_capability_unavailable() {
        // EXEC_ATTACHED without EXEC_LOGS: every reachable session must stream
        // output, so the host fails closed rather than establishing a session
        // that can never deliver stdout/stderr.
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
        ];
        assert_eq!(
            gate_capabilities(&caps, false),
            Err(ExecEstablishError::Capability)
        );
        assert_eq!(
            gate_capabilities(&caps, true),
            Err(ExecEstablishError::Capability)
        );
    }

    #[test]
    fn exec_without_signals_is_capability_unavailable() {
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
        ];
        assert_eq!(
            gate_capabilities(&caps, false),
            Err(ExecEstablishError::Capability)
        );
    }

    #[test]
    fn non_tty_session_succeeds_without_tty_caps() {
        let caps = vec![
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED),
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
            cap(pb::GuestCapability::GUEST_CAPABILITY_SIGNALS),
        ];
        let negotiated = gate_capabilities(&caps, false).expect("non-tty session is allowed");
        assert_eq!(
            negotiated,
            NegotiatedCaps {
                tty: false,
                signals: true,
                tty_resize: false,
                output: true,
            }
        );
    }

    #[test]
    fn negotiated_caps_reflect_output_and_resize_advertisements() {
        // The cap snapshot used for per-op gating reflects exactly what the
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
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
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
            cap(pb::GuestCapability::GUEST_CAPABILITY_EXEC_LOGS),
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

    #[test]
    fn stale_component_session_maps_to_old_generation() {
        assert_eq!(
            map_component_session_exec_error(ExecOpError::StaleSession),
            ExecEstablishError::OldGeneration
        );
        assert_eq!(
            map_component_session_exec_error(ExecOpError::Transport),
            ExecEstablishError::Transport
        );
    }

    /// Daemon-side fail-closed complement to the CLI-side
    /// `vm_exec_old_generation_fails_closed_without_proxy_or_ssh`: when the real
    /// connector cannot reach the guest vsock (absent socket / an old
    /// generation that never shipped guest-control), `establish` fails CLOSED
    /// with the typed unreachable error. It never returns `Ok`, never proxies
    /// an exec op, and never falls back to SSH - the connector has exactly one
    /// success path, which requires a live authenticated handshake.
    ///
    /// This drives the REAL `establish` path through `new_for_tests`, whose
    /// connect uses the relaxed-directory test policy so the failure is the
    /// GENUINE `SocketMissing` transport branch (validated below) rather than
    /// the production state-root ownership pre-validation tripping first under a
    /// non-root tempdir. Because `connect_client` fails before any client is
    /// built, no `ExecCreate` (or any other exec op) is ever issued and there is
    /// no path to an SSH/raw fallback.
    #[tokio::test]
    async fn establish_against_absent_vsock_fails_closed_with_typed_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A socket path that does not exist: the guest-control endpoint is
        // absent, modelling an old generation with no guest-control listener.
        let absent_socket = dir.path().join("guest-control.sock");
        assert!(!absent_socket.exists());

        // Sanity: the relaxed-directory connect reaches the genuine
        // socket-missing branch (NOT a directory pre-validation failure) for
        // this exact (tempdir, absent socket) shape - so the establish failure
        // below is the real transport-unreachable path, not a false positive.
        let probe = crate::guest_control_vsock::connect_guest_control_vsock_for_tests(
            &absent_socket,
            dir.path(),
            Duration::from_millis(200),
        );
        assert_eq!(
            probe.failure(),
            Some(&crate::guest_control_vsock::GuestControlTransportFailure::SocketMissing),
            "the connect must fail at the genuine socket-missing branch"
        );

        let params = ProbeParams {
            vm_id: "work".to_owned(),
            socket_path: absent_socket,
            state_root: dir.path().to_path_buf(),
            expected_state_root_uid: 0,
            expected_state_root_gid: 0,
            expected_peer_uid: 0,
            expected_peer_gid: 0,
        };
        // A broker socket path that is never reached: the connect fails first,
        // so no broker sign and no exec op is ever attempted.
        let connector = RealExecConnector::new_for_tests(
            params,
            dir.path().join("broker.sock"),
            ExecOpDeadlines::default(),
        );

        let spec = ExecStartSpec {
            vm: "work".to_owned(),
            request_id: None,
            argv: vec!["true".to_owned()],
            tty: false,
            detached: false,
            env: Vec::new(),
            cwd: None,
            term_size: None,
        };

        let result = connector.establish(&spec).await;

        // Fail closed: a typed unreachable error, never Ok, never a silent
        // SSH/raw fallback. (`establish` has exactly one `Ok` arm, reached only
        // after a live authenticated handshake + `ExecCreate`.)
        assert_eq!(
            result.err(),
            Some(ExecEstablishError::Transport),
            "an absent guest-control endpoint must fail closed to the typed \
             transport-unreachable error, never establish or fall back"
        );
    }

    #[test]
    fn exec_create_request_leaves_user_unset() {
        // The exec target user is host-fixed by guestd (the workload user); the
        // daemon must NOT set the wire `user` field (guestd ignores it, and a
        // client must never be able to select/escalate the target user).
        let spec = ExecStartSpec {
            vm: "work".to_owned(),
            request_id: Some("workload-launch:0123456789abcdef0123456789abcdef".to_owned()),
            argv: vec!["true".to_owned()],
            tty: false,
            detached: false,
            env: Vec::new(),
            cwd: None,
            term_size: None,
        };
        let request = build_exec_create_request("work", &spec);
        assert_eq!(
            request
                .metadata
                .as_ref()
                .map(|metadata| metadata.request_id.as_str()),
            Some("workload-launch:0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            request.user.as_deref(),
            None,
            "the daemon must leave the wire user unset; guestd fixes the \
             target user host-side",
        );
        // Non-TTY exec must close stdin: guestd's non-TTY validators
        // (`validate_and_authorize` / `_detached`) reject `stdin_open`, so a
        // regression back to `stdin_open = true` makes every non-interactive
        // and detached exec fail `UnsupportedMode` before spawning.
        assert!(!request.tty, "non-tty spec must not request a tty");
        assert!(
            !request.stdin_open,
            "non-tty exec must close stdin so guestd's non-TTY validator accepts it",
        );
    }

    #[test]
    fn exec_create_request_opens_stdin_only_for_tty() {
        // Interactive TTY exec is the only mode guestd's
        // `validate_and_authorize_tty` accepts with an open stdin.
        let spec = ExecStartSpec {
            vm: "work".to_owned(),
            request_id: None,
            argv: vec!["true".to_owned()],
            tty: true,
            detached: false,
            env: Vec::new(),
            cwd: None,
            term_size: Some((24, 80)),
        };
        let request = build_exec_create_request("work", &spec);
        assert!(request.tty, "tty spec must request a tty");
        assert!(
            request.stdin_open,
            "tty exec must open stdin for guestd's interactive validator",
        );
    }

    struct StubResourceBackend;

    #[async_trait]
    impl TerminalBackend for StubResourceBackend {
        type Error = ExecOpError;

        async fn write_stdin(
            &self,
            offset: u64,
            data: Vec<u8>,
            eof: bool,
            _timeout: Duration,
        ) -> Result<WriteStdinOutcome, Self::Error> {
            Ok(WriteStdinOutcome {
                accepted_len: data.len() as u64,
                next_offset: offset + data.len() as u64,
                backpressured: false,
                stdin_closed: eof,
            })
        }

        async fn read_output(
            &self,
            _stream: OutputStreamSel,
            offset: u64,
            _max_len: u64,
            _wait: bool,
            _timeout_ms: u64,
            _timeout: Duration,
        ) -> Result<ReadOutputOutcome, Self::Error> {
            Ok(ReadOutputOutcome {
                data: Vec::new(),
                next_offset: offset,
                eof: true,
                dropped_bytes: 0,
                truncated: false,
                timed_out: false,
            })
        }

        async fn signal(
            &self,
            _control_seq: u64,
            _signo: u32,
            _timeout: Duration,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn resize(
            &self,
            _control_seq: u64,
            _rows: u32,
            _cols: u32,
            _timeout: Duration,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn wait(
            &self,
            _timeout_ms: u64,
            _timeout: Duration,
        ) -> Result<WaitOutcome, Self::Error> {
            Ok(WaitOutcome {
                running: true,
                terminal: None,
            })
        }

        async fn close_stdin(&self, _offset: u64, _timeout: Duration) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingResourcePort {
        created: std::sync::Mutex<Vec<ResourceRef>>,
    }

    #[async_trait]
    impl ProcessResourcePort for RecordingResourcePort {
        async fn create_ephemeral_process(
            &self,
            execution_ref: &ResourceRef,
            _spec: &ExecStartSpec,
        ) -> Result<EphemeralProcessHandle, ExecEstablishError> {
            self.created.lock().unwrap().push(execution_ref.clone());
            EphemeralProcessHandle::new(
                ResourceRef::parse("EphemeralProcess/run").unwrap(),
                3,
                4,
                5,
            )
        }

        async fn attach_process(
            &self,
            _process: &EphemeralProcessHandle,
            _tty: bool,
            _initial_size: Option<(u32, u32)>,
        ) -> Result<Arc<dyn ExecGuestClient>, ExecEstablishError> {
            Ok(Arc::new(StubResourceBackend))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resource_exec_connector_creates_and_attaches_one_ephemeral_process() {
        let port = RecordingResourcePort::default();
        let connector =
            ResourceExecConnector::new(port, ResourceRef::parse("Guest/work").unwrap()).unwrap();
        let spec = ExecStartSpec {
            vm: "work".to_owned(),
            request_id: None,
            argv: vec!["true".to_owned()],
            tty: false,
            detached: false,
            env: Vec::new(),
            cwd: None,
            term_size: None,
        };
        let established = connector.establish(&spec).await.unwrap();
        assert_eq!(established.info.stdout_offset, 3);
        assert_eq!(established.info.stderr_offset, 4);
        assert_eq!(established.control_seq, 5);
        assert!(established.caps.output);
        assert!(!established.caps.tty);
    }

    #[test]
    fn resource_exec_connector_rejects_non_execution_targets() {
        assert_eq!(
            ResourceExecConnector::<RecordingResourcePort>::new(
                RecordingResourcePort::default(),
                ResourceRef::parse("Process/not-a-target").unwrap(),
            )
            .unwrap_err()
            .slug(),
            ExecEstablishError::Protocol.slug()
        );
    }

    #[test]
    fn ephemeral_process_handle_debug_redacts_resource_identity() {
        let handle = EphemeralProcessHandle::new(
            ResourceRef::parse("EphemeralProcess/secret-command").unwrap(),
            0,
            0,
            0,
        )
        .unwrap();
        let rendered = format!("{handle:?}");
        assert!(!rendered.contains("secret-command"));
        assert!(rendered.contains("resource_ref"));
    }
}
