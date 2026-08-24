//! Guest-domain EphemeralProcess execution and attachment contracts.
//!
//! The target-local execution command and session protocol are represented here
//! as typed resource/session intents. The request carries an
//! `EphemeralProcess` reference and bounded terminal geometry only; command
//! argv, environment, paths, and credentials are resolved by the signed
//! Provider template and never cross this boundary.

use std::{
    fmt,
    future::Future,
    sync::atomic::{AtomicU8, Ordering},
};

use d2b_contracts_control::public_wire::{
    NamedProcessStreamRequestFrame, NamedProcessStreamResponseFrame,
};
use d2b_contracts_resource::v3::{
    ResourceRef,
    execution_policy::{BoundedToken, DurationMs},
    process::{EphemeralProcessSpec, ExecutionSpec, ProcessClass},
};
use d2b_provider_toolkit::{ComponentSessionDriver, StreamEvent, StreamId};

/// The retained failed-job TTL required for detached guest execution.
pub const DETACHED_FAILED_TTL_MS: u64 = 24 * 60 * 60 * 1_000;
/// Default per-direction named-stream credit for guest attachment.
pub const GUEST_EXEC_STREAM_CREDIT: u32 = 256 * 1024;
/// The authenticated named stream used by guest Process attachments.
pub const GUEST_EXEC_STREAM_NAME: &str = "process";

const STREAM_OPEN: u8 = 0;
const STREAM_CLOSING: u8 = 1;
const STREAM_CLOSED: u8 = 2;

/// Bounded terminal geometry supplied to an attach operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TtySize {
    /// Terminal columns.
    pub columns: u16,
    /// Terminal rows.
    pub rows: u16,
}

impl TtySize {
    /// Validate non-zero terminal geometry.
    pub const fn new(columns: u16, rows: u16) -> Result<Self, GuestExecError> {
        if columns == 0 || rows == 0 {
            Err(GuestExecError::InvalidTtySize)
        } else {
            Ok(Self { columns, rows })
        }
    }
}

/// A typed guest exec create request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestExecRequest {
    process_ref: ResourceRef,
    execution_ref: ResourceRef,
    template: BoundedToken,
    detached: bool,
    tty: bool,
    initial_size: Option<TtySize>,
}

impl GuestExecRequest {
    /// Construct an EphemeralProcess request.
    pub fn new(
        process_ref: ResourceRef,
        execution_ref: ResourceRef,
        template: BoundedToken,
        detached: bool,
        tty: bool,
        initial_size: Option<TtySize>,
    ) -> Result<Self, GuestExecError> {
        if process_ref.resource_type().as_str() != "EphemeralProcess"
            || execution_ref.resource_type().as_str() != "Guest"
        {
            return Err(GuestExecError::WrongResourceType);
        }
        if tty && initial_size.is_none() {
            return Err(GuestExecError::TtySizeRequired);
        }
        if !tty && initial_size.is_some() {
            return Err(GuestExecError::TtySizeWithoutTty);
        }
        Ok(Self {
            process_ref,
            execution_ref,
            template,
            detached,
            tty,
            initial_size,
        })
    }

    /// Borrow the Zone-local EphemeralProcess reference.
    pub const fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    /// Borrow the owning Guest reference.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the signed template identifier.
    pub const fn template(&self) -> &BoundedToken {
        &self.template
    }

    /// Whether the process is detached and retained for later observation.
    pub const fn detached(&self) -> bool {
        self.detached
    }

    /// Whether a named terminal stream is requested.
    pub const fn tty(&self) -> bool {
        self.tty
    }

    /// Return initial terminal geometry.
    pub const fn initial_size(&self) -> Option<TtySize> {
        self.initial_size
    }

    /// Build the resource-owned EphemeralProcess spec for this request.
    ///
    /// The command remains a signed template concern. Only the Guest target,
    /// worker classification, and bounded lifecycle retention cross this
    /// Provider seam.
    pub fn ephemeral_process_spec(&self) -> Result<EphemeralProcessSpec, GuestExecError> {
        let execution = ExecutionSpec::minimal(
            self.execution_ref.clone(),
            ProcessClass::Worker,
            self.template.clone(),
        )
        .map_err(|_| GuestExecError::InvalidProcessSpec)?;
        let runtime_deadline = if self.detached { "24h" } else { "6h" };
        EphemeralProcessSpec::new(
            execution,
            DurationMs::parse("60s", 1_000, 3_600_000)
                .map_err(|_| GuestExecError::InvalidProcessSpec)?,
            DurationMs::parse(runtime_deadline, 1_000, 86_400_000)
                .map_err(|_| GuestExecError::InvalidProcessSpec)?,
            DurationMs::parse("1h", 0, 7 * 86_400_000)
                .map_err(|_| GuestExecError::InvalidProcessSpec)?,
            DurationMs::parse("24h", 0, 30 * 86_400_000)
                .map_err(|_| GuestExecError::InvalidProcessSpec)?,
            false,
        )
        .map_err(|_| GuestExecError::InvalidProcessSpec)
    }
}

