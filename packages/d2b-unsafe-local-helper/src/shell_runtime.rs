use crate::runtime::{
    LaunchReservation, PersistedScope, PersistedShellMetadata, RuntimeError, ScopeRuntime,
    ShellOperationBegin, persist_ledger,
};
use crate::shell_socket::{
    connect_owned_stream, supervisor_socket_path, validate_runtime_directory,
};
use crate::shell_supervisor::{
    BlockedShellSupervisor, DEFAULT_SHELL_OUTPUT_RING_BYTES, MAX_HELPER_SHELL_OUTPUT_BYTES,
    ShellSupervisorError, ShellSupervisorSpec,
};
use crate::supervisor_protocol::{
    SUPERVISOR_PROTOCOL_VERSION, SupervisorAction, SupervisorFailure, SupervisorRequest,
    SupervisorResponse, SupervisorResult, read_frame, write_frame,
};
use crate::systemd::{ScopeInspection, UserScopeManager};
use d2b_contracts::public_wire::{
    ShellCloseCause, ShellDetachResult, ShellKillResult, ShellListEntry, ShellListResult,
    ShellName, ShellSessionState,
};
use d2b_contracts::unsafe_local_wire::{
    HelperPersistentShellSnapshot, HelperScopeKind, HelperScopeState, HelperShellAttachResult,
    HelperShellDetachResponse, HelperShellKillResponse, HelperShellListResponse, HelperShellPolicy,
    HelperShellRequest, HelperShellResponse, HelperSupervisorId, HelperTerminalReady,
    HelperTerminalTransport, UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION, UnsafeLocalHelperToDaemon,
};
use d2b_core::workload_identity::WorkloadIdentity;
use d2b_realm_core::ids::OperationId;
use nix::libc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};
use uzers::get_user_by_uid;
use uzers::os::unix::UserExt;

const SUPERVISOR_CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const SHELL_CLOSE_GRACE: Duration = Duration::from_millis(250);
const SCOPE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const SHELL_LIST_RECONCILE_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ShellDispatch {
    pub(crate) response: UnsafeLocalHelperToDaemon,
    pub(crate) terminal_fd: Option<OwnedFd>,
}

fn collect_verified_scope<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    scope: &crate::systemd::VerifiedScope,
) -> Result<(), RuntimeError> {
    if !wait_for_scope_exit(runtime, scope, SHELL_CLOSE_GRACE)? {
        runtime.manager.terminate_scope(scope, libc::SIGTERM)?;
        if !wait_for_scope_exit(runtime, scope, SHELL_CLOSE_GRACE)? {
            runtime.manager.terminate_scope(scope, libc::SIGKILL)?;
            let _ = wait_for_scope_exit(runtime, scope, SHELL_CLOSE_GRACE)?;
        }
    }
    match runtime.manager.inspect_scope(scope) {
        Ok(inspection)
            if inspection.identity_matches && inspection.state != HelperScopeState::Exited =>
        {
            runtime.manager.stop_scope(scope)?;
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

impl std::fmt::Debug for ShellDispatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellDispatch")
            .field("response", &self.response)
            .field("has_terminal_fd", &self.terminal_fd.is_some())
            .finish()
    }
}

impl ShellDispatch {
    fn shell(response: HelperShellResponse) -> Self {
        Self {
            response: UnsafeLocalHelperToDaemon::Shell(response),
            terminal_fd: None,
        }
    }

    fn terminal(response: HelperTerminalReady, stream: UnixStream) -> Self {
        Self {
            response: UnsafeLocalHelperToDaemon::TerminalReady(response),
            terminal_fd: Some(stream.into()),
        }
    }

    pub fn into_parts(self) -> (UnsafeLocalHelperToDaemon, Option<OwnedFd>) {
        (self.response, self.terminal_fd)
    }
}

pub(crate) fn dispatch<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    request: HelperShellRequest,
) -> Result<ShellDispatch, RuntimeError> {
    request
        .validate_bounds()
        .map_err(|_| RuntimeError::InvalidRequest)?;
    let fingerprint = shell_fingerprint(&request)?;
    let operation_id = request.operation_id().clone();
    let begin = runtime
        .ledger
        .lock()
        .map_err(|_| RuntimeError::Internal)?
        .begin_shell_operation(&operation_id, fingerprint)?;
    match begin {
        ShellOperationBegin::Replayed(Some(response)) => Ok(ShellDispatch::shell(
            recorrelate_response(response, request.request_id(), operation_id),
        )),
        ShellOperationBegin::Replayed(None) => replay_attach(runtime, request),
        ShellOperationBegin::ExistingScope(scope) => {
            replay_persisted_attach(runtime, request, *scope)
        }
        ShellOperationBegin::Started(reservation) => {
            let result = dispatch_started(runtime, request, reservation);
            if result.is_err()
                && let Ok(mut ledger) = runtime.ledger.lock()
            {
                ledger.clear_shell_operation(&operation_id, reservation, None);
            }
            result
        }
    }
}

fn recorrelate_response(
    mut response: HelperShellResponse,
    request_id: u64,
    operation_id: OperationId,
) -> HelperShellResponse {
    match &mut response {
        HelperShellResponse::List(value) => {
            value.request_id = request_id;
            value.operation_id = operation_id;
        }
        HelperShellResponse::Detach(value) => {
            value.request_id = request_id;
            value.operation_id = operation_id;
        }
        HelperShellResponse::Kill(value) => {
            value.request_id = request_id;
            value.operation_id = operation_id;
        }
    }
    response
}

