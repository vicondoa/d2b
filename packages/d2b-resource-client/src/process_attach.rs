//! Typed Process and EphemeralProcess attachment over ComponentSession.
//!
//! This module owns the caller-side shape of an attach request and the
//! lifecycle of its named stream. The session adapter still owns Noise,
//! subject resolution, session authorization, stream allocation, and all
//! transport I/O. In particular, this client never accepts an authority
//! token, a subject claim, an fd, or a path.

use core::future::Future;
use std::{
    fmt,
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};

use d2b_contracts::v3::{ResourceRef, zone_routing::ZonePath};

use crate::{
    AttemptDisposition, CallDriver, CallOptions, CancellationToken, ClientError, MethodProfile,
    ResourceClient, ServiceOwner, SessionFailure, SystemClock, TargetInput, TargetResolver,
    TransportKind, TransportSelection, WallClock, ZoneClient, ZoneServiceKind,
    ZoneSessionConnector, call::REQUEST_ID_BYTES, zone_client::ConnectedZoneSession,
};

/// The maximum logical message accepted by one attach stream.
///
/// This is the ComponentSession transport ceiling, not a second attach
/// protocol limit.
pub const MAX_PROCESS_ATTACH_MESSAGE_BYTES: usize =
    d2b_contracts::v3::component_session::MAX_LOGICAL_MESSAGE_BYTES as usize;

const STREAM_OPEN: u8 = 0;
const STREAM_CLOSING: u8 = 1;
const STREAM_CLOSED: u8 = 2;
const SHELL_SESSION_TYPE: &str = "shell-terminal.d2bus.org.ShellSession";

/// Resource classes that may be named by an attach request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessAttachKind {
    /// A caller-created or controller-created one-shot process.
    EphemeralProcess,
    /// A configured launcher Process whose command is resolved by trusted
    /// configuration rather than by the caller.
    ConfiguredLauncher,
    /// A persistent unsafe-local or guest shell session.
    ShellSession,
}

/// An attach target scoped to one exact Zone and one allowed process resource.
///
/// Constructing this value validates only the resource shape. It does not
/// authorize the caller; the authenticated ComponentSession must still admit
/// the attach operation for the session's authoritative subject.
#[derive(Clone, PartialEq, Eq)]
pub enum ProcessAttachTarget {
    /// Attach to an existing `EphemeralProcess/<name>`.
    EphemeralProcess {
        zone: ZonePath,
        resource: ResourceRef,
    },
    /// Attach to a configured `Process/<name>` launcher.
    ConfiguredLauncher {
        zone: ZonePath,
        resource: ResourceRef,
    },
    /// Attach to a persistent `ShellSession/<name>`.
    ShellSession {
        zone: ZonePath,
        resource: ResourceRef,
        execution_ref: Option<ResourceRef>,
        force: bool,
    },
}

impl ProcessAttachTarget {
    /// Construct an EphemeralProcess target.
    pub fn ephemeral_process(zone: ZonePath, resource: ResourceRef) -> Result<Self, ClientError> {
        if resource.resource_type().as_str() != "EphemeralProcess" {
            return Err(ClientError::InvalidTarget);
        }
        Ok(Self::EphemeralProcess { zone, resource })
    }

    /// Construct a configured launcher Process target.
    pub fn configured_launcher(zone: ZonePath, resource: ResourceRef) -> Result<Self, ClientError> {
        if resource.resource_type().as_str() != "Process" {
            return Err(ClientError::InvalidTarget);
        }
        Ok(Self::ConfiguredLauncher { zone, resource })
    }

    /// Construct a persistent shell-session target.  Creation carries the
    /// trusted execution reference; reconnects intentionally omit it and let
    /// the daemon resolve the already-created session.
    pub fn shell_session(
        zone: ZonePath,
        resource: ResourceRef,
        execution_ref: Option<ResourceRef>,
        force: bool,
    ) -> Result<Self, ClientError> {
        if resource.resource_type().as_str() != SHELL_SESSION_TYPE
            || execution_ref.as_ref().is_some_and(|reference| {
                !matches!(reference.resource_type().as_str(), "Host" | "Guest")
            })
        {
            return Err(ClientError::InvalidTarget);
        }
        Ok(Self::ShellSession {
            zone,
            resource,
            execution_ref,
            force,
        })
    }

    /// Interpret a resource-shaped target as an EphemeralProcess target.
    pub fn from_target(target: TargetInput) -> Result<Self, ClientError> {
        let zone = target.owner().zone().clone();
        let resource = target.resource_ref().ok_or(ClientError::InvalidTarget)?;
        Self::ephemeral_process(zone, resource)
    }

    /// Interpret a resource-shaped target as a configured launcher target.
    pub fn configured_launcher_from_target(target: TargetInput) -> Result<Self, ClientError> {
        let zone = target.owner().zone().clone();
        let resource = target.resource_ref().ok_or(ClientError::InvalidTarget)?;
        Self::configured_launcher(zone, resource)
    }

