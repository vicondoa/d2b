//! Persistent-shell service behind an authenticated `ComponentSession`.
//!
//! The service accepts only identity-bound requests and descriptors from the
//! established session. Process ownership stays in verified transient user
//! scopes supplied by the authenticated systemd-user runtime service.

use std::collections::VecDeque;
use std::fmt;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::output_ring::{MAX_TOTAL_RING_BYTES, RingRead, RingReservation};
use crate::services::shell::{ENDPOINT_PURPOSE, ENDPOINT_ROLE, SERVICE_PACKAGE};
use crate::shell_socket::{AttachmentError, validate_exact_terminal_attachment};
use crate::shell_supervisor::{ShellRegistry, ShellSupervisorError};
use crate::supervisor_protocol::{MAX_FUTURE_CLOCK_SKEW_MS, MAX_REQUEST_LIFETIME_MS, valid_id};

pub use crate::shell_socket::{AuthenticatedTerminalAttachment, TERMINAL_ATTACHMENT_INDEX};
pub use crate::shell_supervisor::{
    ScopeInspection, ScopeOwnership, ScopeProcessState, ScopeValidationError,
    VerifiedTransientScope,
};
pub use crate::supervisor_protocol::{
    ShellMethod, ShellRequest, ShellResponse, ShellState, ShellSummary,
};

pub const SERVICE_NAME: &str = "ShellService";
pub const MAX_DIAGNOSTIC_EVENTS: usize = 64;
pub const MAX_LIST_RESULTS: usize = 256;
pub const MAX_OUTPUT_READ_BYTES: usize = 1024 * 1024;

pub trait EstablishedShellSession {
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
pub struct ShellOwner {
    uid: u32,
    session_generation: u64,
    realm_id: String,
    workload_id: String,
}

impl ShellOwner {
    pub fn admit(session: &impl EstablishedShellSession) -> Result<Self, ShellServiceError> {
        if !session.is_authenticated() {
            return Err(ShellServiceError::Unauthenticated);
        }
        if !session.uses_pre_authorized_transport()
            || session.service_package() != SERVICE_PACKAGE
            || session.endpoint_purpose() != ENDPOINT_PURPOSE
            || session.endpoint_role() != ENDPOINT_ROLE
        {
            return Err(ShellServiceError::ContractMismatch);
        }
        let uid = session.authenticated_uid();
        if uid == 0
            || uid != session.process_uid()
            || uid != nix::unistd::getuid().as_raw()
            || session.session_generation() == 0
            || !valid_id(session.realm_id())
            || !valid_id(session.workload_id())
        {
            return Err(ShellServiceError::OwnerMismatch);
        }
        Ok(Self {
            uid,
            session_generation: session.session_generation(),
            realm_id: session.realm_id().to_owned(),
            workload_id: session.workload_id().to_owned(),
        })
    }

    pub fn uid(&self) -> u32 {
        self.uid
    }

    pub fn session_generation(&self) -> u64 {
        self.session_generation
    }
}

impl fmt::Debug for ShellOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ShellOwner(<redacted>)")
    }
}

pub trait AuthenticatedSystemdUserRuntime {
    fn create_shell_scope(
        &mut self,
        owner: &ShellOwner,
        resource_id: &str,
        operation_id: &str,
    ) -> Result<VerifiedTransientScope, ShellServiceError>;

    fn inspect_shell_scope(
        &mut self,
        owner: &ShellOwner,
        scope: &VerifiedTransientScope,
    ) -> Result<ScopeInspection, ShellServiceError>;

    fn adopt_shell_scope(
        &mut self,
        owner: &ShellOwner,
        scope: &VerifiedTransientScope,
        operation_id: &str,
    ) -> Result<ScopeInspection, ShellServiceError>;

    fn kill_shell_scope(
        &mut self,
        owner: &ShellOwner,
        scope: &VerifiedTransientScope,
        operation_id: &str,
    ) -> Result<ScopeInspection, ShellServiceError>;

