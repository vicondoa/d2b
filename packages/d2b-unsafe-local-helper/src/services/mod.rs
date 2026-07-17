pub mod runtime_systemd_user;
pub mod shell;
pub mod tty;

use crate::shell_runtime::{
    AuthenticatedSystemdUserRuntime, AuthenticatedTerminalAttachment as ShellTerminalAttachment,
    CancelOutcome as ShellCancelOutcome, EstablishedShellSession, ShellMethod, ShellOwner,
    ShellRequest, ShellResponse, ShellRuntimeService, ShellServiceError, ShellState,
    ShellStateStore, VerifiedTransientScope,
};
use runtime_systemd_user::{
    AuthenticatedTerminalAttachment, CancelRequest, CancelResponse, ConfiguredProcessResolver,
    EstablishedComponentSession, RuntimeMethod, RuntimeOwner, RuntimeRequest, RuntimeResponse,
    RuntimeServiceError, RuntimeSystemdUserService, SystemdUserRuntimePort, WaylandControlPort,
};
use std::collections::BTreeSet;
use std::fmt;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex, MutexGuard};
use tty::{
    CancelOutcome as TtyCancelOutcome, TransientUserScope, TtyOneShotError, TtyOneShotRequest,
    TtyOneShotRuntime, TtyOneShotService, TtyOneShotSpec, ValidatedTerminal,
};

pub const MAX_RECONNECT_ATTEMPTS: u16 = d2b_contracts::v2_component_session::MAX_RECONNECT_ATTEMPTS;
pub const MAX_RECONNECT_WINDOW_MS: u64 =
    d2b_contracts::v2_component_session::MAX_RECONNECT_WINDOW_MS as u64;

#[derive(Clone, PartialEq, Eq)]
pub struct AuthenticatedRuntimeSession {
    uid: u32,
    process_uid: u32,
    generation: u64,
    realm_id: String,
    workload_id: String,
}

impl AuthenticatedRuntimeSession {
    pub fn admit(
        uid: u32,
        process_uid: u32,
        generation: u64,
        realm_id: String,
        workload_id: String,
    ) -> Result<Self, CompositionError> {
        let current_uid = nix::unistd::getuid().as_raw();
        let effective_uid = nix::unistd::geteuid().as_raw();
        if uid == 0
            || uid != process_uid
            || uid != current_uid
            || uid != effective_uid
            || generation == 0
        {
            return Err(CompositionError::OwnerMismatch);
        }
        let session = Self {
            uid,
            process_uid,
            generation,
            realm_id,
            workload_id,
        };
        RuntimeOwner::admit(&RuntimeSession(&session))?;
        ShellOwner::admit(&ShellSession(&session))?;
        Ok(session)
    }

    pub fn for_current_process(
        generation: u64,
        realm_id: String,
        workload_id: String,
    ) -> Result<Self, CompositionError> {
        let uid = nix::unistd::getuid().as_raw();
        Self::admit(uid, uid, generation, realm_id, workload_id)
    }

    pub const fn uid(&self) -> u32 {
        self.uid
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

impl fmt::Debug for AuthenticatedRuntimeSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticatedRuntimeSession(<redacted>)")
    }
}

struct RuntimeSession<'a>(&'a AuthenticatedRuntimeSession);

impl EstablishedComponentSession for RuntimeSession<'_> {
    fn service_package(&self) -> &str {
        runtime_systemd_user::SERVICE_PACKAGE
    }

    fn endpoint_purpose(&self) -> &str {
        runtime_systemd_user::ENDPOINT_PURPOSE
    }

    fn endpoint_role(&self) -> &str {
        runtime_systemd_user::ENDPOINT_ROLE
    }

    fn is_authenticated(&self) -> bool {
        true
    }

    fn uses_pre_authorized_transport(&self) -> bool {
        true
    }

    fn authenticated_uid(&self) -> u32 {
        self.0.uid
    }

    fn process_uid(&self) -> u32 {
        self.0.process_uid
    }

    fn session_generation(&self) -> u64 {
        self.0.generation
    }

    fn realm_id(&self) -> &str {
        &self.0.realm_id
    }

    fn workload_id(&self) -> &str {
        &self.0.workload_id
    }
}

struct ShellSession<'a>(&'a AuthenticatedRuntimeSession);