    /// Return the attach kind.
    pub const fn kind(&self) -> ProcessAttachKind {
        match self {
            Self::EphemeralProcess { .. } => ProcessAttachKind::EphemeralProcess,
            Self::ConfiguredLauncher { .. } => ProcessAttachKind::ConfiguredLauncher,
            Self::ShellSession { .. } => ProcessAttachKind::ShellSession,
        }
    }

    /// Borrow the exact Zone route.
    pub const fn zone(&self) -> &ZonePath {
        match self {
            Self::EphemeralProcess { zone, .. }
            | Self::ConfiguredLauncher { zone, .. }
            | Self::ShellSession { zone, .. } => zone,
        }
    }

    /// Borrow the exact process ResourceRef.
    pub const fn resource_ref(&self) -> &ResourceRef {
        match self {
            Self::EphemeralProcess { resource, .. }
            | Self::ConfiguredLauncher { resource, .. }
            | Self::ShellSession { resource, .. } => resource,
        }
    }

    fn target_input(&self) -> TargetInput {
        // The resource is a method target, while routing goes to the exact
        // Zone service that owns the ComponentSession attach verb.
        let owner = if self.zone() == &ZonePath::local_root() {
            ServiceOwner::ZoneLocal(self.zone().clone())
        } else {
            ServiceOwner::Zone(self.zone().clone())
        };
        TargetInput::Service {
            owner,
            service: ZoneServiceKind::Zone,
        }
    }
}

impl fmt::Debug for ProcessAttachTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessAttachTarget")
            .field("kind", &self.kind())
            .field("zone", &"<redacted>")
            .field("resource", &"<redacted>")
            .finish()
    }
}

/// Terminal geometry supplied when a TTY attach is requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    rows: u16,
    cols: u16,
}

impl TerminalSize {
    /// Construct a non-empty terminal geometry.
    pub const fn new(rows: u16, cols: u16) -> Result<Self, ClientError> {
        if rows == 0 || cols == 0 {
            return Err(ClientError::InvalidMetadata);
        }
        Ok(Self { rows, cols })
    }

    /// The terminal row count.
    pub const fn rows(self) -> u16 {
        self.rows
    }

    /// The terminal column count.
    pub const fn cols(self) -> u16 {
        self.cols
    }
}

/// Caller-controlled, non-authorizing attach presentation options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessAttachOptions {
    interactive: bool,
    tty: bool,
    initial_size: Option<TerminalSize>,
}

impl ProcessAttachOptions {
    /// Validate TTY geometry and build attach options.
    ///
    /// A TTY must carry an initial size; a non-TTY attach must not carry one.
    /// No user, uid, gid, argv, environment, cwd, or authority field is
    /// accepted here. The process's workload identity is resolved from its
    /// resource by the Zone runtime.
    pub const fn new(
        interactive: bool,
        tty: bool,
        initial_size: Option<TerminalSize>,
    ) -> Result<Self, ClientError> {
        match (tty, initial_size) {
            (true, Some(initial_size)) => Ok(Self {
                interactive,
                tty,
                initial_size: Some(initial_size),
            }),
            (true, None) | (false, Some(_)) => Err(ClientError::InvalidMetadata),
            (false, None) => Ok(Self {
                interactive,
                tty,
                initial_size: None,
            }),
        }
    }

    /// Build a non-TTY attach.
    pub const fn non_tty(interactive: bool) -> Self {
        Self {
            interactive,
            tty: false,
            initial_size: None,
        }
    }

    /// Build an interactive TTY attach.
    pub const fn interactive_tty(initial_size: TerminalSize) -> Self {
        Self {
            interactive: true,
            tty: true,
            initial_size: Some(initial_size),
        }
    }

    /// Whether stdin/input is attached.
    pub const fn interactive(self) -> bool {
        self.interactive
    }

    /// Whether the process owns a terminal stream.
    pub const fn tty(self) -> bool {
        self.tty
    }

    /// The validated initial terminal geometry.
    pub const fn initial_size(self) -> Option<TerminalSize> {
        self.initial_size
    }
}

/// The typed request handed to the authenticated ComponentSession adapter.
///
/// The client fills the request id from [`CallOptions`]. The adapter may use
/// it for correlation, but it is not an authorization credential.
#[derive(Clone, PartialEq, Eq)]
pub struct ProcessAttachOpenRequest {
    target: ProcessAttachTarget,
    options: ProcessAttachOptions,
    request_id: [u8; REQUEST_ID_BYTES],
}

impl ProcessAttachOpenRequest {
    fn new(
        target: ProcessAttachTarget,
        options: ProcessAttachOptions,
        request_id: [u8; REQUEST_ID_BYTES],
    ) -> Self {
        Self {
            target,
            options,
            request_id,
        }
    }

    /// Borrow the validated target.
    pub const fn target(&self) -> &ProcessAttachTarget {
        &self.target
    }