fn dispatch_started<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    request: HelperShellRequest,
    reservation: LaunchReservation,
) -> Result<ShellDispatch, RuntimeError> {
    match request {
        HelperShellRequest::List {
            request_id,
            operation_id,
            workload,
            policy,
        } => list(
            runtime,
            request_id,
            operation_id,
            workload,
            policy,
            reservation,
        ),
        HelperShellRequest::Attach {
            request_id,
            operation_id,
            workload,
            policy,
            name,
            force,
            initial_terminal_size,
        } => attach(
            runtime,
            AttachOperation {
                request_id,
                operation_id,
                workload,
                policy,
                name,
                force,
                rows: initial_terminal_size.rows,
                cols: initial_terminal_size.cols,
            },
            reservation,
        ),
        HelperShellRequest::Detach {
            request_id,
            operation_id,
            workload,
            policy: _,
            name,
        } => detach(
            runtime,
            request_id,
            operation_id,
            workload,
            name,
            reservation,
        ),
        HelperShellRequest::Kill {
            request_id,
            operation_id,
            workload,
            policy: _,
            name,
        } => kill(
            runtime,
            request_id,
            operation_id,
            workload,
            name,
            reservation,
        ),
    }
}

#[derive(Clone)]
struct AttachOperation {
    request_id: u64,
    operation_id: OperationId,
    workload: WorkloadIdentity,
    policy: HelperShellPolicy,
    name: Option<ShellName>,
    force: bool,
    rows: u32,
    cols: u32,
}

impl std::fmt::Debug for AttachOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachOperation")
            .field("request_id", &self.request_id)
            .field("operation_id", &self.operation_id)
            .field("workload", &self.workload)
            .field("policy", &self.policy)
            .field("has_name", &self.name.is_some())
            .field("force", &self.force)
            .field("rows", &self.rows)
            .field("cols", &self.cols)
            .finish()
    }
}

