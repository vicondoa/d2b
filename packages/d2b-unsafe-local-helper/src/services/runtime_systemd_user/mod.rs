//! Same-UID systemd user runtime behind the frozen ComponentSession service.
//!
//! Endpoint establishment, authentication, deadline intersection, and
//! attachment validation happen before this adapter is called. The adapter
//! nevertheless rechecks the authenticated owner and request binding before it
//! permits a user-manager mutation. There is intentionally no helper-protocol,
//! compositor, shell-command, or host-shell compatibility path.

use std::collections::VecDeque;
use std::fmt;

pub const SERVICE_PACKAGE: &str = "d2b.runtime.systemd-user.v2";
pub const ENDPOINT_PURPOSE: &str = "runtime-systemd-user";
pub const ENDPOINT_ROLE: &str = "runtime-systemd-user-agent";
pub const SERVICE_NAME: &str = "RuntimeSystemdUserService";

pub const MAX_PRIVATE_ARGV_ENTRIES: usize = 256;
pub const MAX_PRIVATE_ARGV_BYTES: usize = 256 * 1024;
pub const MAX_DIAGNOSTIC_EVENTS: usize = 64;
pub const TERMINAL_ATTACHMENT_INDEX: u32 = 0;
pub const MAX_REQUEST_LIFETIME_MS: u64 = 15 * 60 * 1_000;
pub const MAX_FUTURE_CLOCK_SKEW_MS: u64 = 30 * 1_000;
const MAX_ID_BYTES: usize = 64;

pub trait EstablishedComponentSession {
    fn service_package(&self) -> &str;
    fn endpoint_purpose(&self) -> &str;
    fn endpoint_role(&self) -> &str;
    fn is_authenticated(&self) -> bool;
    fn uses_pre_authorized_transport(&self) -> bool;
    fn authenticated_uid(&self) -> u32;
    fn process_uid(&self) -> u32;
    fn session_generation(&self) -> u64;
    fn realm_id(&self) -> &str;
    fn workload_id(&self) -> &str;
}

#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeOwner {
    uid: u32,
    session_generation: u64,
    realm_id: String,
    workload_id: String,
}

impl RuntimeOwner {
    pub fn admit(
        established: &impl EstablishedComponentSession,
    ) -> Result<Self, RuntimeServiceError> {
        let uid = established.authenticated_uid();
        if !established.is_authenticated() {
            return Err(RuntimeServiceError::Unauthenticated);
        }
        if !established.uses_pre_authorized_transport()
            || established.service_package() != SERVICE_PACKAGE
            || established.endpoint_purpose() != ENDPOINT_PURPOSE
            || established.endpoint_role() != ENDPOINT_ROLE
        {
            return Err(RuntimeServiceError::ContractMismatch);
        }
        if uid == 0
            || uid != established.process_uid()
            || uid != nix::unistd::getuid().as_raw()
            || established.session_generation() == 0
            || !valid_id(established.realm_id())
            || !valid_id(established.workload_id())
        {
            return Err(RuntimeServiceError::OwnerMismatch);
        }
        Ok(Self {
            uid,
            session_generation: established.session_generation(),
            realm_id: established.realm_id().to_owned(),
            workload_id: established.workload_id().to_owned(),
        })
    }

    pub const fn uid(&self) -> u32 {
        self.uid
    }

    pub const fn session_generation(&self) -> u64 {
        self.session_generation
    }
}