impl EstablishedShellSession for ShellSession<'_> {
    fn service_package(&self) -> &str {
        shell::SERVICE_PACKAGE
    }

    fn endpoint_purpose(&self) -> &str {
        shell::ENDPOINT_PURPOSE
    }

    fn endpoint_role(&self) -> &str {
        shell::ENDPOINT_ROLE
    }

    fn is_authenticated(&self) -> bool {
        true
    }

    fn uses_pre_authorized_transport(&self) -> bool {
        true
    }

    fn authenticated_uid(&self) -> u32 {
        self.0.uid
    }

    fn process_uid(&self) -> u32 {
        self.0.process_uid
    }

    fn session_generation(&self) -> u64 {
        self.0.generation
    }

    fn realm_id(&self) -> &str {
        &self.0.realm_id
    }

    fn workload_id(&self) -> &str {
        &self.0.workload_id
    }
}

pub struct SharedSystemdUserRuntime<B> {
    backend: Arc<Mutex<B>>,
}

impl<B> SharedSystemdUserRuntime<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(Mutex::new(backend)),
        }
    }

    fn lock_runtime(&self) -> Result<MutexGuard<'_, B>, RuntimeServiceError> {
        self.backend
            .lock()
            .map_err(|_| RuntimeServiceError::Unavailable)
    }

    fn lock_shell(&self) -> Result<MutexGuard<'_, B>, ShellServiceError> {
        self.backend
            .lock()
            .map_err(|_| ShellServiceError::RuntimeUnavailable)
    }
}

impl<B> Clone for SharedSystemdUserRuntime<B> {
    fn clone(&self) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
        }
    }
}

impl<B> fmt::Debug for SharedSystemdUserRuntime<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SharedSystemdUserRuntime(<redacted>)")
    }
}

impl<B: SystemdUserRuntimePort> SystemdUserRuntimePort for SharedSystemdUserRuntime<B> {
    fn ensure_scope(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
        operation_id: &str,
    ) -> Result<runtime_systemd_user::RuntimeResource, RuntimeServiceError> {
        self.lock_runtime()?
            .ensure_scope(owner, resource_id, operation_id)
    }

    fn start_process(
        &mut self,
        owner: &RuntimeOwner,
        operation_id: &str,
        process: &runtime_systemd_user::ResolvedProcess,
        display: Option<&runtime_systemd_user::WaylandDisplayLease>,
    ) -> Result<runtime_systemd_user::RuntimeResource, RuntimeServiceError> {
        self.lock_runtime()?
            .start_process(owner, operation_id, process, display)
    }

    fn inspect_process(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
    ) -> Result<runtime_systemd_user::RuntimeResource, RuntimeServiceError> {
        self.lock_runtime()?.inspect_process(owner, resource_id)
    }

    fn adopt_process(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
        operation_id: &str,
    ) -> Result<runtime_systemd_user::RuntimeResource, RuntimeServiceError> {
        self.lock_runtime()?
            .adopt_process(owner, resource_id, operation_id)
    }

    fn stop_process(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
        operation_id: &str,
    ) -> Result<runtime_systemd_user::RuntimeResource, RuntimeServiceError> {
        self.lock_runtime()?
            .stop_process(owner, resource_id, operation_id)
    }

    fn open_terminal(
        &mut self,
        owner: &RuntimeOwner,
        resource_id: &str,
        stream_id: &str,
        attachment: &AuthenticatedTerminalAttachment,
    ) -> Result<runtime_systemd_user::RuntimeResource, RuntimeServiceError> {
        self.lock_runtime()?
            .open_terminal(owner, resource_id, stream_id, attachment)
    }

    fn cancel(
        &mut self,
        owner: &RuntimeOwner,
        request_id: [u8; 16],
    ) -> runtime_systemd_user::CancelResult {
        self.lock_runtime().map_or(
            runtime_systemd_user::CancelResult::UnknownRequest,
            |mut backend| SystemdUserRuntimePort::cancel(&mut *backend, owner, request_id),
        )
    }
}

impl<B: AuthenticatedSystemdUserRuntime> AuthenticatedSystemdUserRuntime
    for SharedSystemdUserRuntime<B>
{
    fn create_shell_scope(
        &mut self,
        owner: &ShellOwner,
        resource_id: &str,
        operation_id: &str,
    ) -> Result<VerifiedTransientScope, ShellServiceError> {
        self.lock_shell()?
            .create_shell_scope(owner, resource_id, operation_id)
    }

    fn inspect_shell_scope(
        &mut self,
        owner: &ShellOwner,
        scope: &VerifiedTransientScope,
    ) -> Result<crate::shell_runtime::ScopeInspection, ShellServiceError> {
        self.lock_shell()?.inspect_shell_scope(owner, scope)
    }

    fn adopt_shell_scope(
        &mut self,
        owner: &ShellOwner,
        scope: &VerifiedTransientScope,
        operation_id: &str,
    ) -> Result<crate::shell_runtime::ScopeInspection, ShellServiceError> {
        self.lock_shell()?
            .adopt_shell_scope(owner, scope, operation_id)
    }

    fn kill_shell_scope(
        &mut self,
        owner: &ShellOwner,
        scope: &VerifiedTransientScope,
        operation_id: &str,
    ) -> Result<crate::shell_runtime::ScopeInspection, ShellServiceError> {
        self.lock_shell()?
            .kill_shell_scope(owner, scope, operation_id)
    }

    fn cancel(&mut self, owner: &ShellOwner, request_id: [u8; 16]) -> ShellCancelOutcome {
        self.lock_shell()
            .map_or(ShellCancelOutcome::UnknownRequest, |mut backend| {
                AuthenticatedSystemdUserRuntime::cancel(&mut *backend, owner, request_id)
            })
    }
}