fn list<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    request_id: u64,
    operation_id: OperationId,
    workload: WorkloadIdentity,
    policy: HelperShellPolicy,
    reservation: LaunchReservation,
) -> Result<ShellDispatch, RuntimeError> {
    let entries = runtime
        .ledger
        .lock()
        .map_err(|_| RuntimeError::Internal)?
        .persisted
        .scopes
        .iter()
        .filter(|scope| {
            same_workload(&scope.workload, &workload) && scope.persistent_shell.is_some()
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut sessions = Vec::with_capacity(entries.len());
    let deadline = Instant::now() + SHELL_LIST_RECONCILE_TIMEOUT;
    for entry in entries {
        let metadata = entry
            .persistent_shell
            .as_ref()
            .ok_or(RuntimeError::Internal)?;
        let state = if Instant::now() >= deadline {
            ShellSessionState::PoolUnavailable
        } else {
            inspect_shell(runtime, &entry)
                .map(|status| shell_state(status.running, status.attached))
                .unwrap_or(ShellSessionState::PoolUnavailable)
        };
        sessions.push(ShellListEntry {
            name: metadata.name.clone(),
            state,
            attached: state == ShellSessionState::Attached,
            is_default: metadata.name == policy.default_name,
        });
    }
    sessions.sort_by(|left, right| left.name.cmp(&right.name));
    let response = HelperShellResponse::List(HelperShellListResponse {
        request_id,
        operation_id: operation_id.clone(),
        result: ShellListResult {
            default_name: policy.default_name,
            sessions,
        },
    });
    runtime
        .ledger
        .lock()
        .map_err(|_| RuntimeError::Internal)?
        .complete_shell_operation(&operation_id, reservation, None, Some(response.clone()))?;
    Ok(ShellDispatch::shell(response))
}

fn attach<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    operation: AttachOperation,
    reservation: LaunchReservation,
) -> Result<ShellDispatch, RuntimeError> {
    let resolved_name = operation
        .name
        .clone()
        .unwrap_or_else(|| operation.policy.default_name.clone());
    let name_key = shell_name_key(&operation.workload, &resolved_name);
    let existing = {
        let mut ledger = runtime.ledger.lock().map_err(|_| RuntimeError::Internal)?;
        if let Some(scope) = find_shell(
            &ledger.persisted.scopes,
            &operation.workload,
            &resolved_name,
        ) {
            Some(scope.clone())
        } else {
            let workload_count = ledger
                .persisted
                .scopes
                .iter()
                .filter(|scope| {
                    same_workload(&scope.workload, &operation.workload)
                        && scope.persistent_shell.is_some()
                })
                .count();
            let workload_prefix = format!("{}\u{1f}", operation.workload.target().to_canonical());
            let workload_reserved = ledger
                .shell_name_reservations
                .keys()
                .filter(|key| key.starts_with(&workload_prefix))
                .count();
            let global_shells = ledger
                .persisted
                .scopes
                .iter()
                .filter(|scope| scope.persistent_shell.is_some())
                .count()
                .saturating_add(ledger.shell_name_reservations.len());
            if !shell_quota_allows(
                workload_count,
                workload_reserved,
                operation.policy.max_sessions,
                global_shells,
            ) {
                return Err(RuntimeError::QuotaExceeded);
            }
            ledger.reserve_shell_name(name_key.clone(), reservation)?;
            None
        }
    };

    if let Some(scope) = existing {
        let stream = attach_existing(
            runtime,
            &scope,
            operation.force,
            operation.rows,
            operation.cols,
        )?;
        let force_evicted = stream.force_evicted;
        let terminal = terminal_ready(
            operation.request_id,
            operation.operation_id.clone(),
            &scope,
            resolved_name,
            force_evicted,
        );
        runtime
            .ledger
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .complete_shell_operation(&operation.operation_id, reservation, None, None)?;
        return Ok(ShellDispatch::terminal(terminal, stream.stream));
    }

    let result = create_and_attach(runtime, &operation, resolved_name, &name_key, reservation);
    if result.is_err()
        && let Ok(mut ledger) = runtime.ledger.lock()
    {
        ledger.clear_shell_operation(&operation.operation_id, reservation, Some(&name_key));
    }
    result
}

fn create_and_attach<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    operation: &AttachOperation,
    resolved_name: ShellName,
    name_key: &str,
    reservation: LaunchReservation,
) -> Result<ShellDispatch, RuntimeError> {
    let environment = runtime.manager.manager_environment()?;
    let runtime_directory = environment.runtime_directory()?;
    validate_runtime_directory(&runtime_directory, runtime.uid)
        .map_err(|_| RuntimeError::EnvironmentInvalid)?;
    let user = get_user_by_uid(runtime.uid).ok_or(RuntimeError::InvalidIdentity)?;
    if user.home_dir() != runtime.shell_home || !user.shell().is_absolute() {
        return Err(RuntimeError::InvalidIdentity);
    }
    let child_environment = environment.child_entries(false, None)?;
    let supervisor_id = random_supervisor_id()?;
    supervisor_socket_path(&runtime_directory, &supervisor_id)
        .map_err(|_| RuntimeError::EnvironmentInvalid)?;
    let spec = ShellSupervisorSpec {
        supervisor_id: supervisor_id.clone(),
        runtime_directory,
        environment: child_environment,
        cwd: runtime.shell_home.clone(),
        initial_rows: u16::try_from(operation.rows).map_err(|_| RuntimeError::InvalidRequest)?,
        initial_cols: u16::try_from(operation.cols).map_err(|_| RuntimeError::InvalidRequest)?,
        output_ring_bytes: DEFAULT_SHELL_OUTPUT_RING_BYTES,
    };
    let mut supervisor =
        BlockedShellSupervisor::spawn(&runtime.executable, &spec).map_err(map_supervisor_error)?;
    drop(spec);
    let scope = match runtime
        .manager
        .start_scope(supervisor.id(), HelperScopeKind::PersistentShell)
    {
        Ok(scope) => scope,
        Err(error) => {
            supervisor.abort();
            return Err(error.into());
        }
    };
    if let Err(error) = supervisor.release_and_wait_ready() {
        supervisor.abort();
        runtime.stop_failed_scope(&scope);
        return Err(map_supervisor_error(error));
    }

    let persisted = PersistedScope {
        operation_id: operation.operation_id.clone(),
        fingerprint: Some(reservation.fingerprint),
        workload: operation.workload.clone(),
        unit_name: scope.unit_name.clone(),
        invocation_id: scope.invocation_id.clone(),
        control_group: scope.control_group.clone(),
        kind: scope.kind,
        persistent_shell: Some(PersistedShellMetadata {
            name: resolved_name.clone(),
            supervisor_id,
        }),
    };
    if inspect_shell(runtime, &persisted).is_err() {
        supervisor.abort();
        runtime.stop_failed_scope(&scope);
        return Err(RuntimeError::ShellUnavailable);
    }
    if let Err(error) = commit_shell_scope(runtime, &persisted, reservation, name_key) {
        supervisor.abort();
        runtime.stop_failed_scope(&scope);
        return Err(error);
    }
    supervisor.reap_in_background();

    let attached = match attach_existing(
        runtime,
        &persisted,
        operation.force,
        operation.rows,
        operation.cols,
    ) {
        Ok(attached) => attached,
        Err(error) => {
            cleanup_created_shell(runtime, &persisted);
            return Err(error);
        }
    };
    let terminal = terminal_ready(
        operation.request_id,
        operation.operation_id.clone(),
        &persisted,
        resolved_name,
        attached.force_evicted,
    );
    Ok(ShellDispatch::terminal(terminal, attached.stream))
}

fn detach<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    request_id: u64,
    operation_id: OperationId,
    workload: WorkloadIdentity,
    name: ShellName,
    reservation: LaunchReservation,
) -> Result<ShellDispatch, RuntimeError> {
    let scope = persisted_shell(runtime, &workload, &name)?;
    verify_scope(runtime, &scope)?;
    let result = supervisor_action(runtime, &scope, SupervisorAction::Detach)?;
    let detached = match result.result {
        SupervisorResult::Detached { detached } => detached,
        SupervisorResult::Rejected { code } => return Err(map_supervisor_failure(code)),
        _ => return Err(RuntimeError::ShellUnavailable),
    };
    let response = HelperShellResponse::Detach(HelperShellDetachResponse {
        request_id,
        operation_id: operation_id.clone(),
        result: ShellDetachResult {
            resolved_name: name,
            detached,
            cause: detached.then_some(ShellCloseCause::EvictedByAdminDetach),
        },
    });
    runtime
        .ledger
        .lock()
        .map_err(|_| RuntimeError::Internal)?
        .complete_shell_operation(&operation_id, reservation, None, Some(response.clone()))?;
    Ok(ShellDispatch::shell(response))
}