/// A typed attach request replacing the userd socket protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachRequest {
    process_ref: ResourceRef,
    tty: bool,
    initial_size: Option<TtySize>,
}

impl AttachRequest {
    /// Construct an attach request with retained tty validation.
    pub fn new(
        process_ref: ResourceRef,
        tty: bool,
        initial_size: Option<TtySize>,
    ) -> Result<Self, GuestExecError> {
        if process_ref.resource_type().as_str() != "EphemeralProcess" {
            return Err(GuestExecError::WrongResourceType);
        }
        if tty && initial_size.is_none() {
            return Err(GuestExecError::TtySizeRequired);
        }
        if !tty && initial_size.is_some() {
            return Err(GuestExecError::TtySizeWithoutTty);
        }
        Ok(Self {
            process_ref,
            tty,
            initial_size,
        })
    }

    /// Borrow the target EphemeralProcess.
    pub const fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    /// Whether the attachment includes a terminal.
    pub const fn tty(&self) -> bool {
        self.tty
    }

    /// Return requested terminal geometry.
    pub const fn initial_size(&self) -> Option<TtySize> {
        self.initial_size
    }
}

/// Opaque named ComponentSession stream identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NamedAttachmentStream(BoundedToken);

impl NamedAttachmentStream {
    /// Construct a bounded stream name.
    pub fn new(name: BoundedToken) -> Self {
        Self(name)
    }

    /// Construct the fixed guest Process attachment stream name.
    pub fn process() -> Self {
        Self::new(BoundedToken::parse(GUEST_EXEC_STREAM_NAME).expect("valid process stream name"))
    }

    /// Borrow the stream name for the session mux.
    pub const fn name(&self) -> &BoundedToken {
        &self.0
    }

    /// Return the stream name's bounded text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for NamedAttachmentStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NamedAttachmentStream(<redacted>)")
    }
}

/// An attached guest process stream backed by an authenticated
/// ComponentSession named stream.
pub struct ComponentSessionAttachment<D> {
    driver: D,
    stream: StreamId,
    name: NamedAttachmentStream,
    state: AtomicU8,
}

impl<D> fmt::Debug for ComponentSessionAttachment<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ComponentSessionAttachment(<redacted>)")
    }
}