impl<B: TtyOneShotRuntime> TtyOneShotRuntime for SharedSystemdUserRuntime<B> {
    fn start_transient_user_scope(
        &mut self,
        owner: &RuntimeOwner,
        request: &TtyOneShotRequest,
        spec: &TtyOneShotSpec,
        terminal: ValidatedTerminal,
    ) -> Result<TransientUserScope, TtyOneShotError> {
        self.backend
            .lock()
            .map_err(|_| TtyOneShotError::RuntimeUnavailable)?
            .start_transient_user_scope(owner, request, spec, terminal)
    }

    fn teardown_transient_user_scope(
        &mut self,
        owner: &RuntimeOwner,
        scope: &TransientUserScope,
    ) -> Result<(), TtyOneShotError> {
        self.backend
            .lock()
            .map_err(|_| TtyOneShotError::RuntimeUnavailable)?
            .teardown_transient_user_scope(owner, scope)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLifecycleState {
    Connected,
    Disconnected,
    Failed,
}

#[derive(Clone)]
pub struct RecoveredShell {
    pub scope: VerifiedTransientScope,
    pub operation_id: String,
    pub output_ring_bytes: usize,
}

impl fmt::Debug for RecoveredShell {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveredShell(<redacted>)")
    }
}

#[derive(Debug, Default)]
struct ReconnectBudget {
    attempts: u16,
    window_started_unix_ms: Option<u64>,
}

impl ReconnectBudget {
    fn record(&mut self, now_unix_ms: u64) -> Result<(), CompositionError> {
        if let Some(started) = self.window_started_unix_ms {
            if now_unix_ms < started {
                return Err(CompositionError::ReconnectLimit);
            }
            if now_unix_ms.saturating_sub(started) > MAX_RECONNECT_WINDOW_MS {
                self.attempts = 0;
                self.window_started_unix_ms = Some(now_unix_ms);
            }
        } else {
            self.window_started_unix_ms = Some(now_unix_ms);
        }
        self.attempts = self.attempts.saturating_add(1);
        if self.attempts > MAX_RECONNECT_ATTEMPTS {
            Err(CompositionError::ReconnectLimit)
        } else {
            Ok(())
        }
    }

    fn reset(&mut self) {
        self.attempts = 0;
        self.window_started_unix_ms = None;
    }
}

pub struct RuntimeComposition<R, W, B>
where
    B: SystemdUserRuntimePort + AuthenticatedSystemdUserRuntime + TtyOneShotRuntime,
{
    session: AuthenticatedRuntimeSession,
    state: RuntimeLifecycleState,
    runtime: RuntimeSystemdUserService<R, W, SharedSystemdUserRuntime<B>>,
    shell: ShellRuntimeService<SharedSystemdUserRuntime<B>>,
    tty: TtyOneShotService<SharedSystemdUserRuntime<B>>,
    backend: SharedSystemdUserRuntime<B>,
    known_shells: BTreeSet<String>,
    shell_output_budget: usize,
    reconnect: ReconnectBudget,
}

impl<R, W, B> fmt::Debug for RuntimeComposition<R, W, B>
where
    B: SystemdUserRuntimePort + AuthenticatedSystemdUserRuntime + TtyOneShotRuntime,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeComposition")
            .field("state", &self.state)
            .field("known_shell_count", &self.known_shells.len())
            .finish_non_exhaustive()
    }
}