fn kill<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    request_id: u64,
    operation_id: OperationId,
    workload: WorkloadIdentity,
    name: ShellName,
    reservation: LaunchReservation,
) -> Result<ShellDispatch, RuntimeError> {
    let scope = persisted_shell(runtime, &workload, &name)?;
    verify_scope(runtime, &scope)?;
    let kill_acknowledged = supervisor_action(runtime, &scope, SupervisorAction::Kill)
        .map(|response| matches!(response.result, SupervisorResult::KillAccepted))
        .unwrap_or(false);
    if !kill_acknowledged {
        return Err(RuntimeError::ShellUnavailable);
    }

    let verified = scope.verified();
    collect_verified_scope(runtime, &verified)?;
    remove_shell_scope(runtime, &scope)?;

    let response = HelperShellResponse::Kill(HelperShellKillResponse {
        request_id,
        operation_id: operation_id.clone(),
        result: ShellKillResult {
            name,
            killed: true,
            state: ShellSessionState::Killed,
        },
    });
    runtime
        .ledger
        .lock()
        .map_err(|_| RuntimeError::Internal)?
        .complete_shell_operation(&operation_id, reservation, None, Some(response.clone()))?;
    Ok(ShellDispatch::shell(response))
}

fn replay_attach<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    request: HelperShellRequest,
) -> Result<ShellDispatch, RuntimeError> {
    let HelperShellRequest::Attach {
        request_id,
        operation_id,
        workload,
        policy,
        name,
        initial_terminal_size,
        ..
    } = request
    else {
        return Err(RuntimeError::OperationIdConflict);
    };
    let name = name.unwrap_or(policy.default_name);
    let scope = persisted_shell(runtime, &workload, &name)?;
    let attached = attach_existing(
        runtime,
        &scope,
        true,
        initial_terminal_size.rows,
        initial_terminal_size.cols,
    )?;
    let ready = terminal_ready(
        request_id,
        operation_id,
        &scope,
        name,
        attached.force_evicted,
    );
    Ok(ShellDispatch::terminal(ready, attached.stream))
}

fn replay_persisted_attach<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    request: HelperShellRequest,
    scope: PersistedScope,
) -> Result<ShellDispatch, RuntimeError> {
    let HelperShellRequest::Attach {
        request_id,
        operation_id,
        initial_terminal_size,
        ..
    } = request
    else {
        return Err(RuntimeError::OperationIdConflict);
    };
    let metadata = scope
        .persistent_shell
        .as_ref()
        .ok_or(RuntimeError::OperationIdConflict)?;
    let attached = attach_existing(
        runtime,
        &scope,
        true,
        initial_terminal_size.rows,
        initial_terminal_size.cols,
    )?;
    let ready = terminal_ready(
        request_id,
        operation_id,
        &scope,
        metadata.name.clone(),
        attached.force_evicted,
    );
    Ok(ShellDispatch::terminal(ready, attached.stream))
}

struct AttachedStream {
    stream: UnixStream,
    force_evicted: bool,
}

fn attach_existing<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    scope: &PersistedScope,
    force: bool,
    rows: u32,
    cols: u32,
) -> Result<AttachedStream, RuntimeError> {
    verify_scope(runtime, scope)?;
    let action = SupervisorAction::Attach {
        force,
        initial_terminal_size: d2b_contracts::terminal_wire::TerminalSize { rows, cols },
    };
    let (response, stream) = supervisor_action_with_stream(runtime, scope, action)?;
    match response.result {
        SupervisorResult::Attached { force_evicted } => {
            stream
                .set_read_timeout(None)
                .and_then(|()| stream.set_write_timeout(None))
                .map_err(|_| RuntimeError::ShellUnavailable)?;
            Ok(AttachedStream {
                stream,
                force_evicted,
            })
        }
        SupervisorResult::Rejected {
            code: SupervisorFailure::AlreadyAttached,
        } => Err(RuntimeError::ShellAlreadyAttached),
        SupervisorResult::Rejected { code } => Err(map_supervisor_failure(code)),
        _ => Err(RuntimeError::ShellUnavailable),
    }
}

struct SupervisorStatus {
    running: bool,
    attached: bool,
}

fn inspect_shell<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    scope: &PersistedScope,
) -> Result<SupervisorStatus, RuntimeError> {
    verify_scope(runtime, scope)?;
    let response = supervisor_action(runtime, scope, SupervisorAction::Status)?;
    match response.result {
        SupervisorResult::Status {
            running, attached, ..
        } => Ok(SupervisorStatus { running, attached }),
        _ => Err(RuntimeError::ShellUnavailable),
    }
}

pub(crate) fn snapshot_shell<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    scope: &PersistedScope,
    manager_state: HelperScopeState,
) -> (HelperScopeState, Option<HelperPersistentShellSnapshot>) {
    let Some(metadata) = scope.persistent_shell.as_ref() else {
        return (manager_state, None);
    };
    let (state, shell_state, attached) = if manager_state == HelperScopeState::Degraded {
        (
            HelperScopeState::Degraded,
            ShellSessionState::PoolUnavailable,
            false,
        )
    } else {
        match inspect_shell(runtime, scope) {
            Ok(status) => (
                manager_state,
                shell_state(status.running, status.attached),
                status.attached,
            ),
            Err(_) => (
                HelperScopeState::Degraded,
                ShellSessionState::PoolUnavailable,
                false,
            ),
        }
    };
    (
        state,
        Some(HelperPersistentShellSnapshot {
            name: metadata.name.clone(),
            state: shell_state,
            attached,
            supervisor_id: metadata.supervisor_id.clone(),
        }),
    )
}

