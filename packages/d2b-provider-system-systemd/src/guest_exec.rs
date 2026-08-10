//! Guest-domain EphemeralProcess execution and attachment contracts.
//!
//! The former guestd command and userd socket protocols are represented here
//! as typed resource/session intents.  The request carries an
//! `EphemeralProcess` reference and bounded terminal geometry only; command
//! argv, environment, paths, and credentials are resolved by the signed
//! Provider template and never cross this boundary.

use std::{fmt, future::Future};

use d2b_contracts::v3::{ResourceRef, execution_policy::BoundedToken};
use d2b_provider_toolkit::{ComponentSessionDriver, StreamEvent, StreamId};

/// The retained failed-job TTL required for detached guest execution.
pub const DETACHED_FAILED_TTL_MS: u64 = 24 * 60 * 60 * 1_000;
/// Default per-direction named-stream credit for guest attachment.
pub const GUEST_EXEC_STREAM_CREDIT: u32 = 256 * 1024;

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

    /// Borrow the stream name for the session mux.
    pub const fn name(&self) -> &BoundedToken {
        &self.0
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
        let stream = StreamId::new(stream_number).map_err(|_| GuestExecError::StreamUnavailable)?;
        driver
            .open_named_stream(stream, GUEST_EXEC_STREAM_CREDIT, GUEST_EXEC_STREAM_CREDIT)
            .await
            .map_err(|_| GuestExecError::StreamUnavailable)?;
        Ok(Self {
            driver,
            stream,
            name,
        })
    }

    /// Borrow the opaque stream name.
    pub const fn name(&self) -> &NamedAttachmentStream {
        &self.name
    }

    /// Send one bounded logical stream message.
    pub async fn send(&self, bytes: Vec<u8>) -> Result<(), GuestExecError> {
        self.driver
            .send_named_stream(self.stream, bytes)
            .await
            .map_err(|_| GuestExecError::StreamUnavailable)
    }

    /// Receive the next typed stream event.
    pub async fn receive(&self) -> Result<StreamEvent, GuestExecError> {
        self.driver
            .receive_named_stream()
            .await
            .map_err(|_| GuestExecError::StreamUnavailable)
    }

    /// Close this named stream.
    pub async fn close(&self) -> Result<(), GuestExecError> {
        self.driver
            .close_named_stream(self.stream)
            .await
            .map_err(|_| GuestExecError::StreamUnavailable)
    }

    /// Reset this named stream after cancellation or protocol failure.
    pub async fn reset(&self) -> Result<(), GuestExecError> {
        self.driver
            .reset_named_stream(self.stream)
            .await
            .map_err(|_| GuestExecError::StreamUnavailable)
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
    }
}