    /// Return the presentation options.
    pub const fn options(&self) -> ProcessAttachOptions {
        self.options
    }

    /// Borrow the opaque request correlation id.
    pub const fn request_id(&self) -> &[u8; REQUEST_ID_BYTES] {
        &self.request_id
    }
}

impl fmt::Debug for ProcessAttachOpenRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessAttachOpenRequest")
            .field("target", &self.target)
            .field("options", &self.options)
            .field("request_id", &"<redacted>")
            .finish()
    }
}

/// The session-owned named-stream operations required by process attachment.
///
/// A real implementation adapts this seam to the existing
/// `ComponentSessionDriver`: it authorizes the fixed `attach` operation,
/// opens a named stream, and delegates credit, cancellation, and close to the
/// session driver. This trait deliberately has no subject or transport
/// constructor.
pub trait NamedStreamTransport: Send + Sync {
    /// Send one bounded logical stream message.
    fn send(&self, bytes: Vec<u8>) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Deliver one terminal-size control update outside the stdin byte stream.
    fn resize(&self, _size: TerminalSize) -> impl Future<Output = Result<(), ClientError>> + Send {
        core::future::ready(Err(ClientError::InvalidMethod))
    }

    /// Receive one logical stream message.
    fn receive(&self) -> impl Future<Output = Result<Vec<u8>, ClientError>> + Send;

    /// Half-close/close the named stream.
    fn close(&self) -> impl Future<Output = Result<(), ClientError>> + Send;

    /// Reset the named stream. Adapters should use the ComponentSession reset
    /// operation rather than inventing a second cancellation transport.
    fn cancel(&self) -> impl Future<Output = Result<(), ClientError>> + Send {
        self.close()
    }
}

/// Compatibility spelling for callers that use the ComponentSession term.
pub use NamedStreamTransport as ComponentNamedStream;
/// Short spelling matching the session contract's named-stream terminology.
pub use NamedStreamTransport as NamedStream;

/// An authenticated session adapter that can open one process attach stream.
///
/// The associated stream remains owned by the session adapter. The resource
/// client never obtains a driver, fd, path, socket, subject, or admission
/// evidence.
pub trait ConnectedSession: ConnectedZoneSession {
    /// The session-owned named stream type.
    type Stream: NamedStreamTransport;

    /// Authorize and open the fixed process-attachment named stream.
    fn open_named_stream(
        &self,
        request: ProcessAttachOpenRequest,
        relative_timeout_nanos: u64,
    ) -> impl Future<Output = Result<Self::Stream, ClientError>> + Send;
}

/// Compatibility spelling for callers that name the process-specific session
/// adapter explicitly.
pub use ConnectedSession as ProcessAttachSession;

/// The caller-side owner of one process attachment stream.
pub struct ProcessAttachStream<S> {
    target: ProcessAttachTarget,
    transport: S,
    state: AtomicU8,
}

impl<S> ProcessAttachStream<S> {
    fn new(target: ProcessAttachTarget, transport: S) -> Self {
        Self {
            target,
            transport,
            state: AtomicU8::new(STREAM_OPEN),
        }
    }

    /// Borrow the validated attach target.
    pub const fn target(&self) -> &ProcessAttachTarget {
        &self.target
    }

    /// Whether the named stream has been closed or cancelled.
    pub fn is_closed(&self) -> bool {
        self.state.load(Ordering::Acquire) == STREAM_CLOSED
    }

    fn require_open(&self) -> Result<(), ClientError> {
        if self.state.load(Ordering::Acquire) == STREAM_OPEN {
            Ok(())
        } else {
            Err(ClientError::SessionLost)
        }
    }
}

impl<S> fmt::Debug for ProcessAttachStream<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessAttachStream")
            .field("target", &"<redacted>")
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl<S> ProcessAttachStream<S>
where
    S: NamedStreamTransport,
{
    /// Send one bounded logical message.
    pub async fn send(&self, bytes: &[u8]) -> Result<(), ClientError> {
        if bytes.is_empty() || bytes.len() > MAX_PROCESS_ATTACH_MESSAGE_BYTES {
            return Err(ClientError::InvalidMetadata);
        }
        self.require_open()?;
        self.transport.send(bytes.to_vec()).await
    }

    /// Deliver a terminal-size control update without interpreting stdin.
    pub async fn resize(&self, size: TerminalSize) -> Result<(), ClientError> {
        self.require_open()?;
        self.transport.resize(size).await
    }

    /// Receive one logical message from the workload-user process stream.
    pub async fn receive(&self) -> Result<Vec<u8>, ClientError> {
        self.require_open()?;
        let result = self.transport.receive().await;
        if matches!(result, Err(ClientError::SessionLost)) {
            self.state.store(STREAM_CLOSED, Ordering::Release);
        }
        result
    }

    /// Close the named stream exactly once.
    ///
    /// Dropping this value cannot perform async I/O. Call `close` when the
    /// owner stops consuming the stream so the session can release its stream
    /// credits and remote process attachment.
    pub async fn close(&self) -> Result<(), ClientError> {
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
        match self.transport.close().await {
            Ok(()) => {
                self.state.store(STREAM_CLOSED, Ordering::Release);
                Ok(())
            }
            Err(error) => {
                self.state.store(STREAM_OPEN, Ordering::Release);
                Err(error)
            }
        }
    }

    /// Cancel/reset the named stream exactly once.
    pub async fn cancel(&self) -> Result<(), ClientError> {
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
        match self.transport.cancel().await {
            Ok(()) => {
                self.state.store(STREAM_CLOSED, Ordering::Release);
                Ok(())
            }
            Err(error) => {
                self.state.store(STREAM_OPEN, Ordering::Release);
                Err(error)
            }
        }
    }
}