fn verify_scope<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    scope: &PersistedScope,
) -> Result<ScopeInspection, RuntimeError> {
    let inspection = runtime.manager.inspect_scope(&scope.verified())?;
    if !inspection.identity_matches
        || !matches!(
            inspection.state,
            HelperScopeState::Starting | HelperScopeState::Active
        )
    {
        return Err(RuntimeError::ScopeIdentityMismatch);
    }
    Ok(inspection)
}

fn supervisor_action<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    scope: &PersistedScope,
    action: SupervisorAction,
) -> Result<SupervisorResponse, RuntimeError> {
    supervisor_action_with_stream(runtime, scope, action).map(|(response, _)| response)
}

fn supervisor_action_with_stream<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    scope: &PersistedScope,
    action: SupervisorAction,
) -> Result<(SupervisorResponse, UnixStream), RuntimeError> {
    let metadata = scope
        .persistent_shell
        .as_ref()
        .ok_or(RuntimeError::ShellUnavailable)?;
    let environment = runtime.manager.manager_environment()?;
    let runtime_directory = environment.runtime_directory()?;
    let mut stream = connect_owned_stream(&runtime_directory, &metadata.supervisor_id, runtime.uid)
        .map_err(|_| RuntimeError::ShellUnavailable)?;
    stream
        .set_read_timeout(Some(SUPERVISOR_CONTROL_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(SUPERVISOR_CONTROL_TIMEOUT)))
        .map_err(|_| RuntimeError::ShellUnavailable)?;
    let request_id = random_request_id()?;
    write_frame(
        &mut stream,
        &SupervisorRequest {
            version: SUPERVISOR_PROTOCOL_VERSION,
            request_id,
            action,
        },
    )
    .map_err(|_| RuntimeError::ShellUnavailable)?;
    let response: SupervisorResponse =
        read_frame(&mut stream).map_err(|_| RuntimeError::ShellUnavailable)?;
    if response.version != SUPERVISOR_PROTOCOL_VERSION || response.request_id != request_id {
        return Err(RuntimeError::ShellUnavailable);
    }
    Ok((response, stream))
}

fn wait_for_scope_exit<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    scope: &crate::systemd::VerifiedScope,
    timeout: Duration,
) -> Result<bool, RuntimeError> {
    let deadline = Instant::now() + timeout;
    loop {
        match runtime.manager.inspect_scope(scope) {
            Ok(inspection) if inspection.identity_matches => {
                if inspection.state == HelperScopeState::Exited {
                    return Ok(true);
                }
            }
            Ok(_) => return Ok(true),
            Err(error) => return Err(error.into()),
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(SCOPE_POLL_INTERVAL);
    }
}

fn commit_shell_scope<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    scope: &PersistedScope,
    reservation: LaunchReservation,
    name_key: &str,
) -> Result<(), RuntimeError> {
    let mut ledger = runtime.ledger.lock().map_err(|_| RuntimeError::Internal)?;
    if ledger.shell_name_reservations.get(name_key).copied() != Some(reservation.owner)
        || ledger
            .reservations
            .get(scope.operation_id.as_str())
            .is_none_or(|active| active.owner != reservation.owner)
    {
        return Err(RuntimeError::OperationIdConflict);
    }
    let mut candidate = ledger.persisted.clone();
    candidate.scopes.push(scope.clone());
    persist_ledger(&runtime.ledger_path, &candidate)?;
    ledger.persisted = candidate;
    ledger.clear_shell_operation(&scope.operation_id, reservation, Some(name_key));
    Ok(())
}

fn cleanup_created_shell<M: UserScopeManager>(runtime: &ScopeRuntime<M>, scope: &PersistedScope) {
    let cleaned = (|| {
        verify_scope(runtime, scope)?;
        let response = supervisor_action(runtime, scope, SupervisorAction::Kill)?;
        if !matches!(response.result, SupervisorResult::KillAccepted) {
            return Err(RuntimeError::ShellUnavailable);
        }
        let verified = scope.verified();
        collect_verified_scope(runtime, &verified)
    })();
    if cleaned.is_ok() {
        let _ = remove_shell_scope(runtime, scope);
    }
}

fn remove_shell_scope<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    scope: &PersistedScope,
) -> Result<(), RuntimeError> {
    let mut ledger = runtime.ledger.lock().map_err(|_| RuntimeError::Internal)?;
    let mut candidate = ledger.persisted.clone();
    candidate.scopes.retain(|candidate| {
        !(candidate.operation_id == scope.operation_id
            && candidate.invocation_id == scope.invocation_id
            && candidate.kind == HelperScopeKind::PersistentShell)
    });
    if candidate.scopes.len() == ledger.persisted.scopes.len() {
        return Err(RuntimeError::ShellNotFound);
    }
    persist_ledger(&runtime.ledger_path, &candidate)?;
    ledger.persisted = candidate;
    if let Some(fingerprint) = scope.fingerprint {
        ledger.remember_completed_shell_operation(&scope.operation_id, fingerprint, None);
    }
    Ok(())
}

fn persisted_shell<M: UserScopeManager>(
    runtime: &ScopeRuntime<M>,
    workload: &WorkloadIdentity,
    name: &ShellName,
) -> Result<PersistedScope, RuntimeError> {
    runtime
        .ledger
        .lock()
        .map_err(|_| RuntimeError::Internal)?
        .persisted
        .scopes
        .iter()
        .find(|scope| {
            same_workload(&scope.workload, workload)
                && scope
                    .persistent_shell
                    .as_ref()
                    .is_some_and(|shell| shell.name == *name)
        })
        .cloned()
        .ok_or(RuntimeError::ShellNotFound)
}