    fn cancel(&mut self, owner: &ShellOwner, request_id: [u8; 16]) -> CancelOutcome;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    CancellationSignalled,
    AlreadyTerminal,
    UnknownRequest,
}

#[derive(Clone)]
pub struct ShellStateStore {
    registry: Arc<Mutex<ShellRegistry>>,
    owner: Arc<Mutex<Option<ShellOwner>>>,
}

impl ShellStateStore {
    pub fn new(total_output_bytes: usize) -> Result<Self, ShellServiceError> {
        if total_output_bytes == 0 || total_output_bytes > MAX_TOTAL_RING_BYTES {
            return Err(ShellServiceError::ReservationExhausted);
        }
        Ok(Self {
            registry: Arc::new(Mutex::new(ShellRegistry::new(total_output_bytes))),
            owner: Arc::new(Mutex::new(None)),
        })
    }
}

impl Default for ShellStateStore {
    fn default() -> Self {
        Self::new(MAX_TOTAL_RING_BYTES).expect("fixed shell output budget")
    }
}

impl fmt::Debug for ShellStateStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ShellStateStore(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticOperation {
    Create,
    Attach,
    Detach,
    List,
    Inspect,
    Kill,
    Cancel,
    Adopt,
    Disconnect,
}

impl From<ShellMethod> for DiagnosticOperation {
    fn from(method: ShellMethod) -> Self {
        match method {
            ShellMethod::Create => Self::Create,
            ShellMethod::Attach => Self::Attach,
            ShellMethod::Detach => Self::Detach,
            ShellMethod::List => Self::List,
            ShellMethod::Inspect => Self::Inspect,
            ShellMethod::Kill => Self::Kill,
            ShellMethod::Cancel => Self::Cancel,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticOutcome {
    Succeeded,
    Degraded,
    Denied,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticEvent {
    pub operation: DiagnosticOperation,
    pub outcome: DiagnosticOutcome,
}

pub struct ShellRuntimeService<B> {
    owner: ShellOwner,
    backend: B,
    state: ShellStateStore,
    store_owner_matches: bool,
    diagnostics: VecDeque<DiagnosticEvent>,
}

impl<B> fmt::Debug for ShellRuntimeService<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellRuntimeService")
            .field("owner", &"<redacted>")
            .field("diagnostic_events", &self.diagnostics.len())
            .finish_non_exhaustive()
    }
}

impl<B: AuthenticatedSystemdUserRuntime> ShellRuntimeService<B> {
    pub fn new(owner: ShellOwner, backend: B, state: ShellStateStore) -> Self {
        let store_owner_matches = state
            .owner
            .lock()
            .map(|mut bound| match bound.as_ref() {
                Some(existing) => existing == &owner,
                None => {
                    *bound = Some(owner.clone());
                    true
                }
            })
            .unwrap_or(false);
        Self {
            owner,
            backend,
            state,
            store_owner_matches,
            diagnostics: VecDeque::with_capacity(MAX_DIAGNOSTIC_EVENTS),
        }
    }

    pub fn diagnostics(&self) -> &VecDeque<DiagnosticEvent> {
        &self.diagnostics
    }

    pub fn dispatch(
        &mut self,
        request: &ShellRequest,
        attachments: Vec<AuthenticatedTerminalAttachment>,
        now_unix_ms: u64,
    ) -> Result<ShellResponse, ShellServiceError> {
        self.ensure_store_owner()?;
        let result = self.dispatch_inner(request, attachments, now_unix_ms);
        let outcome = match &result {
            Ok(response) if response.state == ShellState::Degraded => DiagnosticOutcome::Degraded,
            Ok(_) => DiagnosticOutcome::Succeeded,
            Err(ShellServiceError::RuntimeUnavailable) => DiagnosticOutcome::Unavailable,
            Err(_) => DiagnosticOutcome::Denied,
        };
        self.record(DiagnosticEvent {
            operation: request.method.into(),
            outcome,
        });
        result
    }

    fn dispatch_inner(
        &mut self,
        request: &ShellRequest,
        attachments: Vec<AuthenticatedTerminalAttachment>,
        now_unix_ms: u64,
    ) -> Result<ShellResponse, ShellServiceError> {
        self.admit_request(request, attachments.len(), now_unix_ms)?;
        match request.method {
            ShellMethod::Create => self.create(request),
            ShellMethod::Attach => self.attach(request, attachments),
            ShellMethod::Detach => self.detach(request),
            ShellMethod::List => self.list(request),
            ShellMethod::Inspect => self.inspect(request),
            ShellMethod::Kill => self.kill(request),
            ShellMethod::Cancel => Ok(self.cancel(request)),
        }
    }

    fn create(&mut self, request: &ShellRequest) -> Result<ShellResponse, ShellServiceError> {
        let reservation = self
            .registry()?
            .reserve(&request.resource_id, request.output_ring_bytes)
            .map_err(ShellServiceError::from)?;
        let scope = self.backend.create_shell_scope(
            &self.owner,
            &request.resource_id,
            &request.operation_id,
        )?;
        // From here on, the real systemd-user scope + PTY already exists.
        // Any failure must tear it back down before returning, so a
        // partial `Create` failure never leaks a live, untracked
        // supervised process (the reservation itself releases on drop).
        match self.finish_create(request, &scope, reservation) {
            Ok(response) => Ok(response),
            Err(error) => {
                let _ = self
                    .backend
                    .kill_shell_scope(&self.owner, &scope, &request.operation_id);
                Err(error)
            }
        }
    }

    fn finish_create(
        &mut self,
        request: &ShellRequest,
        scope: &VerifiedTransientScope,
        reservation: RingReservation,
    ) -> Result<ShellResponse, ShellServiceError> {
        verify_scope_binding(&self.owner, scope, &request.resource_id)?;
        let inspection = self.backend.inspect_shell_scope(&self.owner, scope)?;
        if inspection
            != (ScopeInspection {
                ownership: ScopeOwnership::Exact,
                process_state: ScopeProcessState::Running,
            })
        {
            return Err(ShellServiceError::ScopeOwnershipMismatch);
        }
        self.registry()?
            .insert_reserved(scope.clone(), reservation, ScopeOwnership::Exact)
            .map_err(ShellServiceError::from)?;
        Ok(response(request, ShellState::Running))
    }

    fn attach(
        &mut self,
        request: &ShellRequest,
        attachments: Vec<AuthenticatedTerminalAttachment>,
    ) -> Result<ShellResponse, ShellServiceError> {
        self.registry()?.reconcile_hangups();
        let scope = self
            .registry()?
            .get(&request.resource_id)
            .map_err(ShellServiceError::from)?
            .scope()
            .clone();
        let inspection = self.backend.inspect_shell_scope(&self.owner, &scope)?;
        if inspection.ownership != ScopeOwnership::Exact
            || inspection.process_state != ScopeProcessState::Running
        {
            self.registry()?
                .get_mut(&request.resource_id)
                .map_err(ShellServiceError::from)?
                .reconcile(inspection);
            return Err(ShellServiceError::ScopeOwnershipMismatch);
        }
        let fd = validate_exact_terminal_attachment(
            attachments,
            self.owner.uid,
            self.owner.session_generation,
            request.request_id,
        )
        .map_err(ShellServiceError::from)?;
        self.registry()?
            .get_mut(&request.resource_id)
            .map_err(ShellServiceError::from)?
            .attach(request.stream_id.clone(), fd)
            .map_err(ShellServiceError::from)?;
        Ok(response(request, ShellState::Attached))
    }

    fn detach(&mut self, request: &ShellRequest) -> Result<ShellResponse, ShellServiceError> {
        let mut registry = self.registry()?;
        let supervisor = registry
            .get_mut(&request.resource_id)
            .map_err(ShellServiceError::from)?;
        supervisor
            .detach(&request.stream_id)
            .map_err(ShellServiceError::from)?;
        Ok(response(request, supervisor.state()))
    }

    fn list(&mut self, request: &ShellRequest) -> Result<ShellResponse, ShellServiceError> {
        let mut registry = self.registry()?;
        registry.reconcile_hangups();
        let shells = registry
            .iter()
            .take(MAX_LIST_RESULTS)
            .map(|(resource_id, supervisor)| ShellSummary {
                resource_id: resource_id.clone(),
                state: supervisor.state(),
            })
            .collect();
        let mut result = response(request, ShellState::Running);
        result.shells = shells;
        Ok(result)
    }

    fn inspect(&mut self, request: &ShellRequest) -> Result<ShellResponse, ShellServiceError> {
        self.registry()?.reconcile_hangups();
        let scope = self
            .registry()?
            .get(&request.resource_id)
            .map_err(ShellServiceError::from)?
            .scope()
            .clone();
        let inspection = self.backend.inspect_shell_scope(&self.owner, &scope)?;
        let mut registry = self.registry()?;
        let supervisor = registry
            .get_mut(&request.resource_id)
            .map_err(ShellServiceError::from)?;
        supervisor.reconcile(inspection);
        Ok(response(request, supervisor.state()))
    }

    fn kill(&mut self, request: &ShellRequest) -> Result<ShellResponse, ShellServiceError> {
        let scope = self
            .registry()?
            .get(&request.resource_id)
            .map_err(ShellServiceError::from)?
            .scope()
            .clone();
        let before = self.backend.inspect_shell_scope(&self.owner, &scope)?;
        if before.ownership != ScopeOwnership::Exact {
            return Err(ShellServiceError::ScopeOwnershipMismatch);
        }
        let killed = self
            .backend
            .kill_shell_scope(&self.owner, &scope, &request.operation_id)?;
        if killed.ownership != ScopeOwnership::Exact
            || killed.process_state != ScopeProcessState::Exited
        {
            return Err(ShellServiceError::ScopeOwnershipMismatch);
        }
        let mut supervisor = self
            .registry()?
            .remove(&request.resource_id)
            .map_err(ShellServiceError::from)?;
        if !supervisor.scope().same_identity(&scope) {
            return Err(ShellServiceError::ScopeOwnershipMismatch);
        }
        supervisor.close();
        Ok(response(request, ShellState::Exited))
    }

    /// Reports the backend's real cancellation outcome instead of a fixed,
    /// fabricated state. `ShellState` has no dedicated cancellation-outcome
    /// variant, so the real `CancelOutcome` is mapped onto the closest
    /// truthful state: an already-terminal scope reports `Exited`, anything
    /// else (signalled or unknown) reports `Running` since neither implies
    /// the scope has actually exited.
    fn cancel(&mut self, request: &ShellRequest) -> ShellResponse {
        let outcome = self.backend.cancel(&self.owner, request.request_id);
        let state = match outcome {
            CancelOutcome::AlreadyTerminal => ShellState::Exited,
            CancelOutcome::CancellationSignalled | CancelOutcome::UnknownRequest => {
                ShellState::Running
            }
        };
        response(request, state)
    }

    pub fn adopt(
        &mut self,
        scope: VerifiedTransientScope,
        operation_id: &str,
        output_ring_bytes: usize,
    ) -> Result<ShellState, ShellServiceError> {
        self.ensure_store_owner()?;
        verify_scope_binding(&self.owner, &scope, scope.resource_id())?;
        if !valid_id(operation_id) {
            return Err(ShellServiceError::InvalidRequest);
        }
        let reservation = self
            .registry()?
            .reserve(scope.resource_id(), output_ring_bytes)
            .map_err(ShellServiceError::from)?;
        let inspection = self
            .backend
            .adopt_shell_scope(&self.owner, &scope, operation_id)?;
        if inspection.ownership == ScopeOwnership::Mismatch {
            return Err(ShellServiceError::ScopeOwnershipMismatch);
        }
        let state = if inspection.ownership == ScopeOwnership::Ambiguous {
            ShellState::Degraded
        } else if inspection.process_state == ScopeProcessState::Running {
            ShellState::Running
        } else {
            ShellState::Exited
        };
        let resource_id = scope.resource_id().to_owned();
        self.registry()?
            .insert_reserved(scope, reservation, inspection.ownership)
            .map_err(ShellServiceError::from)?;
        self.registry()?
            .get_mut(&resource_id)
            .map_err(ShellServiceError::from)?
            .reconcile(inspection);
        self.record(DiagnosticEvent {
            operation: DiagnosticOperation::Adopt,
            outcome: if state == ShellState::Degraded {
                DiagnosticOutcome::Degraded
            } else {
                DiagnosticOutcome::Succeeded
            },
        });
        Ok(state)
    }

    pub fn disconnect(&mut self) -> Result<(), ShellServiceError> {
        self.ensure_store_owner()?;
        self.registry()?.detach_all();
        self.record(DiagnosticEvent {
            operation: DiagnosticOperation::Disconnect,
            outcome: DiagnosticOutcome::Succeeded,
        });
        Ok(())
    }

    pub fn append_output(&self, resource_id: &str, bytes: &[u8]) -> Result<(), ShellServiceError> {
        self.ensure_store_owner()?;
        if !valid_id(resource_id) {
            return Err(ShellServiceError::InvalidRequest);
        }
        self.registry()?
            .get(resource_id)
            .map_err(ShellServiceError::from)?
            .append_output(bytes);
        Ok(())
    }

    pub fn read_output(
        &self,
        resource_id: &str,
        cursor: u64,
        max_len: usize,
        wait: bool,
        timeout: Duration,
    ) -> Result<ShellOutput, ShellServiceError> {
        self.ensure_store_owner()?;
        if !valid_id(resource_id) || max_len == 0 || max_len > MAX_OUTPUT_READ_BYTES {
            return Err(ShellServiceError::InvalidRequest);
        }
        let read = self
            .registry()?
            .get(resource_id)
            .map_err(ShellServiceError::from)?
            .read_output(cursor, max_len, wait, timeout);
        Ok(read.into())
    }

    fn registry(&self) -> Result<std::sync::MutexGuard<'_, ShellRegistry>, ShellServiceError> {
        self.state
            .registry
            .lock()
            .map_err(|_| ShellServiceError::RuntimeUnavailable)
    }

    fn ensure_store_owner(&self) -> Result<(), ShellServiceError> {
        self.store_owner_matches
            .then_some(())
            .ok_or(ShellServiceError::OwnerMismatch)
    }

    fn record(&mut self, event: DiagnosticEvent) {
        if self.diagnostics.len() == MAX_DIAGNOSTIC_EVENTS {
            self.diagnostics.pop_front();
        }
        self.diagnostics.push_back(event);
    }

    fn admit_request(
        &self,
        request: &ShellRequest,
        attachment_count: usize,
        now_unix_ms: u64,
    ) -> Result<(), ShellServiceError> {
        if request.session_generation != self.owner.session_generation {
            return Err(ShellServiceError::GenerationMismatch);
        }
        if request.issued_at_unix_ms > request.expires_at_unix_ms
            || request.issued_at_unix_ms > now_unix_ms.saturating_add(MAX_FUTURE_CLOCK_SKEW_MS)
            || request.expires_at_unix_ms <= now_unix_ms
            || request
                .expires_at_unix_ms
                .saturating_sub(request.issued_at_unix_ms)
                > MAX_REQUEST_LIFETIME_MS
        {
            return Err(ShellServiceError::DeadlineExpired);
        }
        if request.realm_id != self.owner.realm_id
            || request.workload_id != self.owner.workload_id
            || request.request_id == [0; 16]
        {
            return Err(ShellServiceError::OwnerMismatch);
        }
        let resource_required = !matches!(request.method, ShellMethod::List | ShellMethod::Cancel);
        if (resource_required && !valid_id(&request.resource_id))
            || (!request.resource_id.is_empty() && !valid_id(&request.resource_id))
            || (request.method.mutating()
                && (request.idempotency_key.is_none() || !valid_id(&request.operation_id)))
            || (!request.operation_id.is_empty() && !valid_id(&request.operation_id))
        {
            return Err(ShellServiceError::InvalidRequest);
        }
        match request.method {
            ShellMethod::Attach => {
                if attachment_count != 1
                    || request.attachment_indexes != [TERMINAL_ATTACHMENT_INDEX]
                    || !valid_id(&request.stream_id)
                    || request.output_ring_bytes != 0
                {
                    return Err(ShellServiceError::AttachmentMismatch);
                }
            }
            ShellMethod::Detach => {
                if attachment_count != 0
                    || !request.attachment_indexes.is_empty()
                    || !valid_id(&request.stream_id)
                    || request.output_ring_bytes != 0
                {
                    return Err(ShellServiceError::InvalidRequest);
                }
            }
            ShellMethod::Create => {
                if attachment_count != 0
                    || !request.attachment_indexes.is_empty()
                    || !request.stream_id.is_empty()
                {
                    return Err(ShellServiceError::InvalidRequest);
                }
            }
            _ => {
                if attachment_count != 0
                    || !request.attachment_indexes.is_empty()
                    || !request.stream_id.is_empty()
                    || request.output_ring_bytes != 0
                {
                    return Err(ShellServiceError::InvalidRequest);
                }
            }
        }
        Ok(())
    }
}

fn verify_scope_binding(
    owner: &ShellOwner,
    scope: &VerifiedTransientScope,
    resource_id: &str,
) -> Result<(), ShellServiceError> {
    if scope.resource_id() != resource_id
        || scope.owner_uid() != owner.uid
        || scope.session_generation() != owner.session_generation
    {
        return Err(ShellServiceError::ScopeOwnershipMismatch);
    }
    Ok(())
}

fn response(request: &ShellRequest, state: ShellState) -> ShellResponse {
    ShellResponse {
        state,
        operation_id: request.operation_id.clone(),
        resource_id: request.resource_id.clone(),
        stream_id: request.stream_id.clone(),
        attachment_indexes: request.attachment_indexes.clone(),
        shells: Vec::new(),
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ShellOutput {
    pub data: Vec<u8>,
    pub next_cursor: u64,
    pub eof: bool,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub timed_out: bool,
}

impl From<RingRead> for ShellOutput {
    fn from(read: RingRead) -> Self {
        Self {
            data: read.data,
            next_cursor: read.next_cursor,
            eof: read.eof,
            dropped_bytes: read.dropped_bytes,
            truncated: read.truncated,
            timed_out: read.timed_out,
        }
    }
}

impl fmt::Debug for ShellOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellOutput")
            .field("data_len", &self.data.len())
            .field("next_cursor", &self.next_cursor)
            .field("eof", &self.eof)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .field("timed_out", &self.timed_out)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellServiceError {
    Unauthenticated,
    ContractMismatch,
    OwnerMismatch,
    GenerationMismatch,
    DeadlineExpired,
    InvalidRequest,
    AttachmentMismatch,
    ScopeOwnershipMismatch,
    ReservationExhausted,
    AlreadyExists,
    AlreadyAttached,
    NotFound,
    RuntimeUnavailable,
}

impl fmt::Display for ShellServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unauthenticated => "shell-session-unauthenticated",
            Self::ContractMismatch => "shell-session-contract-mismatch",
            Self::OwnerMismatch => "shell-owner-mismatch",
            Self::GenerationMismatch => "shell-generation-mismatch",
            Self::DeadlineExpired => "shell-deadline-expired",
            Self::InvalidRequest => "shell-request-invalid",
            Self::AttachmentMismatch => "shell-attachment-mismatch",
            Self::ScopeOwnershipMismatch => "shell-scope-ownership-mismatch",
            Self::ReservationExhausted => "shell-output-reservation-exhausted",
            Self::AlreadyExists => "shell-already-exists",
            Self::AlreadyAttached => "shell-already-attached",
            Self::NotFound => "shell-not-found",
            Self::RuntimeUnavailable => "shell-runtime-unavailable",
        })
    }
}