impl<S> Drop for ProcessAttachStream<S> {
    fn drop(&mut self) {
        self.state.store(STREAM_CLOSED, Ordering::Release);
    }
}

/// A typed async client for already-authorized Process attachment.
pub struct ProcessAttachClient<R, C, W = SystemClock> {
    zone: ZoneClient<R, C, W>,
}

impl<R, C> ProcessAttachClient<R, C, SystemClock> {
    /// Construct an attach client using the system wall clock.
    pub fn new(resolver: R, connector: C) -> Self {
        Self {
            zone: ZoneClient::new(resolver, connector),
        }
    }
}

impl<R, C, W> ProcessAttachClient<R, C, W> {
    /// Construct an attach client with an injected wall clock.
    pub fn with_clock(resolver: R, connector: C, clock: W) -> Self {
        Self {
            zone: ZoneClient::with_clock(resolver, connector, clock),
        }
    }

    /// Borrow the shared Zone client facade.
    pub const fn zone_client(&self) -> &ZoneClient<R, C, W> {
        &self.zone
    }

    /// Borrow the shared route resolver facade.
    pub const fn resource_client(&self) -> &ResourceClient<R, W> {
        self.zone.resource_client()
    }

    /// Borrow the ComponentSession connector.
    pub const fn connector(&self) -> &C {
        self.zone.connector()
    }
}

impl<R, C, W> fmt::Debug for ProcessAttachClient<R, C, W> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProcessAttachClient(<authenticated-session-adapter>)")
    }
}

impl<R, C, W> ProcessAttachClient<R, C, W>
where
    R: TargetResolver,
    C: ZoneSessionConnector,
    C::Session: ConnectedSession,
    W: WallClock,
{
    /// Attach to an existing EphemeralProcess or configured launcher Process.
    ///
    /// Route resolution and the Zone session pin are checked before the
    /// session adapter sees the request. The adapter is responsible for
    /// authoritative subject mapping and the `attach` authorization verdict;
    /// this method cannot elevate a caller or reuse an exec admission.
    pub async fn attach(
        &self,
        target: ProcessAttachTarget,
        attach_options: ProcessAttachOptions,
        call_options: CallOptions,
        selection: TransportSelection,
        cancellation: &CancellationToken,
    ) -> Result<ProcessAttachStream<<C::Session as ConnectedSession>::Stream>, ClientError> {
        if cancellation.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        let target_input = target.target_input();
        let resolved =
            self.zone
                .resource_client()
                .resolve(&target_input, ZoneServiceKind::Zone, selection)?;
        let lifetime_ms = call_options
            .metadata
            .expires_at_unix_ms()
            .checked_sub(call_options.metadata.issued_at_unix_ms())
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(ClientError::InvalidMetadata)?;
        let profile = MethodProfile::new(ZoneServiceKind::Zone, false, false, lifetime_ms)?;
        let request_id = *call_options.metadata.request_id();
        let mut driver =
            self.zone
                .resource_client()
                .prepare_call(&resolved, profile, call_options, false)?;
        let connection = match await_with_cancellation(
            self.zone
                .connect(&target_input, ZoneServiceKind::Zone, selection),
            cancellation,
        )
        .await
        {
            Ok(connection) => connection?,
            Err(ClientError::Cancelled) => return Err(ClientError::Cancelled),
            Err(error) => return Err(error),
        };
        let request = ProcessAttachOpenRequest::new(target.clone(), attach_options, request_id);

        loop {
            let attempt = match driver.begin_attempt(cancellation) {
                Ok(attempt) => attempt,
                Err(ClientError::Cancelled) => {
                    let _ = connection.session().cancel(request_id).await;
                    return Err(ClientError::Cancelled);
                }
                Err(error) => return Err(error),
            };
            let open = connection
                .session()
                .open_named_stream(request.clone(), attempt.relative_timeout_nanos());
            let result = match await_with_cancellation(open, cancellation).await {
                Ok(result) => result,
                Err(ClientError::Cancelled) => {
                    let _ = connection.session().cancel(request_id).await;
                    return Err(ClientError::Cancelled);
                }
                Err(error) => return Err(error),
            };

            match result {
                Ok(stream) => {
                    let stream = ProcessAttachStream::new(target.clone(), stream);
                    if cancellation.is_cancelled() {
                        let _ = stream.cancel().await;
                        let _ = connection.session().cancel(request_id).await;
                        return Err(ClientError::Cancelled);
                    }
                    return Ok(stream);
                }
                Err(error) => match classify_attach_error(&driver, error) {
                    AttemptDisposition::RetryNow => continue,
                    AttemptDisposition::RetryAfterMs(delay) => {
                        let sleep = tokio::time::sleep(Duration::from_millis(u64::from(delay)));
                        if let Err(ClientError::Cancelled) =
                            await_with_cancellation(sleep, cancellation).await
                        {
                            let _ = connection.session().cancel(request_id).await;
                            return Err(ClientError::Cancelled);
                        }
                    }
                    AttemptDisposition::Fail(error) => return Err(error),
                },
            }
        }
    }

    /// Attach using the local Unix carriage.
    pub async fn attach_local(
        &self,
        target: ProcessAttachTarget,
        attach_options: ProcessAttachOptions,
        call_options: CallOptions,
        cancellation: &CancellationToken,
    ) -> Result<ProcessAttachStream<<C::Session as ConnectedSession>::Stream>, ClientError> {
        self.attach(
            target,
            attach_options,
            call_options,
            TransportSelection::exact(TransportKind::LocalUnix),
            cancellation,
        )
        .await
    }

    /// Establish and close one attachment without exposing the stream handle.
    ///
    /// This is useful for operator surfaces whose current command contract
    /// reports establishment rather than proxying byte I/O. The session
    /// adapter still performs the authorized named-stream open and close.
    pub async fn attach_and_close(
        &self,
        target: ProcessAttachTarget,
        attach_options: ProcessAttachOptions,
        call_options: CallOptions,
        selection: TransportSelection,
        cancellation: &CancellationToken,
    ) -> Result<(), ClientError> {
        let stream = self
            .attach(
                target,
                attach_options,
                call_options,
                selection,
                cancellation,
            )
            .await?;
        stream.close().await
    }
}