impl fmt::Debug for RuntimeOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeOwner")
            .field("uid", &"<redacted>")
            .field("session_generation", &"<redacted>")
            .field("realm_id", &"<redacted>")
            .field("workload_id", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedProcess {
    resource_id: String,
    request_digest: [u8; 32],
    argv: Vec<String>,
    graphical: bool,
}

impl ResolvedProcess {
    pub fn new(
        resource_id: String,
        request_digest: [u8; 32],
        argv: Vec<String>,
        graphical: bool,
    ) -> Result<Self, RuntimeServiceError> {
        let encoded_bytes = argv.iter().try_fold(0usize, |total, value| {
            total
                .checked_add(value.len())
                .and_then(|size| size.checked_add(1))
        });
        if !valid_id(&resource_id)
            || argv.is_empty()
            || argv.len() > MAX_PRIVATE_ARGV_ENTRIES
            || encoded_bytes.is_none_or(|size| size > MAX_PRIVATE_ARGV_BYTES)
            || argv.iter().any(|value| value.contains('\0'))
        {
            return Err(RuntimeServiceError::InvalidResolvedProcess);
        }
        Ok(Self {
            resource_id,
            request_digest,
            argv,
            graphical,
        })
    }

    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    pub const fn graphical(&self) -> bool {
        self.graphical
    }
}

impl fmt::Debug for ResolvedProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedProcess")
            .field("resource_id", &"<redacted>")
            .field("request_digest", &"<redacted>")
            .field("argv_count", &self.argv.len())
            .field("graphical", &self.graphical)
            .finish()
    }
}

pub trait ConfiguredProcessResolver {
    fn resolve(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
        request_digest: &[u8; 32],
    ) -> Result<ResolvedProcess, RuntimeServiceError>;
}

#[derive(Clone, PartialEq, Eq)]
pub struct WaylandDisplayLease {
    handle: String,
}

impl WaylandDisplayLease {
    pub fn new(handle: String) -> Result<Self, RuntimeServiceError> {
        if !valid_id(&handle) {
            return Err(RuntimeServiceError::WaylandUnavailable);
        }
        Ok(Self { handle })
    }

    pub fn handle(&self) -> &str {
        &self.handle
    }
}

impl fmt::Debug for WaylandDisplayLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WaylandDisplayLease(<redacted>)")
    }
}

pub trait WaylandControlPort {
    fn open_display(
        &mut self,
        owner: &RuntimeOwner,
        process: &ResolvedProcess,
        operation_id: &str,
    ) -> Result<WaylandDisplayLease, RuntimeServiceError>;

    fn close_display(&mut self, lease: WaylandDisplayLease);
}

#[derive(Clone, PartialEq, Eq)]
pub struct AuthenticatedTerminalAttachment {
    pub index: u32,
    pub owner_uid: u32,
    pub session_generation: u64,
    pub request_id: [u8; 16],
    pub connected_stream: bool,
    pub cloexec: bool,
}

impl fmt::Debug for AuthenticatedTerminalAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedTerminalAttachment")
            .field("index", &self.index)
            .field("owner_uid", &"<redacted>")
            .field("session_generation", &"<redacted>")
            .field("request_id", &"<redacted>")
            .field("connected_stream", &self.connected_stream)
            .field("cloexec", &self.cloexec)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeProcessState {
    Present,
    Running,
    Stopped,
    Degraded,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeResource {
    pub handle: String,
    pub result_digest: [u8; 32],
    pub state: RuntimeProcessState,
}

impl fmt::Debug for RuntimeResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeResource")
            .field("handle", &"<redacted>")
            .field("result_digest", &"<redacted>")
            .field("state", &self.state)
            .finish()
    }
}