impl<R, W, B> RuntimeComposition<R, W, B>
where
    R: ConfiguredProcessResolver,
    W: WaylandControlPort,
    B: SystemdUserRuntimePort + AuthenticatedSystemdUserRuntime + TtyOneShotRuntime,
{
    pub fn new(
        session: AuthenticatedRuntimeSession,
        resolver: R,
        wayland: W,
        backend: B,
        shell_output_budget: usize,
    ) -> Result<Self, CompositionError> {
        let backend = SharedSystemdUserRuntime::new(backend);
        Self::from_shared(session, resolver, wayland, backend, shell_output_budget)
    }

    fn from_shared(
        session: AuthenticatedRuntimeSession,
        resolver: R,
        wayland: W,
        backend: SharedSystemdUserRuntime<B>,
        shell_output_budget: usize,
    ) -> Result<Self, CompositionError> {
        let runtime_owner = RuntimeOwner::admit(&RuntimeSession(&session))?;
        let shell_owner = ShellOwner::admit(&ShellSession(&session))?;
        let shell_state = ShellStateStore::new(shell_output_budget)?;
        Ok(Self {
            session,
            state: RuntimeLifecycleState::Connected,
            runtime: RuntimeSystemdUserService::new(
                runtime_owner.clone(),
                resolver,
                wayland,
                backend.clone(),
            ),
            shell: ShellRuntimeService::new(shell_owner, backend.clone(), shell_state),
            tty: TtyOneShotService::new(runtime_owner, backend.clone()),
            backend,
            known_shells: BTreeSet::new(),
            shell_output_budget,
            reconnect: ReconnectBudget::default(),
        })
    }

    pub const fn state(&self) -> RuntimeLifecycleState {
        self.state
    }

    pub const fn session(&self) -> &AuthenticatedRuntimeSession {
        &self.session
    }

    fn require_connected(&self) -> Result<(), CompositionError> {
        if self.state == RuntimeLifecycleState::Connected {
            Ok(())
        } else {
            Err(CompositionError::SessionUnavailable)
        }
    }

    pub fn dispatch_runtime(
        &mut self,
        method: RuntimeMethod,
        request: &RuntimeRequest,
        attachments: &[AuthenticatedTerminalAttachment],
        now_unix_ms: u64,
    ) -> Result<RuntimeResponse, CompositionError> {
        self.require_connected()?;
        self.runtime
            .dispatch(method, request, attachments, now_unix_ms)
            .map_err(Into::into)
    }

    pub fn cancel_runtime(
        &mut self,
        request: &CancelRequest,
    ) -> Result<CancelResponse, CompositionError> {
        self.require_connected()?;
        self.runtime.cancel(request).map_err(Into::into)
    }

    pub fn dispatch_shell(
        &mut self,
        request: &ShellRequest,
        attachments: Vec<ShellTerminalAttachment>,
        now_unix_ms: u64,
    ) -> Result<ShellResponse, CompositionError> {
        self.require_connected()?;
        let response = self
            .shell
            .dispatch(request, attachments, now_unix_ms)
            .map_err(CompositionError::from)?;
        match request.method {
            ShellMethod::Create if response.state == ShellState::Running => {
                self.known_shells.insert(request.resource_id.clone());
            }
            ShellMethod::Kill if response.state == ShellState::Exited => {
                self.known_shells.remove(&request.resource_id);
            }
            _ => {}
        }
        Ok(response)
    }

    pub fn adopt_shell(
        &mut self,
        recovered: RecoveredShell,
    ) -> Result<ShellState, CompositionError> {
        self.require_connected()?;
        let resource_id = recovered.scope.resource_id().to_owned();
        let state = self.shell.adopt(
            recovered.scope,
            &recovered.operation_id,
            recovered.output_ring_bytes,
        )?;
        self.known_shells.insert(resource_id);
        Ok(state)
    }

    pub fn start_tty(
        &mut self,
        request: &TtyOneShotRequest,
        spec: &TtyOneShotSpec,
        attachment: &AuthenticatedTerminalAttachment,
        fd: OwnedFd,
    ) -> Result<TransientUserScope, CompositionError> {
        self.require_connected()?;
        self.tty
            .start(request, spec, attachment, fd)
            .map_err(Into::into)
    }

    pub fn cancel_tty(
        &mut self,
        session_generation: u64,
        request_id: [u8; 16],
    ) -> Result<TtyCancelOutcome, CompositionError> {
        self.require_connected()?;
        self.tty
            .cancel(session_generation, request_id)
            .map_err(Into::into)
    }

    pub fn session_lost(&mut self) -> Result<(), CompositionError> {
        if self.state != RuntimeLifecycleState::Connected {
            return Err(CompositionError::SessionUnavailable);
        }
        self.state = RuntimeLifecycleState::Disconnected;
        let shell = self.shell.disconnect();
        let tty = self.tty.teardown_all();
        if shell.is_err() || tty.is_err() {
            self.state = RuntimeLifecycleState::Failed;
            return Err(CompositionError::TeardownFailed);
        }
        Ok(())
    }

    pub fn reconnect_unavailable(&mut self, now_unix_ms: u64) -> Result<(), CompositionError> {
        if self.state != RuntimeLifecycleState::Disconnected {
            return Err(CompositionError::InvalidLifecycle);
        }
        if let Err(error) = self.reconnect.record(now_unix_ms) {
            self.state = RuntimeLifecycleState::Failed;
            return Err(error);
        }
        Err(CompositionError::SessionUnavailable)
    }

    pub fn reconnect(
        &mut self,
        session: AuthenticatedRuntimeSession,
        resolver: R,
        wayland: W,
        recovered_shells: Vec<RecoveredShell>,
        now_unix_ms: u64,
    ) -> Result<(), CompositionError> {
        if self.state != RuntimeLifecycleState::Disconnected {
            return Err(CompositionError::InvalidLifecycle);
        }
        if session.uid != self.session.uid
            || session.process_uid != self.session.process_uid
            || session.realm_id != self.session.realm_id
            || session.workload_id != self.session.workload_id
            || session.generation <= self.session.generation
        {
            self.state = RuntimeLifecycleState::Failed;
            return Err(CompositionError::OwnerMismatch);
        }
        if let Err(error) = self.reconnect.record(now_unix_ms) {
            self.state = RuntimeLifecycleState::Failed;
            return Err(error);
        }

        let recovered_ids = recovered_shells
            .iter()
            .map(|shell| shell.scope.resource_id().to_owned())
            .collect::<BTreeSet<_>>();
        if recovered_ids.len() != recovered_shells.len() || recovered_ids != self.known_shells {
            self.state = RuntimeLifecycleState::Failed;
            return Err(CompositionError::RecoveryMismatch);
        }

        let runtime_owner = RuntimeOwner::admit(&RuntimeSession(&session))?;
        let shell_owner = ShellOwner::admit(&ShellSession(&session))?;
        let mut shell = ShellRuntimeService::new(
            shell_owner,
            self.backend.clone(),
            ShellStateStore::new(self.shell_output_budget)?,
        );
        for recovered in recovered_shells {
            if shell
                .adopt(
                    recovered.scope,
                    &recovered.operation_id,
                    recovered.output_ring_bytes,
                )
                .is_err()
            {
                self.state = RuntimeLifecycleState::Failed;
                return Err(CompositionError::RecoveryMismatch);
            }
        }

        self.runtime = RuntimeSystemdUserService::new(
            runtime_owner.clone(),
            resolver,
            wayland,
            self.backend.clone(),
        );
        self.shell = shell;
        self.tty = TtyOneShotService::new(runtime_owner, self.backend.clone());
        self.session = session;
        self.state = RuntimeLifecycleState::Connected;
        self.reconnect.reset();
        Ok(())
    }
}