fn find_shell<'a>(
    scopes: &'a [PersistedScope],
    workload: &WorkloadIdentity,
    name: &ShellName,
) -> Option<&'a PersistedScope> {
    scopes.iter().find(|scope| {
        same_workload(&scope.workload, workload)
            && scope
                .persistent_shell
                .as_ref()
                .is_some_and(|shell| shell.name == *name)
    })
}

fn shell_name_key(workload: &WorkloadIdentity, name: &ShellName) -> String {
    format!(
        "{}\u{1f}{}",
        workload.target().to_canonical(),
        name.as_str()
    )
}

fn same_workload(left: &WorkloadIdentity, right: &WorkloadIdentity) -> bool {
    left.target() == right.target()
}

fn shell_state(running: bool, attached: bool) -> ShellSessionState {
    if !running {
        ShellSessionState::Killed
    } else if attached {
        ShellSessionState::Attached
    } else {
        ShellSessionState::Detached
    }
}

fn shell_quota_allows(
    workload_shells: usize,
    workload_reservations: usize,
    max_sessions: u16,
    global_shells_and_reservations: usize,
) -> bool {
    workload_shells.saturating_add(workload_reservations) < max_sessions as usize
        && global_shells_and_reservations.saturating_mul(DEFAULT_SHELL_OUTPUT_RING_BYTES)
            < MAX_HELPER_SHELL_OUTPUT_BYTES
}

fn terminal_ready(
    request_id: u64,
    operation_id: OperationId,
    scope: &PersistedScope,
    resolved_name: ShellName,
    force_evicted: bool,
) -> HelperTerminalReady {
    HelperTerminalReady {
        request_id,
        operation_id,
        terminal_protocol_version: UNSAFE_LOCAL_TERMINAL_PROTOCOL_VERSION,
        transport: HelperTerminalTransport::ConnectedUnixStream,
        scope: scope.verified().wire_identity(),
        result: HelperShellAttachResult {
            resolved_name,
            state: ShellSessionState::Attached,
            force_evicted,
        },
    }
}

fn shell_fingerprint(request: &HelperShellRequest) -> Result<[u8; 32], RuntimeError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    enum Fingerprint<'a> {
        List {
            workload: &'a WorkloadIdentity,
            policy: &'a HelperShellPolicy,
        },
        Attach {
            workload: &'a WorkloadIdentity,
            policy: &'a HelperShellPolicy,
            name: &'a Option<ShellName>,
            force: bool,
            rows: u32,
            cols: u32,
        },
        Detach {
            workload: &'a WorkloadIdentity,
            policy: &'a HelperShellPolicy,
            name: &'a ShellName,
        },
        Kill {
            workload: &'a WorkloadIdentity,
            policy: &'a HelperShellPolicy,
            name: &'a ShellName,
        },
    }
    let fingerprint = match request {
        HelperShellRequest::List {
            workload, policy, ..
        } => Fingerprint::List { workload, policy },
        HelperShellRequest::Attach {
            workload,
            policy,
            name,
            force,
            initial_terminal_size,
            ..
        } => Fingerprint::Attach {
            workload,
            policy,
            name,
            force: *force,
            rows: initial_terminal_size.rows,
            cols: initial_terminal_size.cols,
        },
        HelperShellRequest::Detach {
            workload,
            policy,
            name,
            ..
        } => Fingerprint::Detach {
            workload,
            policy,
            name,
        },
        HelperShellRequest::Kill {
            workload,
            policy,
            name,
            ..
        } => Fingerprint::Kill {
            workload,
            policy,
            name,
        },
    };
    let encoded = serde_json::to_vec(&fingerprint).map_err(|_| RuntimeError::Internal)?;
    Ok(Sha256::digest(encoded).into())
}

fn random_supervisor_id() -> Result<HelperSupervisorId, RuntimeError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|_| RuntimeError::Internal)?;
    HelperSupervisorId::new(hex(&bytes)).map_err(|_| RuntimeError::Internal)
}

fn random_request_id() -> Result<u64, RuntimeError> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).map_err(|_| RuntimeError::Internal)?;
    let value = u64::from_le_bytes(bytes);
    Ok(if value == 0 { 1 } else { value })
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn map_supervisor_error(error: ShellSupervisorError) -> RuntimeError {
    match error {
        ShellSupervisorError::InvalidSpec => RuntimeError::InvalidRequest,
        ShellSupervisorError::ReadyTimeout => RuntimeError::Timeout,
        ShellSupervisorError::SpawnFailed | ShellSupervisorError::RuntimeUnavailable => {
            RuntimeError::ShellUnavailable
        }
    }
}