impl std::error::Error for ShellServiceError {}

impl From<ShellSupervisorError> for ShellServiceError {
    fn from(error: ShellSupervisorError) -> Self {
        match error {
            ShellSupervisorError::ScopeOwnershipMismatch => Self::ScopeOwnershipMismatch,
            ShellSupervisorError::ReservationExhausted => Self::ReservationExhausted,
            ShellSupervisorError::AlreadyExists => Self::AlreadyExists,
            ShellSupervisorError::NotFound => Self::NotFound,
            ShellSupervisorError::AlreadyAttached => Self::AlreadyAttached,
            ShellSupervisorError::AttachmentMismatch => Self::AttachmentMismatch,
            ShellSupervisorError::LegacyEntrypointDisabled => Self::RuntimeUnavailable,
        }
    }
}

impl From<AttachmentError> for ShellServiceError {
    fn from(_: AttachmentError) -> Self {
        Self::AttachmentMismatch
    }
}

// The old helper call graph remains frozen outside this component. It fails
// closed here until the shared bootstrap owner removes it.
use crate::runtime::{
    PersistedScope, RuntimeError, RuntimeLedger, ScopeRuntime, ShellOperationBegin,
};
use crate::systemd::UserScopeManager;
use d2b_contracts::unsafe_local_wire::{
    HelperPersistentShellSnapshot, HelperScopeState, HelperShellRequest, UnsafeLocalHelperToDaemon,
};