async fn await_with_cancellation<F, T>(
    future: F,
    cancellation: &CancellationToken,
) -> Result<T, ClientError>
where
    F: Future<Output = T> + Send,
{
    let mut future = Box::pin(future);
    let mut cancelled = Box::pin(cancellation.cancelled());
    core::future::poll_fn(move |context| {
        if let core::task::Poll::Ready(value) = future.as_mut().poll(context) {
            return core::task::Poll::Ready(Ok(value));
        }
        if let core::task::Poll::Ready(()) = cancelled.as_mut().poll(context) {
            return core::task::Poll::Ready(Err(ClientError::Cancelled));
        }
        core::task::Poll::Pending
    })
    .await
}

fn classify_attach_error<W: WallClock>(
    driver: &CallDriver<W>,
    error: ClientError,
) -> AttemptDisposition {
    match error {
        ClientError::SessionLost => driver.record_session_failure(SessionFailure::Disconnected),
        ClientError::TransportFailed => driver.record_session_failure(SessionFailure::Retryable),
        ClientError::DeadlineExpired => driver.record_session_failure(SessionFailure::Deadline),
        ClientError::Cancelled => driver.record_session_failure(SessionFailure::Cancelled),
        ClientError::ContractViolation => driver.record_session_failure(SessionFailure::Protocol),
        ClientError::Remote { kind, retry } => driver.record_remote_verdict(kind, retry),
        other => AttemptDisposition::Fail(other),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use d2b_contracts::v3::{
        CanonicalJsonObject, ResourceErrorKind, RetryClass, zone_routing::ZoneLabelId,
    };

    use super::*;
    use crate::{
        MetadataInput, RetryPolicy, RouteRecord, RouteTable, ServiceOwner, ZonePeerIdentity,
        ZoneSessionPin,
    };

    const ISSUED: u64 = 10_000;

    #[derive(Debug)]
    struct FixedClock;

    impl WallClock for FixedClock {
        fn now_unix_ms(&self) -> u64 {
            ISSUED
        }
    }

    fn zone(name: &str) -> ZonePath {
        ZonePath::new(vec![ZoneLabelId::parse(name).unwrap()]).unwrap()
    }

    fn process_ref(kind: &str, name: &str) -> ResourceRef {
        ResourceRef::parse(&format!("{kind}/{name}")).unwrap()
    }

    fn target() -> ProcessAttachTarget {
        ProcessAttachTarget::ephemeral_process(
            zone("dev"),
            process_ref("EphemeralProcess", "command"),
        )
        .unwrap()
    }

    fn call_options(attempts: u8) -> CallOptions {
        CallOptions {
            metadata: MetadataInput::new([7; REQUEST_ID_BYTES], ISSUED, ISSUED + 30_000).unwrap(),
            retry: RetryPolicy::new(attempts).unwrap(),
        }
    }

    fn pin(service: ZoneServiceKind) -> ZoneSessionPin {
        ZoneSessionPin::new(
            ZonePeerIdentity::from_observed_static_key(zone("dev"), [3; 32]),
            service,
            TransportKind::LocalUnix,
            1,
            [4; 32],
        )
        .unwrap()
    }

    #[derive(Default)]
    struct FakeStream {
        sent: Mutex<Vec<Vec<u8>>>,
        received: Mutex<VecDeque<Vec<u8>>>,
        closes: AtomicUsize,
        cancels: AtomicUsize,
    }

    impl NamedStreamTransport for Arc<FakeStream> {
        fn send(&self, bytes: Vec<u8>) -> impl Future<Output = Result<(), ClientError>> + Send {
            self.sent.lock().unwrap().push(bytes);
            core::future::ready(Ok(()))
        }

        fn receive(&self) -> impl Future<Output = Result<Vec<u8>, ClientError>> + Send {
            let result = self
                .received
                .lock()
                .unwrap()
                .pop_front()
                .ok_or(ClientError::SessionLost);
            core::future::ready(result)
        }

        fn close(&self) -> impl Future<Output = Result<(), ClientError>> + Send {
            self.closes.fetch_add(1, Ordering::AcqRel);
            core::future::ready(Ok(()))
        }

        fn cancel(&self) -> impl Future<Output = Result<(), ClientError>> + Send {
            self.cancels.fetch_add(1, Ordering::AcqRel);
            core::future::ready(Ok(()))
        }
    }

    struct FakeSession {
        outcomes: Mutex<VecDeque<Result<Arc<FakeStream>, ClientError>>>,
        opens: AtomicUsize,
        gate: Option<Arc<tokio::sync::Notify>>,
        seen: Mutex<Vec<ProcessAttachOpenRequest>>,
    }

    impl FakeSession {
        fn new(
            outcomes: Vec<Result<Arc<FakeStream>, ClientError>>,
            gate: Option<Arc<tokio::sync::Notify>>,
        ) -> Self {
            Self {
                outcomes: Mutex::new(outcomes.into()),
                opens: AtomicUsize::new(0),
                gate,
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl ConnectedZoneSession for FakeSession {
        fn call(
            &self,
            _verb: crate::ResourceVerb,
            _target: Option<ResourceRef>,
            _payload: CanonicalJsonObject,
        ) -> impl Future<Output = Result<CanonicalJsonObject, ClientError>> + Send {
            core::future::ready(Err(ClientError::ContractViolation))
        }
    }

    impl ConnectedSession for FakeSession {
        type Stream = Arc<FakeStream>;

        fn open_named_stream(
            &self,
            request: ProcessAttachOpenRequest,
            _relative_timeout_nanos: u64,
        ) -> impl Future<Output = Result<Self::Stream, ClientError>> + Send {
            self.opens.fetch_add(1, Ordering::AcqRel);
            self.seen.lock().unwrap().push(request);
            let gate = self.gate.clone();
            let result = self
                .outcomes
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(ClientError::SessionLost));
            async move {
                if let Some(gate) = gate {
                    gate.notified().await;
                }
                result
            }
        }
    }

    struct FakeConnector {
        session: Arc<FakeSession>,
        pin: ZoneSessionPin,
        requested_services: Arc<Mutex<Vec<ZoneServiceKind>>>,
    }

    impl ZoneSessionConnector for FakeConnector {
        type Session = Arc<FakeSession>;

        fn connect(
            &self,
            _target: &crate::ResolvedTarget,
            service: ZoneServiceKind,
        ) -> impl Future<Output = Result<(Self::Session, ZoneSessionPin), ClientError>> + Send
        {
            self.requested_services.lock().unwrap().push(service);
            core::future::ready(Ok((Arc::clone(&self.session), self.pin.clone())))
        }
    }

    impl ConnectedZoneSession for Arc<FakeSession> {
        fn call(
            &self,
            verb: crate::ResourceVerb,
            target: Option<ResourceRef>,
            payload: CanonicalJsonObject,
        ) -> impl Future<Output = Result<CanonicalJsonObject, ClientError>> + Send {
            (**self).call(verb, target, payload)
        }
    }

    impl ConnectedSession for Arc<FakeSession> {
        type Stream = Arc<FakeStream>;

        fn open_named_stream(
            &self,
            request: ProcessAttachOpenRequest,
            relative_timeout_nanos: u64,
        ) -> impl Future<Output = Result<Self::Stream, ClientError>> + Send {
            (**self).open_named_stream(request, relative_timeout_nanos)
        }
    }

    fn client(
        connector: FakeConnector,
    ) -> ProcessAttachClient<RouteTable, FakeConnector, FixedClock> {
        ProcessAttachClient::with_clock(
            RouteTable::new(vec![RouteRecord::new(
                ServiceOwner::Zone(zone("dev")),
                TransportKind::LocalUnix,
            )]),
            connector,
            FixedClock,
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn authorized_attach_opens_a_named_stream_and_closes_once() {
        let stream = Arc::new(FakeStream {
            received: Mutex::new(VecDeque::from([b"output".to_vec()])),
            ..Default::default()
        });
        let session = Arc::new(FakeSession::new(vec![Ok(Arc::clone(&stream))], None));
        let requested_services = Arc::new(Mutex::new(Vec::new()));
        let client = client(FakeConnector {
            session,
            pin: pin(ZoneServiceKind::Zone),
            requested_services: Arc::clone(&requested_services),
        });
        let cancellation = CancellationToken::default();
        let attached = client
            .attach(
                target(),
                ProcessAttachOptions::non_tty(true),
                call_options(2),
                TransportSelection::exact(TransportKind::LocalUnix),
                &cancellation,
            )
            .await
            .unwrap();
        assert_eq!(
            requested_services.lock().unwrap().as_slice(),
            &[ZoneServiceKind::Zone]
        );
        attached.send(b"input").await.unwrap();
        assert_eq!(attached.receive().await.unwrap(), b"output");
        attached.close().await.unwrap();
        attached.close().await.unwrap();
        assert_eq!(stream.closes.load(Ordering::Acquire), 1);
        assert!(attached.is_closed());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancellation_stops_a_pending_open_and_does_not_leak_a_stream() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let session = Arc::new(FakeSession::new(
            vec![Ok(Arc::new(FakeStream::default()))],
            Some(gate),
        ));
        let requested_services = Arc::new(Mutex::new(Vec::new()));
        let client = client(FakeConnector {
            session,
            pin: pin(ZoneServiceKind::Zone),
            requested_services,
        });
        let cancellation = CancellationToken::default();
        let cancel = cancellation.clone();
        let task = tokio::spawn(async move {
            client
                .attach(
                    target(),
                    ProcessAttachOptions::non_tty(false),
                    call_options(2),
                    TransportSelection::exact(TransportKind::LocalUnix),
                    &cancellation,
                )
                .await
        });
        tokio::task::yield_now().await;
        cancel.cancel();
        assert_eq!(task.await.unwrap().unwrap_err(), ClientError::Cancelled);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn retry_classification_retries_transport_but_not_authorization() {
        let first = Arc::new(FakeStream::default());
        let session = Arc::new(FakeSession::new(
            vec![Err(ClientError::TransportFailed), Ok(first)],
            None,
        ));
        let requested_services = Arc::new(Mutex::new(Vec::new()));
        let first_client = client(FakeConnector {
            session: Arc::clone(&session),
            pin: pin(ZoneServiceKind::Zone),
            requested_services,
        });
        let cancellation = CancellationToken::default();
        let attached = first_client
            .attach(
                target(),
                ProcessAttachOptions::non_tty(false),
                call_options(2),
                TransportSelection::exact(TransportKind::LocalUnix),
                &cancellation,
            )
            .await
            .unwrap();
        assert_eq!(session.opens.load(Ordering::Acquire), 2);
        attached.close().await.unwrap();

        let denied_session = Arc::new(FakeSession::new(
            vec![Err(ClientError::Remote {
                kind: ResourceErrorKind::AuthorizationDenied,
                retry: RetryClass::Never,
            })],
            None,
        ));
        let denied_client = client(FakeConnector {
            session: denied_session.clone(),
            pin: pin(ZoneServiceKind::Zone),
            requested_services: Arc::new(Mutex::new(Vec::new())),
        });
        let error = denied_client
            .attach(
                target(),
                ProcessAttachOptions::non_tty(false),
                call_options(8),
                TransportSelection::exact(TransportKind::LocalUnix),
                &CancellationToken::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error,
            ClientError::Remote {
                kind: ResourceErrorKind::AuthorizationDenied,
                retry: RetryClass::Never,
            }
        );
        assert_eq!(denied_session.opens.load(Ordering::Acquire), 1);
    }

    #[test]
    fn wrong_resource_type_and_wrong_zone_fail_before_open() {
        assert_eq!(
            ProcessAttachTarget::ephemeral_process(
                zone("dev"),
                process_ref("Process", "wrong-type")
            )
            .unwrap_err(),
            ClientError::InvalidTarget
        );
        assert_eq!(
            ProcessAttachTarget::configured_launcher(
                zone("dev"),
                process_ref("EphemeralProcess", "wrong-type")
            )
            .unwrap_err(),
            ClientError::InvalidTarget
        );

        let session = Arc::new(FakeSession::new(Vec::new(), None));
        let client = client(FakeConnector {
            session,
            pin: pin(ZoneServiceKind::Zone),
            requested_services: Arc::new(Mutex::new(Vec::new())),
        });
        let wrong_zone = ProcessAttachTarget::ephemeral_process(
            zone("other"),
            process_ref("EphemeralProcess", "command"),
        )
        .unwrap();
        let result = futures_block_on(client.attach(
            wrong_zone,
            ProcessAttachOptions::non_tty(false),
            call_options(1),
            TransportSelection::exact(TransportKind::LocalUnix),
            &CancellationToken::default(),
        ));
        assert_eq!(result.unwrap_err(), ClientError::RouteUnavailable);
    }

    #[test]
    fn shell_session_target_requires_qualified_type_and_host_or_guest_execution() {
        let shell = process_ref(SHELL_SESSION_TYPE, "primary");
        let host = process_ref("Host", "tools");
        let target =
            ProcessAttachTarget::shell_session(zone("dev"), shell.clone(), Some(host), false)
                .expect("qualified shell target");
        assert_eq!(target.kind(), ProcessAttachKind::ShellSession);
        assert_eq!(target.resource_ref(), &shell);

        assert_eq!(
            ProcessAttachTarget::shell_session(
                zone("dev"),
                process_ref("Process", "primary"),
                None,
                false,
            ),
            Err(ClientError::InvalidTarget)
        );
        assert_eq!(
            ProcessAttachTarget::shell_session(
                zone("dev"),
                process_ref(SHELL_SESSION_TYPE, "primary"),
                Some(process_ref("Process", "wrong")),
                false,
            ),
            Err(ClientError::InvalidTarget)
        );
    }

    #[test]
    fn local_root_attach_uses_the_local_zone_owner_route() {
        let local_zone = ZonePath::local_root();
        let session = Arc::new(FakeSession::new(
            vec![Ok(Arc::new(FakeStream::default()))],
            None,
        ));
        let connector = FakeConnector {
            session,
            pin: ZoneSessionPin::new(
                ZonePeerIdentity::from_observed_static_key(local_zone.clone(), [3; 32]),
                ZoneServiceKind::Zone,
                TransportKind::LocalUnix,
                1,
                [4; 32],
            )
            .unwrap(),
            requested_services: Arc::new(Mutex::new(Vec::new())),
        };
        let client = ProcessAttachClient::with_clock(
            RouteTable::new(vec![RouteRecord::new(
                ServiceOwner::ZoneLocal(local_zone.clone()),
                TransportKind::LocalUnix,
            )]),
            connector,
            FixedClock,
        );
        let target = ProcessAttachTarget::ephemeral_process(
            local_zone,
            process_ref("EphemeralProcess", "command"),
        )
        .unwrap();
        let result = futures_block_on(client.attach(
            target,
            ProcessAttachOptions::non_tty(false),
            call_options(1),
            TransportSelection::exact(TransportKind::LocalUnix),
            &CancellationToken::default(),
        ));
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reused_session_evidence_is_denied_by_the_zone_pin() {
        let session = Arc::new(FakeSession::new(
            vec![Ok(Arc::new(FakeStream::default()))],
            None,
        ));
        let client = client(FakeConnector {
            session: Arc::clone(&session),
            pin: pin(ZoneServiceKind::Resource),
            requested_services: Arc::new(Mutex::new(Vec::new())),
        });
        let error = client
            .attach(
                target(),
                ProcessAttachOptions::non_tty(false),
                call_options(1),
                TransportSelection::exact(TransportKind::LocalUnix),
                &CancellationToken::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(error, ClientError::TransportPolicyMismatch);
        assert_eq!(session.opens.load(Ordering::Acquire), 0);
    }

    #[test]
    fn attach_inputs_and_diagnostics_are_bounded_and_redacted() {
        assert_eq!(
            TerminalSize::new(0, 80).unwrap_err(),
            ClientError::InvalidMetadata
        );
        assert_eq!(
            ProcessAttachOptions::new(true, true, None).unwrap_err(),
            ClientError::InvalidMetadata
        );
        assert_eq!(
            ProcessAttachOptions::new(false, false, TerminalSize::new(24, 80).ok()).unwrap_err(),
            ClientError::InvalidMetadata
        );
        let marker = format!("marker{:x}", std::process::id());
        let target = ProcessAttachTarget::ephemeral_process(
            zone(&marker),
            process_ref("EphemeralProcess", &marker),
        )
        .unwrap();
        let rendered = format!("{target:?}");
        assert!(!rendered.contains(&marker), "{rendered}");
        assert_eq!(
            format!(
                "{:?}",
                ProcessAttachOpenRequest::new(
                    target,
                    ProcessAttachOptions::non_tty(false),
                    [0xAB; REQUEST_ID_BYTES],
                )
            ),
            "ProcessAttachOpenRequest { target: ProcessAttachTarget { kind: EphemeralProcess, zone: \"<redacted>\", resource: \"<redacted>\" }, options: ProcessAttachOptions { interactive: false, tty: false, initial_size: None }, request_id: \"<redacted>\" }"
        );
    }

    fn futures_block_on<F: Future>(future: F) -> F::Output {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(future)
    }
}