pub trait SystemdUserRuntimePort {
    fn ensure_scope(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
        operation_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError>;

    fn start_process(
        &mut self,
        owner: &RuntimeOwner,
        operation_id: &str,
        process: &ResolvedProcess,
        display: Option<&WaylandDisplayLease>,
    ) -> Result<RuntimeResource, RuntimeServiceError>;

    fn inspect_process(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError>;

    fn adopt_process(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
        operation_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError>;

    fn stop_process(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
        operation_id: &str,
    ) -> Result<RuntimeResource, RuntimeServiceError>;

    fn open_terminal(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
        stream_id: &str,
        attachment: &AuthenticatedTerminalAttachment,
    ) -> Result<RuntimeResource, RuntimeServiceError>;

    fn cancel(&mut self, owner: &RuntimeOwner, request_id: [u8; 16]) -> CancelResult;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelResult {
    CancelledBeforeDispatch,
    CancellationSignalled,
    AlreadyTerminal,
    UnknownRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMethod {
    EnsureScope,
    StartProcess,
    InspectProcess,
    AdoptProcess,
    StopProcess,
    OpenTerminal,
}

impl RuntimeMethod {
    pub const fn name(self) -> &'static str {
        match self {
            Self::EnsureScope => "EnsureScope",
            Self::StartProcess => "StartProcess",
            Self::InspectProcess => "InspectProcess",
            Self::AdoptProcess => "AdoptProcess",
            Self::StopProcess => "StopProcess",
            Self::OpenTerminal => "OpenTerminal",
        }
    }

    pub const fn mutating(self) -> bool {
        !matches!(self, Self::InspectProcess)
    }

    const fn expected_state(self) -> DesiredState {
        match self {
            Self::EnsureScope | Self::AdoptProcess => DesiredState::Present,
            Self::StartProcess => DesiredState::Running,
            Self::InspectProcess => DesiredState::Unspecified,
            Self::StopProcess => DesiredState::Stopped,
            Self::OpenTerminal => DesiredState::Attached,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesiredState {
    Unspecified,
    Present,
    Running,
    Stopped,
    Attached,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeRequest {
    pub request_id: [u8; 16],
    pub idempotency_key: Option<[u8; 32]>,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub session_generation: u64,
    pub realm_id: String,
    pub workload_id: String,
    pub resource_id: String,
    pub operation_id: String,
    pub request_digest: Option<[u8; 32]>,
    pub stream_id: String,
    pub attachment_indexes: Vec<u32>,
    pub desired_state: DesiredState,
}

impl fmt::Debug for RuntimeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeRequest")
            .field("desired_state", &self.desired_state)
            .field("has_idempotency_key", &self.idempotency_key.is_some())
            .field("has_request_digest", &self.request_digest.is_some())
            .field("attachment_count", &self.attachment_indexes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOutcome {
    Succeeded,
    Degraded,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeResponse {
    pub outcome: RuntimeOutcome,
    pub operation_id: String,
    pub resource_handle: String,
    pub stream_id: String,
    pub result_digest: [u8; 32],
    pub attachment_indexes: Vec<u32>,
}

impl fmt::Debug for RuntimeResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeResponse")
            .field("outcome", &self.outcome)
            .field("operation_id", &"<redacted>")
            .field("resource_handle", &"<redacted>")
            .field("stream_id", &"<redacted>")
            .field("attachment_count", &self.attachment_indexes.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct CancelRequest {
    pub session_generation: u64,
    pub request_id: [u8; 16],
}

impl fmt::Debug for CancelRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CancelRequest(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelResponse {
    pub outcome: CancelResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticOutcome {
    Success,
    Denied,
    Unavailable,
    Degraded,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticOperation {
    EnsureScope,
    StartProcess,
    InspectProcess,
    AdoptProcess,
    StopProcess,
    OpenTerminal,
    Cancel,
}

impl From<RuntimeMethod> for DiagnosticOperation {
    fn from(method: RuntimeMethod) -> Self {
        match method {
            RuntimeMethod::EnsureScope => Self::EnsureScope,
            RuntimeMethod::StartProcess => Self::StartProcess,
            RuntimeMethod::InspectProcess => Self::InspectProcess,
            RuntimeMethod::AdoptProcess => Self::AdoptProcess,
            RuntimeMethod::StopProcess => Self::StopProcess,
            RuntimeMethod::OpenTerminal => Self::OpenTerminal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticEvent {
    pub operation: DiagnosticOperation,
    pub outcome: DiagnosticOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDiagnostics {
    events: VecDeque<DiagnosticEvent>,
}

impl RuntimeDiagnostics {
    fn record(&mut self, event: DiagnosticEvent) {
        if self.events.len() == MAX_DIAGNOSTIC_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    pub fn events(&self) -> &VecDeque<DiagnosticEvent> {
        &self.events
    }
}

pub struct RuntimeSystemdUserService<R, W, B> {
    owner: RuntimeOwner,
    resolver: R,
    wayland: W,
    backend: B,
    diagnostics: RuntimeDiagnostics,
}

impl<R, W, B> fmt::Debug for RuntimeSystemdUserService<R, W, B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeSystemdUserService")
            .field("owner", &"<redacted>")
            .field("diagnostic_events", &self.diagnostics.events.len())
            .finish_non_exhaustive()
    }
}

impl<R, W, B> RuntimeSystemdUserService<R, W, B>
where
    R: ConfiguredProcessResolver,
    W: WaylandControlPort,
    B: SystemdUserRuntimePort,
{
    pub fn new(owner: RuntimeOwner, resolver: R, wayland: W, backend: B) -> Self {
        Self {
            owner,
            resolver,
            wayland,
            backend,
            diagnostics: RuntimeDiagnostics {
                events: VecDeque::with_capacity(MAX_DIAGNOSTIC_EVENTS),
            },
        }
    }

    pub fn diagnostics(&self) -> &RuntimeDiagnostics {
        &self.diagnostics
    }

    pub fn dispatch(
        &mut self,
        method: RuntimeMethod,
        request: &RuntimeRequest,
        attachments: &[AuthenticatedTerminalAttachment],
        now_unix_ms: u64,
    ) -> Result<RuntimeResponse, RuntimeServiceError> {
        let result = self.dispatch_inner(method, request, attachments, now_unix_ms);
        let outcome = match &result {
            Ok(response) if response.outcome == RuntimeOutcome::Degraded => {
                DiagnosticOutcome::Degraded
            }
            Ok(_) => DiagnosticOutcome::Success,
            Err(RuntimeServiceError::Unavailable | RuntimeServiceError::WaylandUnavailable) => {
                DiagnosticOutcome::Unavailable
            }
            Err(_) => DiagnosticOutcome::Denied,
        };
        self.diagnostics.record(DiagnosticEvent {
            operation: method.into(),
            outcome,
        });
        result
    }

    fn dispatch_inner(
        &mut self,
        method: RuntimeMethod,
        request: &RuntimeRequest,
        attachments: &[AuthenticatedTerminalAttachment],
        now_unix_ms: u64,
    ) -> Result<RuntimeResponse, RuntimeServiceError> {
        self.admit_request(method, request, attachments, now_unix_ms)?;
        let resource = match method {
            RuntimeMethod::EnsureScope => self.backend.ensure_scope(
                &self.owner,
                &request.resource_id,
                &request.operation_id,
            )?,
            RuntimeMethod::StartProcess => {
                let digest = request
                    .request_digest
                    .ok_or(RuntimeServiceError::InvalidRequest)?;
                let process = self
                    .resolver
                    .resolve(&self.owner, &request.resource_id, &digest)?;
                if process.resource_id != request.resource_id || process.request_digest != digest {
                    return Err(RuntimeServiceError::ResolverMismatch);
                }
                let display = process
                    .graphical()
                    .then(|| {
                        self.wayland
                            .open_display(&self.owner, &process, &request.operation_id)
                    })
                    .transpose()?;
                let started = self.backend.start_process(
                    &self.owner,
                    &request.operation_id,
                    &process,
                    display.as_ref(),
                );
                if started.is_err()
                    && let Some(display) = display
                {
                    self.wayland.close_display(display);
                }
                started?
            }
            RuntimeMethod::InspectProcess => self
                .backend
                .inspect_process(&self.owner, &request.resource_id)?,
            RuntimeMethod::AdoptProcess => self.backend.adopt_process(
                &self.owner,
                &request.resource_id,
                &request.operation_id,
            )?,
            RuntimeMethod::StopProcess => self.backend.stop_process(
                &self.owner,
                &request.resource_id,
                &request.operation_id,
            )?,
            RuntimeMethod::OpenTerminal => self.backend.open_terminal(
                &self.owner,
                &request.resource_id,
                &request.stream_id,
                &attachments[0],
            )?,
        };
        response_for(request, resource)
    }

    fn admit_request(
        &self,
        method: RuntimeMethod,
        request: &RuntimeRequest,
        attachments: &[AuthenticatedTerminalAttachment],
        now_unix_ms: u64,
    ) -> Result<(), RuntimeServiceError> {
        if request.session_generation != self.owner.session_generation {
            return Err(RuntimeServiceError::GenerationMismatch);
        }
        if request.issued_at_unix_ms > request.expires_at_unix_ms
            || request.issued_at_unix_ms > now_unix_ms.saturating_add(MAX_FUTURE_CLOCK_SKEW_MS)
            || request.expires_at_unix_ms <= now_unix_ms
            || request
                .expires_at_unix_ms
                .saturating_sub(request.issued_at_unix_ms)
                > MAX_REQUEST_LIFETIME_MS
        {
            return Err(RuntimeServiceError::DeadlineExpired);
        }
        if request.realm_id != self.owner.realm_id
            || request.workload_id != self.owner.workload_id
            || !valid_id(&request.resource_id)
            || (!request.operation_id.is_empty() && !valid_id(&request.operation_id))
            || (!request.stream_id.is_empty() && !valid_id(&request.stream_id))
            || request.request_id == [0; 16]
        {
            return Err(RuntimeServiceError::OwnerMismatch);
        }
        if method.mutating()
            && (request.operation_id.is_empty()
                || request.request_digest.is_none()
                || request.idempotency_key.is_none())
        {
            return Err(RuntimeServiceError::InvalidRequest);
        }

        if request.desired_state != method.expected_state() {
            return Err(RuntimeServiceError::InvalidRequest);
        }

        match method {
            RuntimeMethod::OpenTerminal => {
                let [attachment] = attachments else {
                    return Err(RuntimeServiceError::AttachmentMismatch);
                };
                if request.stream_id.is_empty()
                    || request.attachment_indexes != [TERMINAL_ATTACHMENT_INDEX]
                    || attachment.index != TERMINAL_ATTACHMENT_INDEX
                    || attachment.owner_uid != self.owner.uid
                    || attachment.session_generation != self.owner.session_generation
                    || attachment.request_id != request.request_id
                    || !attachment.connected_stream
                    || !attachment.cloexec
                {
                    return Err(RuntimeServiceError::AttachmentMismatch);
                }
            }
            _ if !attachments.is_empty()
                || !request.attachment_indexes.is_empty()
                || !request.stream_id.is_empty() =>
            {
                return Err(RuntimeServiceError::AttachmentMismatch);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn cancel(
        &mut self,
        request: &CancelRequest,
    ) -> Result<CancelResponse, RuntimeServiceError> {
        if request.session_generation != self.owner.session_generation {
            return Err(RuntimeServiceError::GenerationMismatch);
        }
        let outcome = self.backend.cancel(&self.owner, request.request_id);
        self.diagnostics.record(DiagnosticEvent {
            operation: DiagnosticOperation::Cancel,
            outcome: DiagnosticOutcome::Cancelled,
        });
        Ok(CancelResponse { outcome })
    }
}

fn response_for(
    request: &RuntimeRequest,
    resource: RuntimeResource,
) -> Result<RuntimeResponse, RuntimeServiceError> {
    if !valid_id(&resource.handle) {
        return Err(RuntimeServiceError::BackendInvariant);
    }

    Ok(RuntimeResponse {
        outcome: match resource.state {
            RuntimeProcessState::Degraded => RuntimeOutcome::Degraded,
            _ => RuntimeOutcome::Succeeded,
        },
        operation_id: request.operation_id.clone(),
        resource_handle: resource.handle,
        stream_id: request.stream_id.clone(),
        result_digest: resource.result_digest,
        attachment_indexes: request.attachment_indexes.clone(),
    })
}

fn valid_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= MAX_ID_BYTES
        && matches!(bytes.next(), Some(first) if first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeServiceError {
    Unauthenticated,
    ContractMismatch,
    OwnerMismatch,
    GenerationMismatch,
    DeadlineExpired,
    InvalidRequest,
    InvalidResolvedProcess,
    ResolverMismatch,
    AttachmentMismatch,
    WaylandUnavailable,
    Unavailable,
    Conflict,
    NotFound,
    BackendInvariant,
}

impl fmt::Display for RuntimeServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unauthenticated => "runtime-session-unauthenticated",
            Self::ContractMismatch => "runtime-session-contract-mismatch",
            Self::OwnerMismatch => "runtime-owner-mismatch",
            Self::GenerationMismatch => "runtime-generation-mismatch",
            Self::DeadlineExpired => "runtime-deadline-expired",
            Self::InvalidRequest => "runtime-request-invalid",
            Self::InvalidResolvedProcess => "runtime-process-invalid",
            Self::ResolverMismatch => "runtime-resolver-mismatch",
            Self::AttachmentMismatch => "runtime-attachment-mismatch",
            Self::WaylandUnavailable => "runtime-wayland-unavailable",
            Self::Unavailable => "runtime-unavailable",
            Self::Conflict => "runtime-conflict",
            Self::NotFound => "runtime-not-found",
            Self::BackendInvariant => "runtime-backend-invariant",
        })
    }
}

impl std::error::Error for RuntimeServiceError {}

#[cfg(test)]
mod tests {
    use super::*;

    struct Session {
        uid: u32,
        process_uid: u32,
        generation: u64,
        authenticated: bool,
    }

    impl EstablishedComponentSession for Session {
        fn service_package(&self) -> &str {
            SERVICE_PACKAGE
        }

        fn endpoint_purpose(&self) -> &str {
            ENDPOINT_PURPOSE
        }

        fn endpoint_role(&self) -> &str {
            ENDPOINT_ROLE
        }

        fn is_authenticated(&self) -> bool {
            self.authenticated
        }

        fn uses_pre_authorized_transport(&self) -> bool {
            true
        }

        fn authenticated_uid(&self) -> u32 {
            self.uid
        }

        fn process_uid(&self) -> u32 {
            self.process_uid
        }

        fn session_generation(&self) -> u64 {
            self.generation
        }

        fn realm_id(&self) -> &str {
            "host"
        }

        fn workload_id(&self) -> &str {
            "tools"
        }
    }

    fn session() -> Session {
        let uid = nix::unistd::getuid().as_raw();
        Session {
            uid,
            process_uid: uid,
            generation: 7,
            authenticated: true,
        }
    }

    struct Resolver {
        graphical: bool,
    }

    impl ConfiguredProcessResolver for Resolver {
        fn resolve(
            &mut self,
            _: &RuntimeOwner,
            resource_id: &str,
            request_digest: &[u8; 32],
        ) -> Result<ResolvedProcess, RuntimeServiceError> {
            ResolvedProcess::new(
                resource_id.to_owned(),
                *request_digest,
                vec!["configured-program".to_owned()],
                self.graphical,
            )
        }
    }

    #[derive(Default)]
    struct Wayland {
        opens: usize,
        closes: usize,
    }

    impl WaylandControlPort for Wayland {
        fn open_display(
            &mut self,
            _: &RuntimeOwner,
            _: &ResolvedProcess,
            _: &str,
        ) -> Result<WaylandDisplayLease, RuntimeServiceError> {
            self.opens += 1;
            WaylandDisplayLease::new("display-handle".to_owned())
        }

        fn close_display(&mut self, _: WaylandDisplayLease) {
            self.closes += 1;
        }
    }

    struct Backend {
        fail_start: bool,
        terminal_opens: usize,
    }

    impl Backend {
        fn resource(state: RuntimeProcessState) -> RuntimeResource {
            RuntimeResource {
                handle: "process-handle".to_owned(),
                result_digest: [9; 32],
                state,
            }
        }
    }

    impl SystemdUserRuntimePort for Backend {
        fn ensure_scope(
            &mut self,
            _: &RuntimeOwner,
            _: &str,
            _: &str,
        ) -> Result<RuntimeResource, RuntimeServiceError> {
            Ok(Self::resource(RuntimeProcessState::Present))
        }

        fn start_process(
            &mut self,
            _: &RuntimeOwner,
            _: &str,
            process: &ResolvedProcess,
            display: Option<&WaylandDisplayLease>,
        ) -> Result<RuntimeResource, RuntimeServiceError> {
            assert_eq!(process.graphical(), display.is_some());
            if self.fail_start {
                Err(RuntimeServiceError::Unavailable)
            } else {
                Ok(Self::resource(RuntimeProcessState::Running))
            }
        }

        fn inspect_process(
            &mut self,
            _: &RuntimeOwner,
            _: &str,
        ) -> Result<RuntimeResource, RuntimeServiceError> {
            Ok(Self::resource(RuntimeProcessState::Running))
        }

        fn adopt_process(
            &mut self,
            _: &RuntimeOwner,
            _: &str,
            _: &str,
        ) -> Result<RuntimeResource, RuntimeServiceError> {
            Ok(Self::resource(RuntimeProcessState::Running))
        }

        fn stop_process(
            &mut self,
            _: &RuntimeOwner,
            _: &str,
            _: &str,
        ) -> Result<RuntimeResource, RuntimeServiceError> {
            Ok(Self::resource(RuntimeProcessState::Stopped))
        }

        fn open_terminal(
            &mut self,
            _: &RuntimeOwner,
            _: &str,
            _: &str,
            _: &AuthenticatedTerminalAttachment,
        ) -> Result<RuntimeResource, RuntimeServiceError> {
            self.terminal_opens += 1;
            Ok(Self::resource(RuntimeProcessState::Running))
        }

        fn cancel(&mut self, _: &RuntimeOwner, _: [u8; 16]) -> CancelResult {
            CancelResult::CancellationSignalled
        }
    }

    fn request(method: RuntimeMethod) -> RuntimeRequest {
        RuntimeRequest {
            request_id: [1; 16],
            idempotency_key: method.mutating().then_some([2; 32]),
            issued_at_unix_ms: 900,
            expires_at_unix_ms: 2_000,
            session_generation: 7,
            realm_id: "host".to_owned(),
            workload_id: "tools".to_owned(),
            resource_id: "browser".to_owned(),
            operation_id: if method.mutating() {
                "operation-1".to_owned()
            } else {
                String::new()
            },
            request_digest: method.mutating().then_some([3; 32]),
            stream_id: String::new(),
            attachment_indexes: Vec::new(),
            desired_state: method.expected_state(),
        }
    }

    fn service(
        graphical: bool,
        fail_start: bool,
    ) -> RuntimeSystemdUserService<Resolver, Wayland, Backend> {
        RuntimeSystemdUserService::new(
            RuntimeOwner::admit(&session()).unwrap(),
            Resolver { graphical },
            Wayland::default(),
            Backend {
                fail_start,
                terminal_opens: 0,
            },
        )
    }

    #[test]
    fn session_admission_requires_exact_non_root_process_uid() {
        let current = session();
        if current.uid != 0 {
            let owner = RuntimeOwner::admit(&current).unwrap();
            assert_eq!(owner.uid(), current.uid);
            assert!(!format!("{owner:?}").contains(&current.uid.to_string()));
        }

        let mut mismatch = session();
        mismatch.process_uid = mismatch.uid.saturating_add(1);
        assert_eq!(
            RuntimeOwner::admit(&mismatch),
            Err(RuntimeServiceError::OwnerMismatch)
        );

        let mut unauthenticated = session();
        unauthenticated.authenticated = false;
        assert_eq!(
            RuntimeOwner::admit(&unauthenticated),
            Err(RuntimeServiceError::Unauthenticated)
        );
    }

    #[test]
    fn resolved_process_is_bounded_and_debug_redacted() {
        let canary = "private-argv-canary";
        let process = ResolvedProcess::new(
            "browser".to_owned(),
            [3; 32],
            vec!["firefox".to_owned(), canary.to_owned()],
            true,
        )
        .unwrap();
        assert_eq!(process.argv()[1], canary);
        assert!(!format!("{process:?}").contains(canary));
        assert_eq!(
            ResolvedProcess::new("browser".to_owned(), [3; 32], Vec::new(), false),
            Err(RuntimeServiceError::InvalidResolvedProcess)
        );
    }

    #[test]
    fn diagnostics_are_closed_and_bounded() {
        let mut diagnostics = RuntimeDiagnostics {
            events: VecDeque::new(),
        };
        for _ in 0..MAX_DIAGNOSTIC_EVENTS + 10 {
            diagnostics.record(DiagnosticEvent {
                operation: DiagnosticOperation::StartProcess,
                outcome: DiagnosticOutcome::Success,
            });
        }
        assert_eq!(diagnostics.events().len(), MAX_DIAGNOSTIC_EVENTS);
        assert!(!format!("{diagnostics:?}").contains("uid"));
    }

    #[test]
    fn graphical_start_requires_wayland_control_and_closes_failed_lease() {
        if nix::unistd::getuid().is_root() {
            return;
        }
        let mut runtime = service(true, false);
        let response = runtime
            .dispatch(
                RuntimeMethod::StartProcess,
                &request(RuntimeMethod::StartProcess),
                &[],
                1_000,
            )
            .unwrap();
        assert_eq!(response.outcome, RuntimeOutcome::Succeeded);
        assert_eq!(runtime.wayland.opens, 1);
        assert_eq!(runtime.wayland.closes, 0);

        let mut runtime = service(true, true);
        assert_eq!(
            runtime.dispatch(
                RuntimeMethod::StartProcess,
                &request(RuntimeMethod::StartProcess),
                &[],
                1_000,
            ),
            Err(RuntimeServiceError::Unavailable)
        );
        assert_eq!(runtime.wayland.opens, 1);
        assert_eq!(runtime.wayland.closes, 1);
    }

    #[test]
    fn terminal_attachment_is_exactly_bound_to_owner_session_and_request() {
        if nix::unistd::getuid().is_root() {
            return;
        }
        let mut runtime = service(false, false);
        let mut request = request(RuntimeMethod::OpenTerminal);
        request.stream_id = "terminal-1".to_owned();
        request.attachment_indexes = vec![TERMINAL_ATTACHMENT_INDEX];
        let attachment = AuthenticatedTerminalAttachment {
            index: TERMINAL_ATTACHMENT_INDEX,
            owner_uid: runtime.owner.uid(),
            session_generation: runtime.owner.session_generation(),
            request_id: request.request_id,
            connected_stream: true,
            cloexec: true,
        };
        runtime
            .dispatch(
                RuntimeMethod::OpenTerminal,
                &request,
                std::slice::from_ref(&attachment),
                1_000,
            )
            .unwrap();
        assert_eq!(runtime.backend.terminal_opens, 1);

        let mut wrong_owner = attachment;
        wrong_owner.owner_uid = wrong_owner.owner_uid.saturating_add(1);
        assert_eq!(
            runtime.dispatch(RuntimeMethod::OpenTerminal, &request, &[wrong_owner], 1_000,),
            Err(RuntimeServiceError::AttachmentMismatch)
        );
        assert_eq!(runtime.backend.terminal_opens, 1);
    }

    #[test]
    fn request_identity_generation_deadline_and_digest_fail_closed() {
        if nix::unistd::getuid().is_root() {
            return;
        }
        let mut runtime = service(false, false);
        let mut invalid = request(RuntimeMethod::StartProcess);
        invalid.workload_id = "other".to_owned();
        assert_eq!(
            runtime.dispatch(RuntimeMethod::StartProcess, &invalid, &[], 1_000),
            Err(RuntimeServiceError::OwnerMismatch)
        );

        invalid = request(RuntimeMethod::StartProcess);
        invalid.session_generation += 1;
        assert_eq!(
            runtime.dispatch(RuntimeMethod::StartProcess, &invalid, &[], 1_000),
            Err(RuntimeServiceError::GenerationMismatch)
        );

        invalid = request(RuntimeMethod::StartProcess);
        invalid.expires_at_unix_ms = 1_000;
        assert_eq!(
            runtime.dispatch(RuntimeMethod::StartProcess, &invalid, &[], 1_000),
            Err(RuntimeServiceError::DeadlineExpired)
        );

        invalid = request(RuntimeMethod::StartProcess);
        invalid.request_digest = None;
        assert_eq!(
            runtime.dispatch(RuntimeMethod::StartProcess, &invalid, &[], 1_000),
            Err(RuntimeServiceError::InvalidRequest)
        );
    }
}