impl<D> ComponentSessionAttachment<D>
where
    D: ComponentSessionDriver,
{
    /// Open one named stream after attach validation has succeeded.
    pub async fn open(
        driver: D,
        stream_number: u16,
        name: NamedAttachmentStream,
    ) -> Result<Self, GuestExecError> {
        if name.as_str() != GUEST_EXEC_STREAM_NAME {
            return Err(GuestExecError::InvalidStreamName);
        }
        let stream = StreamId::new(stream_number).map_err(|_| GuestExecError::StreamUnavailable)?;
        driver
            .open_named_stream(stream, GUEST_EXEC_STREAM_CREDIT, GUEST_EXEC_STREAM_CREDIT)
            .await
            .map_err(|_| GuestExecError::StreamUnavailable)?;
        Ok(Self {
            driver,
            stream,
            name,
            state: AtomicU8::new(STREAM_OPEN),
        })
    }

    /// Borrow the opaque stream name.
    pub const fn name(&self) -> &NamedAttachmentStream {
        &self.name
    }

    /// Whether the named stream has been closed or reset.
    pub fn is_closed(&self) -> bool {
        self.state.load(Ordering::Acquire) == STREAM_CLOSED
    }

    /// Send one bounded logical stream message.
    pub async fn send(&self, bytes: Vec<u8>) -> Result<(), GuestExecError> {
        if bytes.is_empty()
            || bytes.len()
                > d2b_contracts_zone_session::v3::component_session::MAX_LOGICAL_MESSAGE_BYTES
                    as usize
        {
            return Err(GuestExecError::InvalidStreamPayload);
        }
        if self.state.load(Ordering::Acquire) != STREAM_OPEN {
            return Err(GuestExecError::StreamUnavailable);
        }
        self.driver
            .send_named_stream(self.stream, bytes)
            .await
            .map_err(|_| GuestExecError::StreamUnavailable)
    }

    /// Send one canonical Process named-stream request frame.
    pub async fn send_frame(
        &self,
        frame: &NamedProcessStreamRequestFrame,
    ) -> Result<(), GuestExecError> {
        if frame.request_id == 0 {
            return Err(GuestExecError::InvalidStreamPayload);
        }
        let bytes = serde_json::to_vec(frame).map_err(|_| GuestExecError::InvalidStreamPayload)?;
        self.send(bytes).await
    }

    /// Receive the next typed stream event.
    pub async fn receive(&self) -> Result<StreamEvent, GuestExecError> {
        if self.state.load(Ordering::Acquire) != STREAM_OPEN {
            return Err(GuestExecError::StreamUnavailable);
        }
        let event = self
            .driver
            .receive_named_stream()
            .await
            .map_err(|_| GuestExecError::StreamUnavailable)?;
        if matches!(
            event,
            StreamEvent::RemoteClosed { .. } | StreamEvent::Reset { .. }
        ) {
            self.state.store(STREAM_CLOSED, Ordering::Release);
        }
        Ok(event)
    }

    /// Receive and decode one canonical Process named-stream response frame.
    pub async fn receive_frame(&self) -> Result<NamedProcessStreamResponseFrame, GuestExecError> {
        let event = self.receive().await?;
        let StreamEvent::Data { stream, bytes } = event else {
            return Err(GuestExecError::InvalidStreamPayload);
        };
        if stream != self.stream {
            return Err(GuestExecError::InvalidStreamPayload);
        }
        let frame: NamedProcessStreamResponseFrame =
            serde_json::from_slice(&bytes).map_err(|_| GuestExecError::InvalidStreamPayload)?;
        if frame.request_id == 0 {
            return Err(GuestExecError::InvalidStreamPayload);
        }
        Ok(frame)
    }

    /// Close this named stream.
    pub async fn close(&self) -> Result<(), GuestExecError> {
        if self
            .state
            .compare_exchange(
                STREAM_OPEN,
                STREAM_CLOSING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return Ok(());
        }
        match self.driver.close_named_stream(self.stream).await {
            Ok(()) => {
                self.state.store(STREAM_CLOSED, Ordering::Release);
                Ok(())
            }
            Err(_) => {
                self.state.store(STREAM_OPEN, Ordering::Release);
                Err(GuestExecError::StreamUnavailable)
            }
        }
    }

    /// Reset this named stream after cancellation or protocol failure.
    pub async fn reset(&self) -> Result<(), GuestExecError> {
        if self
            .state
            .compare_exchange(
                STREAM_OPEN,
                STREAM_CLOSING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return Ok(());
        }
        match self.driver.reset_named_stream(self.stream).await {
            Ok(()) => {
                self.state.store(STREAM_CLOSED, Ordering::Release);
                Ok(())
            }
            Err(_) => {
                self.state.store(STREAM_OPEN, Ordering::Release);
                Err(GuestExecError::StreamUnavailable)
            }
        }
    }
}

/// Typed guest execution errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestExecError {
    /// A request names the wrong ResourceType.
    WrongResourceType,
    /// Terminal geometry is zero.
    InvalidTtySize,
    /// TTY mode requires geometry.
    TtySizeRequired,
    /// Non-TTY mode may not carry geometry.
    TtySizeWithoutTty,
    /// The process is not attachable in its current phase.
    NotAttachable,
    /// The ComponentSession stream could not be opened.
    StreamUnavailable,
    /// The request used a stream name outside the Process contract.
    InvalidStreamName,
    /// The stream payload was empty or exceeded the ComponentSession bound.
    InvalidStreamPayload,
    /// The resource-owned EphemeralProcess spec could not be built.
    InvalidProcessSpec,
}

impl fmt::Display for GuestExecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongResourceType => "guest-exec-resource-type-invalid",
            Self::InvalidTtySize => "guest-exec-tty-size-invalid",
            Self::TtySizeRequired => "guest-exec-tty-size-required",
            Self::TtySizeWithoutTty => "guest-exec-tty-size-unexpected",
            Self::NotAttachable => "guest-exec-process-not-attachable",
            Self::StreamUnavailable => "guest-exec-stream-unavailable",
            Self::InvalidStreamName => "guest-exec-stream-name-invalid",
            Self::InvalidStreamPayload => "guest-exec-stream-payload-invalid",
            Self::InvalidProcessSpec => "guest-exec-process-spec-invalid",
        })
    }
}