impl<R, W, B> Drop for RuntimeComposition<R, W, B>
where
    B: SystemdUserRuntimePort + AuthenticatedSystemdUserRuntime + TtyOneShotRuntime,
{
    fn drop(&mut self) {
        let _ = self.shell.disconnect();
        let _ = self.tty.teardown_all();
        self.state = RuntimeLifecycleState::Failed;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionError {
    OwnerMismatch,
    SessionUnavailable,
    InvalidLifecycle,
    ReconnectLimit,
    RecoveryMismatch,
    TeardownFailed,
    Runtime(RuntimeServiceError),
    Shell(ShellServiceError),
    Tty(TtyOneShotError),
}

impl From<RuntimeServiceError> for CompositionError {
    fn from(error: RuntimeServiceError) -> Self {
        Self::Runtime(error)
    }
}

impl From<ShellServiceError> for CompositionError {
    fn from(error: ShellServiceError) -> Self {
        Self::Shell(error)
    }
}

impl From<TtyOneShotError> for CompositionError {
    fn from(error: TtyOneShotError) -> Self {
        Self::Tty(error)
    }
}

impl fmt::Display for CompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OwnerMismatch => "runtime-composition-owner-mismatch",
            Self::SessionUnavailable => "runtime-composition-session-unavailable",
            Self::InvalidLifecycle => "runtime-composition-lifecycle-invalid",
            Self::ReconnectLimit => "runtime-composition-reconnect-limit",
            Self::RecoveryMismatch => "runtime-composition-recovery-mismatch",
            Self::TeardownFailed => "runtime-composition-teardown-failed",
            Self::Runtime(_) => "runtime-composition-runtime-failed",
            Self::Shell(_) => "runtime-composition-shell-failed",
            Self::Tty(_) => "runtime-composition-tty-failed",
        })
    }
}