fn map_supervisor_failure(error: SupervisorFailure) -> RuntimeError {
    match error {
        SupervisorFailure::AlreadyAttached => RuntimeError::ShellAlreadyAttached,
        SupervisorFailure::Closed => RuntimeError::TerminalClosed,
        SupervisorFailure::InvalidRequest => RuntimeError::InvalidRequest,
        SupervisorFailure::Internal => RuntimeError::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::ManagerEnvironment;
    use crate::runtime::{PersistedScopeLedger, RuntimeLedger};
    use crate::shell_socket::OwnedShellListener;
    use crate::systemd::{ScopeError, VerifiedScope};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use uzers::get_current_uid;

    fn workload() -> WorkloadIdentity {
        serde_json::from_value(serde_json::json!({
            "workloadId": "tools",
            "realmId": "host",
            "realmPath": ["host"],
            "canonicalTarget": "tools.host.d2b"
        }))
        .unwrap()
    }

    fn policy() -> HelperShellPolicy {
        HelperShellPolicy {
            default_name: ShellName::new("host").unwrap(),
            max_sessions: 2,
        }
    }

    #[derive(Debug)]
    struct FakeManager {
        environment: ManagerEnvironment,
        stop_calls: Arc<AtomicUsize>,
    }

    impl UserScopeManager for FakeManager {
        fn manager_environment(&self) -> Result<ManagerEnvironment, ScopeError> {
            Ok(self.environment.clone())
        }

        fn start_scope(
            &self,
            _supervisor_pid: u32,
            _kind: HelperScopeKind,
        ) -> Result<VerifiedScope, ScopeError> {
            Err(ScopeError::CreateFailed)
        }

        fn inspect_scope(&self, _scope: &VerifiedScope) -> Result<ScopeInspection, ScopeError> {
            Ok(ScopeInspection {
                state: HelperScopeState::Active,
                identity_matches: true,
            })
        }

        fn terminate_scope(&self, _scope: &VerifiedScope, _signal: i32) -> Result<(), ScopeError> {
            self.stop_calls.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        fn stop_scope(&self, _scope: &VerifiedScope) -> Result<(), ScopeError> {
            self.stop_calls.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(std::path::Path::parent)
                .unwrap();
            for _ in 0..32 {
                let mut random = [0u8; 2];
                getrandom::getrandom(&mut random).unwrap();
                let path = root.join(format!("s{:02x}{:02x}", random[0], random[1]));
                if fs::create_dir(&path).is_ok() {
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                    return Self(path);
                }
            }
            panic!("could not create repository-local shell test directory");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn attach_request(operation: &str, name: &str) -> HelperShellRequest {
        HelperShellRequest::Attach {
            request_id: 1,
            operation_id: OperationId::parse(operation).unwrap(),
            workload: workload(),
            policy: policy(),
            name: Some(ShellName::new(name).unwrap()),
            force: false,
            initial_terminal_size: d2b_contracts::terminal_wire::TerminalSize {
                rows: 24,
                cols: 80,
            },
        }
    }

    #[test]
    fn concurrent_duplicate_attach_reserves_one_operation() {
        const CONTENDERS: usize = 16;
        let ledger = Arc::new(Mutex::new(RuntimeLedger::from_persisted(
            PersistedScopeLedger {
                schema_version: 1,
                scopes: Vec::new(),
            },
        )));
        let barrier = Arc::new(Barrier::new(CONTENDERS));
        let request = attach_request("op-concurrent-shell", "host");
        let fingerprint = shell_fingerprint(&request).unwrap();
        let operation_id = request.operation_id().clone();
        let mut threads = Vec::new();
        for _ in 0..CONTENDERS {
            let ledger = Arc::clone(&ledger);
            let barrier = Arc::clone(&barrier);
            let operation_id = operation_id.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                ledger
                    .lock()
                    .unwrap()
                    .begin_shell_operation(&operation_id, fingerprint)
            }));
        }
        let results = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Ok(ShellOperationBegin::Started(_))))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(RuntimeError::OperationInProgress)))
                .count(),
            CONTENDERS - 1
        );
    }

    #[test]
    fn changed_operation_fingerprint_conflicts() {
        let first = attach_request("op-shell", "one");
        let changed = attach_request("op-shell", "two");
        let mut ledger = RuntimeLedger::from_persisted(PersistedScopeLedger {
            schema_version: 1,
            scopes: Vec::new(),
        });
        assert!(matches!(
            ledger.begin_shell_operation(first.operation_id(), shell_fingerprint(&first).unwrap()),
            Ok(ShellOperationBegin::Started(_))
        ));
        assert!(matches!(
            ledger.begin_shell_operation(
                changed.operation_id(),
                shell_fingerprint(&changed).unwrap()
            ),
            Err(RuntimeError::OperationIdConflict)
        ));
    }

    #[test]
    fn idempotent_management_replay_uses_current_request_correlation() {
        let response = HelperShellResponse::Detach(HelperShellDetachResponse {
            request_id: 1,
            operation_id: OperationId::parse("op-replay").unwrap(),
            result: ShellDetachResult {
                resolved_name: ShellName::new("host").unwrap(),
                detached: true,
                cause: Some(ShellCloseCause::EvictedByAdminDetach),
            },
        });
        let replayed = recorrelate_response(response, 99, OperationId::parse("op-replay").unwrap());
        assert_eq!(replayed.request_id(), 99);
        assert_eq!(replayed.operation_id().as_str(), "op-replay");
    }

    #[test]
    fn shell_keys_are_workload_scoped_and_debug_redacted() {
        let name = ShellName::new("private-name-canary").unwrap();
        let key = shell_name_key(&workload(), &name);
        assert!(key.ends_with("private-name-canary"));
        let operation = AttachOperation {
            request_id: 1,
            operation_id: OperationId::parse("op-redact").unwrap(),
            workload: workload(),
            policy: policy(),
            name: Some(name),
            force: false,
            rows: 24,
            cols: 80,
        };
        assert!(!format!("{operation:?}").contains("private-name-canary"));
    }

    #[test]
    fn helper_wide_ring_reservation_is_bounded() {
        assert!(shell_quota_allows(1, 0, 2, 63));
        assert!(!shell_quota_allows(2, 0, 2, 2));
        assert!(!shell_quota_allows(1, 1, 2, 2));
        assert!(!shell_quota_allows(0, 0, 64, 64));
    }

    #[test]
    fn reconstructed_runtime_adopts_verified_shell_and_degrades_missing_socket() {
        if get_current_uid() == 0 {
            return;
        }
        let scratch = Scratch::new();
        let supervisor_id = HelperSupervisorId::new("adoption-test").unwrap();
        let listener =
            OwnedShellListener::bind(&scratch.0, &supervisor_id, get_current_uid()).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.listener().accept().unwrap();
            let request: SupervisorRequest = read_frame(&mut stream).unwrap();
            write_frame(
                &mut stream,
                &SupervisorResponse {
                    version: SUPERVISOR_PROTOCOL_VERSION,
                    request_id: request.request_id,
                    result: SupervisorResult::Status {
                        running: true,
                        attached: false,
                        terminal_status: None,
                    },
                },
            )
            .unwrap();
        });
        let identity = workload();
        let ledger = PersistedScopeLedger {
            schema_version: 1,
            scopes: vec![
                PersistedScope {
                    operation_id: OperationId::parse("op-launcher").unwrap(),
                    fingerprint: Some([1; 32]),
                    workload: identity.clone(),
                    unit_name: "launcher.scope".to_owned(),
                    invocation_id: "launcher-invocation".to_owned(),
                    control_group: "/user/launcher.scope".to_owned(),
                    kind: HelperScopeKind::LauncherApp,
                    persistent_shell: None,
                },
                PersistedScope {
                    operation_id: OperationId::parse("op-shell-adopt").unwrap(),
                    fingerprint: Some([2; 32]),
                    workload: identity,
                    unit_name: "shell.scope".to_owned(),
                    invocation_id: "shell-invocation".to_owned(),
                    control_group: "/user/shell.scope".to_owned(),
                    kind: HelperScopeKind::PersistentShell,
                    persistent_shell: Some(PersistedShellMetadata {
                        name: ShellName::new("host").unwrap(),
                        supervisor_id,
                    }),
                },
            ],
        };
        let ledger_path = scratch.0.join("ledger.json");
        persist_ledger(&ledger_path, &ledger).unwrap();
        let stops = Arc::new(AtomicUsize::new(0));
        let manager = FakeManager {
            environment: ManagerEnvironment::parse(vec![
                "PATH=/bin".to_owned(),
                format!("XDG_RUNTIME_DIR={}", scratch.0.display()),
            ])
            .unwrap(),
            stop_calls: Arc::clone(&stops),
        };
        let runtime = ScopeRuntime::with_paths_and_executable(
            manager,
            scratch.0.clone(),
            ledger_path,
            std::env::current_exe().unwrap(),
        )
        .unwrap();
        let adopted = runtime.snapshot(7).unwrap();
        server.join().unwrap();
        assert_eq!(adopted.scopes.len(), 2);
        let shell = adopted
            .scopes
            .iter()
            .find(|scope| scope.persistent_shell.is_some())
            .unwrap();
        assert_eq!(shell.state, HelperScopeState::Active);
        assert_eq!(
            shell.persistent_shell.as_ref().unwrap().state,
            ShellSessionState::Detached
        );

        let degraded = runtime.snapshot(8).unwrap();
        let shell = degraded
            .scopes
            .iter()
            .find(|scope| scope.persistent_shell.is_some())
            .unwrap();
        assert_eq!(shell.state, HelperScopeState::Degraded);
        assert_eq!(
            shell.persistent_shell.as_ref().unwrap().state,
            ShellSessionState::PoolUnavailable
        );
        assert_eq!(stops.load(Ordering::Acquire), 0);
        assert_eq!(runtime.ledger.lock().unwrap().persisted.scopes.len(), 2);
    }

    #[test]
    fn shell_name_reservation_conflicts_without_consuming_other_names() {
        let mut ledger = RuntimeLedger::from_persisted(PersistedScopeLedger {
            schema_version: 1,
            scopes: Vec::new(),
        });
        let first = attach_request("op-name-one", "host");
        let second = attach_request("op-name-two", "host");
        let other = attach_request("op-name-three", "other");
        let first_reservation = match ledger
            .begin_shell_operation(first.operation_id(), shell_fingerprint(&first).unwrap())
            .unwrap()
        {
            ShellOperationBegin::Started(reservation) => reservation,
            _ => panic!("unexpected replay"),
        };
        ledger
            .reserve_shell_name(
                shell_name_key(&workload(), &ShellName::new("host").unwrap()),
                first_reservation,
            )
            .unwrap();
        let second_reservation = match ledger
            .begin_shell_operation(second.operation_id(), shell_fingerprint(&second).unwrap())
            .unwrap()
        {
            ShellOperationBegin::Started(reservation) => reservation,
            _ => panic!("unexpected replay"),
        };
        assert_eq!(
            ledger.reserve_shell_name(
                shell_name_key(&workload(), &ShellName::new("host").unwrap()),
                second_reservation,
            ),
            Err(RuntimeError::OperationInProgress)
        );
        let other_reservation = match ledger
            .begin_shell_operation(other.operation_id(), shell_fingerprint(&other).unwrap())
            .unwrap()
        {
            ShellOperationBegin::Started(reservation) => reservation,
            _ => panic!("unexpected replay"),
        };
        assert!(
            ledger
                .reserve_shell_name(
                    shell_name_key(&workload(), &ShellName::new("other").unwrap()),
                    other_reservation,
                )
                .is_ok()
        );
    }
}