impl std::error::Error for GuestExecError {}

/// The typed guest resource/session seam used by the runtime Provider.
pub trait GuestExecPort: Send + Sync {
    /// Create the EphemeralProcess resource.
    fn create(
        &self,
        request: &GuestExecRequest,
    ) -> impl Future<Output = Result<(), GuestExecError>> + Send;

    /// Attach one named stream after ResourceClient Watch/Get confirms
    /// attachable state.
    fn attach(
        &self,
        request: &AttachRequest,
    ) -> impl Future<Output = Result<NamedAttachmentStream, GuestExecError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process() -> ResourceRef {
        ResourceRef::parse("EphemeralProcess/exec-1").unwrap()
    }

    fn guest() -> ResourceRef {
        ResourceRef::parse("Guest/dev-vm").unwrap()
    }

    #[test]
    fn detached_exec_is_a_guest_ephemeral_process_with_a_failed_ttl() {
        let request = GuestExecRequest::new(
            process(),
            guest(),
            BoundedToken::parse("shell-terminal").unwrap(),
            true,
            false,
            None,
        )
        .unwrap();
        assert!(request.detached());
        assert_eq!(DETACHED_FAILED_TTL_MS, 86_400_000);
    }

    #[test]
    fn detached_exec_builds_the_guest_ephemeral_process_spec() {
        let request = GuestExecRequest::new(
            process(),
            guest(),
            BoundedToken::parse("shell-terminal").unwrap(),
            true,
            false,
            None,
        )
        .unwrap();
        let spec = request.ephemeral_process_spec().unwrap();
        assert_eq!(spec.execution().execution_ref(), &guest());
        assert_eq!(
            spec.execution().process_class(),
            d2b_contracts_resource::v3::process::ProcessClass::Worker
        );
        assert_eq!(spec.execution().template().as_str(), "shell-terminal");
        assert_eq!(spec.failed_ttl().as_millis(), DETACHED_FAILED_TTL_MS);
        let attached = GuestExecRequest::new(
            process(),
            guest(),
            BoundedToken::parse("shell-terminal").unwrap(),
            false,
            false,
            None,
        )
        .unwrap();
        assert_eq!(
            attached
                .ephemeral_process_spec()
                .unwrap()
                .runtime_deadline()
                .as_millis(),
            21_600_000
        );
    }

    #[test]
    fn attach_validation_preserves_tty_and_stream_error_boundaries() {
        assert_eq!(
            AttachRequest::new(process(), true, None).unwrap_err(),
            GuestExecError::TtySizeRequired
        );
        let size = TtySize::new(80, 24).unwrap();
        assert!(AttachRequest::new(process(), true, Some(size)).is_ok());
        assert_eq!(
            AttachRequest::new(process(), false, Some(size)).unwrap_err(),
            GuestExecError::TtySizeWithoutTty
        );
        assert_eq!(
            NamedAttachmentStream::process().as_str(),
            GUEST_EXEC_STREAM_NAME
        );
        assert_eq!(
            GuestExecError::InvalidStreamName.to_string(),
            "guest-exec-stream-name-invalid"
        );
    }

    #[test]
    fn process_attachment_round_trips_the_shared_named_stream_frame() {
        let request = d2b_contracts_control::public_wire::NamedProcessStreamRequestFrame::new(
            9,
            d2b_contracts_control::public_wire::NamedProcessStreamRequest::Close,
        );
        let encoded = serde_json::to_vec(&request).unwrap();
        let decoded: NamedProcessStreamRequestFrame = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, request);

        let response = d2b_contracts_control::public_wire::NamedProcessStreamResponseFrame::new(
            decoded.request_id,
            d2b_contracts_control::public_wire::NamedProcessStreamResponse::Closed(
                d2b_contracts_control::public_wire::ExecCloseResult { stdin_closed: true },
            ),
        );
        let response_bytes = serde_json::to_vec(&response).unwrap();
        let response: NamedProcessStreamResponseFrame =
            serde_json::from_slice(&response_bytes).unwrap();
        assert_eq!(response.request_id, 9);
    }
}