impl std::error::Error for CompositionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell_runtime::{ScopeInspection, ScopeOwnership, ScopeProcessState};
    use runtime_systemd_user::{
        CancelResult, DesiredState, ResolvedProcess, RuntimeProcessState, RuntimeResource,
        WaylandDisplayLease,
    };
    use std::fs::OpenOptions;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct BackendState {
        runtime_starts: usize,
        tty_starts: usize,
        tty_teardowns: usize,
    }

    #[derive(Clone, Default)]
    struct Backend {
        state: Arc<Mutex<BackendState>>,
    }

    impl Backend {
        fn shell_scope(owner: &ShellOwner, resource_id: &str) -> VerifiedTransientScope {
            VerifiedTransientScope::new(
                resource_id.to_owned(),
                format!("d2b-shell-{resource_id}.scope"),
                "scope-invocation".to_owned(),
                format!("/user.slice/{resource_id}"),
                owner.uid(),
                owner.session_generation(),
            )
            .unwrap()
        }

        fn runtime_resource(state: RuntimeProcessState) -> RuntimeResource {
            RuntimeResource {
                handle: "runtime-handle".to_owned(),
                result_digest: [7; 32],
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
            Ok(Self::runtime_resource(RuntimeProcessState::Present))
        }

        fn start_process(
            &mut self,
            _: &RuntimeOwner,
            _: &str,
            process: &ResolvedProcess,
            display: Option<&WaylandDisplayLease>,
        ) -> Result<RuntimeResource, RuntimeServiceError> {
            if process.graphical() != display.is_some() {
                return Err(RuntimeServiceError::WaylandUnavailable);
            }
            self.state.lock().unwrap().runtime_starts += 1;
            Ok(Self::runtime_resource(RuntimeProcessState::Running))
        }

        fn inspect_process(
            &mut self,
            _: &RuntimeOwner,
            _: &str,
        ) -> Result<RuntimeResource, RuntimeServiceError> {
            Ok(Self::runtime_resource(RuntimeProcessState::Running))
        }

        fn adopt_process(
            &mut self,
            _: &RuntimeOwner,
            _: &str,
            _: &str,
        ) -> Result<RuntimeResource, RuntimeServiceError> {
            Ok(Self::runtime_resource(RuntimeProcessState::Running))
        }

        fn stop_process(
            &mut self,
            _: &RuntimeOwner,
            _: &str,
            _: &str,
        ) -> Result<RuntimeResource, RuntimeServiceError> {
            Ok(Self::runtime_resource(RuntimeProcessState::Stopped))
        }

        fn open_terminal(
            &mut self,
            _: &RuntimeOwner,
            _: &str,
            _: &str,
            _: &AuthenticatedTerminalAttachment,
        ) -> Result<RuntimeResource, RuntimeServiceError> {
            Ok(Self::runtime_resource(RuntimeProcessState::Running))
        }

        fn cancel(&mut self, _: &RuntimeOwner, _: [u8; 16]) -> CancelResult {
            CancelResult::CancellationSignalled
        }
    }

    impl AuthenticatedSystemdUserRuntime for Backend {
        fn create_shell_scope(
            &mut self,
            owner: &ShellOwner,
            resource_id: &str,
            _: &str,
        ) -> Result<VerifiedTransientScope, ShellServiceError> {
            Ok(Self::shell_scope(owner, resource_id))
        }

        fn inspect_shell_scope(
            &mut self,
            _: &ShellOwner,
            _: &VerifiedTransientScope,
        ) -> Result<ScopeInspection, ShellServiceError> {
            Ok(ScopeInspection {
                ownership: ScopeOwnership::Exact,
                process_state: ScopeProcessState::Running,
            })
        }

        fn adopt_shell_scope(
            &mut self,
            _: &ShellOwner,
            _: &VerifiedTransientScope,
            _: &str,
        ) -> Result<ScopeInspection, ShellServiceError> {
            Ok(ScopeInspection {
                ownership: ScopeOwnership::Exact,
                process_state: ScopeProcessState::Running,
            })
        }

        fn kill_shell_scope(
            &mut self,
            _: &ShellOwner,
            _: &VerifiedTransientScope,
            _: &str,
        ) -> Result<ScopeInspection, ShellServiceError> {
            Ok(ScopeInspection {
                ownership: ScopeOwnership::Exact,
                process_state: ScopeProcessState::Exited,
            })
        }

        fn cancel(&mut self, _: &ShellOwner, _: [u8; 16]) -> ShellCancelOutcome {
            ShellCancelOutcome::CancellationSignalled
        }
    }

    impl TtyOneShotRuntime for Backend {
        fn start_transient_user_scope(
            &mut self,
            owner: &RuntimeOwner,
            request: &TtyOneShotRequest,
            _: &TtyOneShotSpec,
            _: ValidatedTerminal,
        ) -> Result<TransientUserScope, TtyOneShotError> {
            self.state.lock().unwrap().tty_starts += 1;
            let request_id = request.request_id();
            let hex = request_id
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let scope_name = format!("d2b-tty-{hex}.scope");
            TransientUserScope::new(
                owner,
                request_id,
                scope_name.clone(),
                "00112233445566778899aabbccddeeff".to_owned(),
                format!(
                    "/user.slice/user-{}.slice/user@{}.service/app.slice/{scope_name}",
                    owner.uid(),
                    owner.uid()
                ),
            )
        }

        fn teardown_transient_user_scope(
            &mut self,
            _: &RuntimeOwner,
            _: &TransientUserScope,
        ) -> Result<(), TtyOneShotError> {
            self.state.lock().unwrap().tty_teardowns += 1;
            Ok(())
        }
    }

    struct Resolver;

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
                vec!["/bin/tool".to_owned()],
                true,
            )
        }
    }

    #[derive(Clone)]
    struct Wayland(Arc<AtomicUsize>);

    impl WaylandControlPort for Wayland {
        fn open_display(
            &mut self,
            _: &RuntimeOwner,
            _: &ResolvedProcess,
            _: &str,
        ) -> Result<WaylandDisplayLease, RuntimeServiceError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            WaylandDisplayLease::new("display-handle".to_owned())
        }

        fn close_display(&mut self, _: WaylandDisplayLease) {}
    }

    fn session(generation: u64) -> AuthenticatedRuntimeSession {
        AuthenticatedRuntimeSession::for_current_process(
            generation,
            "local-root".to_owned(),
            "developer-tools".to_owned(),
        )
        .unwrap()
    }

    fn runtime_request(generation: u64) -> RuntimeRequest {
        RuntimeRequest {
            request_id: [1; 16],
            idempotency_key: Some([2; 32]),
            issued_at_unix_ms: 900,
            expires_at_unix_ms: 5_000,
            session_generation: generation,
            realm_id: "local-root".to_owned(),
            workload_id: "developer-tools".to_owned(),
            resource_id: "browser".to_owned(),
            operation_id: "start-browser".to_owned(),
            request_digest: Some([3; 32]),
            stream_id: String::new(),
            attachment_indexes: Vec::new(),
            desired_state: DesiredState::Running,
        }
    }

    fn shell_request(generation: u64, method: ShellMethod) -> ShellRequest {
        ShellRequest {
            method,
            request_id: [4; 16],
            idempotency_key: method.mutating().then_some([5; 32]),
            issued_at_unix_ms: 900,
            expires_at_unix_ms: 5_000,
            session_generation: generation,
            realm_id: "local-root".to_owned(),
            workload_id: "developer-tools".to_owned(),
            resource_id: "primary-shell".to_owned(),
            operation_id: if method.mutating() {
                "shell-operation"
            } else {
                ""
            }
            .to_owned(),
            stream_id: String::new(),
            attachment_indexes: Vec::new(),
            output_ring_bytes: if method == ShellMethod::Create {
                4096
            } else {
                0
            },
        }
    }

    #[test]
    fn exact_uid_session_is_the_only_admitted_owner() {
        if nix::unistd::getuid().is_root() {
            return;
        }
        let current = nix::unistd::getuid().as_raw();
        let admitted = session(1);
        assert_eq!(admitted.uid(), current);
        assert!(!format!("{admitted:?}").contains(&current.to_string()));
        assert_eq!(
            AuthenticatedRuntimeSession::admit(
                current,
                current.saturating_add(1),
                1,
                "local-root".to_owned(),
                "developer-tools".to_owned(),
            ),
            Err(CompositionError::OwnerMismatch)
        );
    }

    #[test]
    fn services_share_one_owner_backend_and_fail_closed_on_session_loss() {
        if nix::unistd::getuid().is_root() {
            return;
        }
        let backend = Backend::default();
        let state = Arc::clone(&backend.state);
        let wayland_calls = Arc::new(AtomicUsize::new(0));
        let mut runtime = RuntimeComposition::new(
            session(1),
            Resolver,
            Wayland(Arc::clone(&wayland_calls)),
            backend,
            64 * 1024,
        )
        .unwrap();

        runtime
            .dispatch_runtime(RuntimeMethod::StartProcess, &runtime_request(1), &[], 1_000)
            .unwrap();
        runtime
            .dispatch_shell(&shell_request(1, ShellMethod::Create), Vec::new(), 1_000)
            .unwrap();

        let owner = RuntimeOwner::admit(&RuntimeSession(runtime.session())).unwrap();
        let tty_request = TtyOneShotRequest::new(
            [8; 16],
            1,
            "terminal".to_owned(),
            "open-terminal".to_owned(),
        )
        .unwrap();
        let tty_spec = TtyOneShotSpec::new(
            &owner,
            24,
            80,
            vec!["/bin/sh".to_owned()],
            vec!["PATH=/bin".to_owned(), "TERM=xterm".to_owned()],
        )
        .unwrap();
        let attachment = AuthenticatedTerminalAttachment {
            index: runtime_systemd_user::TERMINAL_ATTACHMENT_INDEX,
            owner_uid: owner.uid(),
            session_generation: owner.session_generation(),
            request_id: tty_request.request_id(),
            connected_stream: true,
            cloexec: true,
        };
        let terminal: OwnedFd = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/ptmx")
            .unwrap()
            .into();
        runtime
            .start_tty(&tty_request, &tty_spec, &attachment, terminal)
            .unwrap();

        assert_eq!(wayland_calls.load(Ordering::Relaxed), 1);
        assert_eq!(state.lock().unwrap().runtime_starts, 1);
        assert_eq!(state.lock().unwrap().tty_starts, 1);

        runtime.session_lost().unwrap();
        assert_eq!(runtime.state(), RuntimeLifecycleState::Disconnected);
        assert_eq!(state.lock().unwrap().tty_teardowns, 1);
        assert_eq!(
            runtime
                .dispatch_runtime(RuntimeMethod::StartProcess, &runtime_request(1), &[], 1_000,)
                .unwrap_err(),
            CompositionError::SessionUnavailable
        );

        let new_session = session(2);
        let shell_owner = ShellOwner::admit(&ShellSession(&new_session)).unwrap();
        runtime
            .reconnect(
                new_session,
                Resolver,
                Wayland(wayland_calls),
                vec![RecoveredShell {
                    scope: Backend::shell_scope(&shell_owner, "primary-shell"),
                    operation_id: "adopt-shell".to_owned(),
                    output_ring_bytes: 4096,
                }],
                2_000,
            )
            .unwrap();
        assert_eq!(runtime.state(), RuntimeLifecycleState::Connected);
        assert_eq!(
            runtime
                .dispatch_shell(&shell_request(2, ShellMethod::Inspect), Vec::new(), 2_000,)
                .unwrap()
                .state,
            ShellState::Running
        );
    }

    #[test]
    fn reconnect_requires_complete_recovery_and_same_principal() {
        if nix::unistd::getuid().is_root() {
            return;
        }
        let mut runtime = RuntimeComposition::new(
            session(1),
            Resolver,
            Wayland(Arc::new(AtomicUsize::new(0))),
            Backend::default(),
            64 * 1024,
        )
        .unwrap();
        runtime
            .dispatch_shell(&shell_request(1, ShellMethod::Create), Vec::new(), 1_000)
            .unwrap();
        runtime.session_lost().unwrap();
        assert_eq!(
            runtime
                .reconnect(
                    session(2),
                    Resolver,
                    Wayland(Arc::new(AtomicUsize::new(0))),
                    Vec::new(),
                    2_000,
                )
                .unwrap_err(),
            CompositionError::RecoveryMismatch
        );
        assert_eq!(runtime.state(), RuntimeLifecycleState::Failed);
    }

    #[test]
    fn unavailable_reconnects_are_bounded_and_then_fail_terminally() {
        if nix::unistd::getuid().is_root() {
            return;
        }
        let mut runtime = RuntimeComposition::new(
            session(1),
            Resolver,
            Wayland(Arc::new(AtomicUsize::new(0))),
            Backend::default(),
            64 * 1024,
        )
        .unwrap();
        runtime.session_lost().unwrap();
        for attempt in 0..MAX_RECONNECT_ATTEMPTS {
            assert_eq!(
                runtime.reconnect_unavailable(u64::from(attempt)),
                Err(CompositionError::SessionUnavailable)
            );
        }
        assert_eq!(
            runtime.reconnect_unavailable(u64::from(MAX_RECONNECT_ATTEMPTS)),
            Err(CompositionError::ReconnectLimit)
        );
        assert_eq!(runtime.state(), RuntimeLifecycleState::Failed);
    }
}