pub struct ShellDispatch {
    pub(crate) response: UnsafeLocalHelperToDaemon,
    pub(crate) terminal_fd: Option<OwnedFd>,
}

impl ShellDispatch {
    pub fn into_parts(self) -> (UnsafeLocalHelperToDaemon, Option<OwnedFd>) {
        (self.response, self.terminal_fd)
    }
}

impl fmt::Debug for ShellDispatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellDispatch")
            .field("response", &"<redacted>")
            .field("has_terminal_fd", &self.terminal_fd.is_some())
            .finish()
    }
}

pub(crate) fn dispatch<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    _request: HelperShellRequest,
) -> Result<ShellDispatch, RuntimeError> {
    let _ = (&runtime.shell_home, &runtime.executable);
    let _ = RuntimeLedger::begin_shell_operation;
    let _ = RuntimeLedger::reserve_shell_name;
    let _ = RuntimeLedger::clear_shell_operation;
    let _ = RuntimeLedger::complete_shell_operation;
    let _ = RuntimeLedger::remember_completed_shell_operation;
    let _ = consume_legacy_operation;
    Err(RuntimeError::InvalidRequest)
}

fn consume_legacy_operation(operation: ShellOperationBegin) {
    match operation {
        ShellOperationBegin::Started(reservation) => {
            let _ = reservation;
        }
        ShellOperationBegin::ExistingScope(scope) => {
            let _ = scope;
        }
        ShellOperationBegin::Replayed(response) => {
            let _ = response;
        }
    }
}

pub(crate) fn snapshot_shell<M: UserScopeManager>(
    _runtime: &ScopeRuntime<M>,
    scope: &PersistedScope,
    manager_state: HelperScopeState,
) -> (HelperScopeState, Option<HelperPersistentShellSnapshot>) {
    let snapshot = scope
        .persistent_shell
        .as_ref()
        .map(|metadata| HelperPersistentShellSnapshot {
            name: metadata.name.clone(),
            state: d2b_contracts::public_wire::ShellSessionState::PoolUnavailable,
            attached: false,
            supervisor_id: metadata.supervisor_id.clone(),
        });
    let state = if snapshot.is_some() {
        HelperScopeState::Degraded
    } else {
        manager_state
    };
    (state, snapshot)
}
