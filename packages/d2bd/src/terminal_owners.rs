//! Production owners behind public daemon terminal streams.

use std::{fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use d2b_contracts::{
    public_wire, terminal_wire as legacy_terminal,
    v2_services::{
        guest::{GuestExecRequest, GuestOpenShellRequest},
        terminal::{self, terminal_selection},
    },
};
use protobuf::{EnumOrUnknown, MessageField};

use crate::{
    ServerState,
    admission::{PeerIdentity, PeerRole},
    daemon_terminal::{
        PreparedTerminal, TerminalBinding, TerminalCommand, TerminalFailure, TerminalFinish,
        TerminalOpenResult, TerminalOutputStream, TerminalOwner, TerminalOwnerEvent,
        cancelled_outcome, closed_outcome, detached_outcome,
    },
    shell_backend,
    typed_error::TypedError,
    workload_dispatch::WorkloadRoute,
};

const OWNER_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

enum PreparedRoute {
    Guest(Arc<crate::guest_terminal::GuestTerminalSession>),
    UnsafeLocalShell,
    ConfiguredLaunch,
    Console(public_wire::ConsoleProviderKind),
}

pub struct ProductionPreparedTerminal {
    state: Arc<ServerState>,
    peer: PeerIdentity,
    request: terminal::TerminalOpenRequest,
    route: PreparedRoute,
    kind: terminal::TerminalKind,
}

impl fmt::Debug for ProductionPreparedTerminal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProductionPreparedTerminal")
            .field("peer_uid", &self.peer.uid)
            .field("binding", &"<redacted>")
            .field("kind", &self.kind)
            .finish()
    }
}

pub async fn prepare(
    state: Arc<ServerState>,
    peer: PeerIdentity,
    kind: terminal::TerminalKind,
    request: &terminal::TerminalOpenRequest,
    session_generation: u64,
) -> Result<Arc<dyn PreparedTerminal>, TerminalFailure> {
    if session_generation == 0
        || request
            .metadata
            .as_ref()
            .is_none_or(|metadata| metadata.session_generation != session_generation)
    {
        return Err(TerminalFailure::GenerationMismatch);
    }
    let route = match kind {
        terminal::TerminalKind::TERMINAL_KIND_EXEC if request.resource_id.ends_with(".d2b") => {
            preflight_configured_launch(&state, &request.resource_id)?;
            PreparedRoute::ConfiguredLaunch
        }
        terminal::TerminalKind::TERMINAL_KIND_EXEC => {
            require_guest_scope(request)?;
            PreparedRoute::Guest(
                state
                    .guest_terminal_connector
                    .connect(&request.resource_id)
                    .await?,
            )
        }
        terminal::TerminalKind::TERMINAL_KIND_SHELL => {
            if !matches!(peer.role, PeerRole::Admin) {
                return Err(TerminalFailure::Unauthorized);
            }
            let resolved = crate::resolve_shell_target(&state, &request.resource_id)
                .map_err(map_typed_error)?;
            match resolved.route {
                WorkloadRoute::UnsafeLocal => {
                    match state.unsafe_local_helpers.availability(peer.uid) {
                        crate::unsafe_local_helper::HelperAvailability::Ready => {
                            PreparedRoute::UnsafeLocalShell
                        }
                        crate::unsafe_local_helper::HelperAvailability::Unavailable
                        | crate::unsafe_local_helper::HelperAvailability::Stale => {
                            return Err(TerminalFailure::Unavailable);
                        }
                    }
                }
                WorkloadRoute::LocalVm { vm } => {
                    require_guest_scope(request)?;
                    PreparedRoute::Guest(state.guest_terminal_connector.connect(&vm).await?)
                }
                WorkloadRoute::CapabilityUnavailable { .. } => {
                    return Err(TerminalFailure::InvalidSelection);
                }
            }
        }
        terminal::TerminalKind::TERMINAL_KIND_CONSOLE => {
            let provider = crate::resolve_console_provider_kind(&state, &request.resource_id)
                .map_err(map_typed_error)?;
            PreparedRoute::Console(provider)
        }
        _ => return Err(TerminalFailure::Protocol),
    };
    Ok(Arc::new(ProductionPreparedTerminal {
        state,
        peer,
        request: request.clone(),
        route,
        kind,
    }))
}

fn require_guest_scope(request: &terminal::TerminalOpenRequest) -> Result<(), TerminalFailure> {
    if request
        .scope
        .as_ref()
        .is_none_or(|scope| scope.workload_id.is_empty())
    {
        Err(TerminalFailure::InvalidSelection)
    } else {
        Ok(())
    }
}

fn preflight_configured_launch(state: &ServerState, target: &str) -> Result<(), TerminalFailure> {
    let target =
        d2b_realm_core::RealmTarget::parse(target).map_err(|_| TerminalFailure::NotFound)?;
    let resolver = crate::load_bundle_resolver(state).map_err(map_typed_error)?;
    let catalog = crate::workload_dispatch::WorkloadCatalog::from_resolver(&resolver)
        .map_err(|_| TerminalFailure::Unavailable)?;
    catalog
        .resolve(&target)
        .map(|_| ())
        .map_err(|_| TerminalFailure::NotFound)
}

#[async_trait]
impl PreparedTerminal for ProductionPreparedTerminal {
    async fn open(
        &self,
        binding: &TerminalBinding,
        selection: terminal::TerminalSelection,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        if binding.operation_id != self.request.operation_id
            || binding.peer_uid != self.peer.uid
            || binding.peer_principal
                != format!(
                    "local-{}-{}",
                    self.peer.uid,
                    peer_role_label(self.peer.role)
                )
            || binding.kind != self.kind
        {
            return Err(TerminalFailure::GenerationMismatch);
        }

        fn peer_role_label(role: PeerRole) -> &'static str {
            match role {
                PeerRole::Launcher => "launcher",
                PeerRole::Admin => "admin",
                PeerRole::HostShutdown => "host-shutdown",
            }
        }
        match (&self.route, selection.selection.as_ref()) {
            (PreparedRoute::Guest(session), Some(terminal_selection::Selection::Exec(exec))) => {
                if !matches!(self.peer.role, PeerRole::Admin)
                    || exec.authority.enum_value().ok()
                        != Some(terminal::ExecAuthority::EXEC_AUTHORITY_ADMIN_ARBITRARY)
                {
                    return Err(TerminalFailure::Unauthorized);
                }
                let guest_request = GuestExecRequest {
                    terminal: MessageField::some(guest_request(
                        &self.request,
                        session.generation(),
                    )?),
                    ..Default::default()
                };
                session
                    .open_exec(guest_request, selection, OWNER_OPERATION_TIMEOUT)
                    .await
            }
            (PreparedRoute::ConfiguredLaunch, Some(terminal_selection::Selection::Exec(exec))) => {
                self.open_configured_launch(exec).await
            }
            (PreparedRoute::Guest(session), Some(terminal_selection::Selection::Shell(_))) => {
                let guest_request = GuestOpenShellRequest {
                    terminal: MessageField::some(guest_request(
                        &self.request,
                        session.generation(),
                    )?),
                    ..Default::default()
                };
                session
                    .open_shell(guest_request, selection, OWNER_OPERATION_TIMEOUT)
                    .await
            }
            (
                PreparedRoute::UnsafeLocalShell,
                Some(terminal_selection::Selection::Shell(shell)),
            ) => self.open_unsafe_shell(shell).await,
            (PreparedRoute::Console(provider), Some(terminal_selection::Selection::Console(_))) => {
                self.open_console(*provider)
            }
            _ => Err(TerminalFailure::InvalidSelection),
        }
    }
}

fn guest_request(
    public: &terminal::TerminalOpenRequest,
    generation: u64,
) -> Result<terminal::TerminalOpenRequest, TerminalFailure> {
    let mut request = public.clone();
    let metadata = request.metadata.as_mut().ok_or(TerminalFailure::Protocol)?;
    metadata.session_generation = generation;
    Ok(request)
}

impl ProductionPreparedTerminal {
    async fn open_configured_launch(
        &self,
        exec: &terminal::ExecSelection,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        use terminal::exec_selection::Selection;
        if !matches!(self.peer.role, PeerRole::Admin | PeerRole::Launcher)
            || exec.authority.enum_value().ok()
                != Some(terminal::ExecAuthority::EXEC_AUTHORITY_CONFIGURED_LAUNCH)
        {
            return Err(TerminalFailure::Unauthorized);
        }
        let Some(Selection::ConfiguredLaunch(configured)) = exec.selection.as_ref() else {
            return Err(TerminalFailure::InvalidSelection);
        };
        let target = d2b_realm_core::RealmTarget::parse(&self.request.resource_id)
            .map_err(|_| TerminalFailure::NotFound)?;
        let item_id = d2b_realm_core::ProtocolToken::parse(configured.configured_item_id.clone())
            .map_err(|_| TerminalFailure::InvalidSelection)?;
        let operation_id = d2b_realm_core::OperationId::parse(self.request.operation_id.clone())
            .map_err(|_| TerminalFailure::InvalidSelection)?;
        let state = Arc::clone(&self.state);
        let peer = self.peer.clone();
        tokio::task::spawn_blocking(move || {
            crate::dispatch_workload(
                &state,
                &peer,
                public_wire::WorkloadOp::LauncherExec(public_wire::LauncherExecArgs {
                    target,
                    item_id,
                    operation_id,
                }),
            )
        })
        .await
        .map_err(|_| TerminalFailure::Internal)?
        .map_err(map_typed_error)?;
        Ok(TerminalOpenResult::Terminal(detached_outcome()))
    }

    async fn open_unsafe_shell(
        &self,
        shell: &terminal::ShellSelection,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        let action = shell
            .action
            .enum_value()
            .map_err(|_| TerminalFailure::InvalidSelection)?;
        match action {
            terminal::ShellAction::SHELL_ACTION_ATTACH_DEFAULT
            | terminal::ShellAction::SHELL_ACTION_ATTACH_CONFIGURED => {
                self.attach_unsafe_shell(shell, action).await
            }
            terminal::ShellAction::SHELL_ACTION_LIST
            | terminal::ShellAction::SHELL_ACTION_DETACH
            | terminal::ShellAction::SHELL_ACTION_KILL => {
                self.manage_unsafe_shell(shell, action).await
            }
            terminal::ShellAction::SHELL_ACTION_UNSPECIFIED => {
                Err(TerminalFailure::InvalidSelection)
            }
        }
    }

    async fn attach_unsafe_shell(
        &self,
        shell: &terminal::ShellSelection,
        action: terminal::ShellAction,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        let size = shell
            .initial_size
            .as_ref()
            .ok_or(TerminalFailure::InvalidSelection)?;
        let name = if action == terminal::ShellAction::SHELL_ACTION_ATTACH_DEFAULT {
            None
        } else {
            Some(
                public_wire::ShellName::new(shell.configured_shell_id.clone())
                    .map_err(|_| TerminalFailure::InvalidSelection)?,
            )
        };
        let attach = public_wire::ShellAttachArgs {
            vm: self.request.resource_id.clone(),
            name,
            force: shell.force,
            initial_terminal_size: legacy_terminal::TerminalSize {
                rows: size.rows,
                cols: size.columns,
            },
        };
        let state = Arc::clone(&self.state);
        let peer_uid = self.peer.uid;
        let (runtime, established) = tokio::task::spawn_blocking(move || {
            let runtime = Arc::new(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| TerminalFailure::Internal)?,
            );
            let established = runtime
                .block_on(crate::establish_shell_backend(&state, peer_uid, &attach))
                .map_err(map_typed_error)?;
            Ok::<_, TerminalFailure>((runtime, established))
        })
        .await
        .map_err(|_| TerminalFailure::Internal)??;
        Ok(TerminalOpenResult::Active {
            started: terminal::TerminalStarted {
                kind: EnumOrUnknown::new(terminal::TerminalKind::TERMINAL_KIND_SHELL),
                tty: true,
                ..Default::default()
            },
            owner: Box::new(UnsafeShellOwner {
                runtime,
                session: established.attach.session,
                backend: established.backend,
                control_sequence: established.initial_control_sequence,
                stdout_offset: 0,
                stderr_offset: 0,
                terminal: false,
            }),
        })
    }

    async fn manage_unsafe_shell(
        &self,
        shell: &terminal::ShellSelection,
        action: terminal::ShellAction,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        let vm = self.request.resource_id.clone();
        let op = match action {
            terminal::ShellAction::SHELL_ACTION_LIST => {
                public_wire::ShellOp::List(public_wire::ShellListArgs { vm })
            }
            terminal::ShellAction::SHELL_ACTION_DETACH => {
                public_wire::ShellOp::Detach(public_wire::ShellDetachArgs {
                    vm,
                    name: Some(
                        public_wire::ShellName::new(shell.shell_handle.clone())
                            .map_err(|_| TerminalFailure::InvalidSelection)?,
                    ),
                })
            }
            terminal::ShellAction::SHELL_ACTION_KILL => {
                public_wire::ShellOp::Kill(public_wire::ShellKillArgs {
                    vm,
                    name: public_wire::ShellName::new(shell.shell_handle.clone())
                        .map_err(|_| TerminalFailure::InvalidSelection)?,
                })
            }
            _ => return Err(TerminalFailure::InvalidSelection),
        };
        let state = Arc::clone(&self.state);
        let peer = self.peer.clone();
        let response = tokio::task::spawn_blocking(move || {
            crate::dispatch_shell_management(&state, &peer, op)
        })
        .await
        .map_err(|_| TerminalFailure::Internal)?
        .map_err(map_typed_error)?;
        let mut object = response
            .as_object()
            .cloned()
            .ok_or(TerminalFailure::Protocol)?;
        object.remove("type");
        let response: public_wire::ShellOpResponse =
            serde_json::from_value(serde_json::Value::Object(object))
                .map_err(|_| TerminalFailure::Protocol)?;
        Ok(TerminalOpenResult::Immediate(vec![
            TerminalOwnerEvent::ShellResult(shell_management_result(action, response)?),
            TerminalOwnerEvent::Outcome(closed_outcome()),
        ]))
    }

    fn open_console(
        &self,
        provider: public_wire::ConsoleProviderKind,
    ) -> Result<TerminalOpenResult, TerminalFailure> {
        let mut sessions = self
            .state
            .console_sessions
            .lock()
            .map_err(|_| TerminalFailure::Internal)?;
        if !sessions.has_session(&self.request.resource_id) {
            let session = crate::create_console_session_for_vm(
                &self.state,
                &self.request.resource_id,
                provider,
            )
            .map_err(map_typed_error)?;
            sessions.register_session(self.request.resource_id.clone(), session);
        }
        let (handle, provider, start_offset) = sessions
            .attach(&self.request.resource_id, self.peer.uid)
            .map_err(|_| TerminalFailure::Internal)?
            .ok_or(TerminalFailure::ResourceExhausted)?;
        Ok(TerminalOpenResult::Active {
            started: terminal::TerminalStarted {
                kind: EnumOrUnknown::new(terminal::TerminalKind::TERMINAL_KIND_CONSOLE),
                tty: true,
                stdout_offset: start_offset,
                console_provider: EnumOrUnknown::new(map_console_provider(provider)),
                ..Default::default()
            },
            owner: Box::new(ConsoleOwner {
                sessions: Arc::clone(&self.state.console_sessions),
                handle: handle.as_str().to_owned(),
                offset: start_offset,
                eof_sent: false,
                terminal: false,
            }),
        })
    }
}

fn shell_management_result(
    action: terminal::ShellAction,
    response: public_wire::ShellOpResponse,
) -> Result<terminal::ShellManagementResult, TerminalFailure> {
    let mut result = terminal::ShellManagementResult {
        action: EnumOrUnknown::new(action),
        ..Default::default()
    };
    match response {
        public_wire::ShellOpResponse::List(list) => {
            result.sessions = list
                .sessions
                .into_iter()
                .take(d2b_contracts::v2_services::MAX_PAGE_SIZE as usize)
                .map(|entry| terminal::ShellSession {
                    shell_handle: entry.name.as_str().to_owned(),
                    state: EnumOrUnknown::new(map_shell_state(entry.state)),
                    is_default: entry.is_default,
                    ..Default::default()
                })
                .collect();
            result.truncated = false;
        }
        public_wire::ShellOpResponse::Detach(detach) => {
            if !detach.detached {
                return Err(TerminalFailure::NotFound);
            }
            result.affected_shell_handle = detach.resolved_name.as_str().to_owned();
            result.applied = true;
        }
        public_wire::ShellOpResponse::Kill(kill) => {
            if !kill.killed {
                return Err(TerminalFailure::NotFound);
            }
            result.affected_shell_handle = kill.name.as_str().to_owned();
            result.applied = true;
        }
        _ => return Err(TerminalFailure::Protocol),
    }
    Ok(result)
}

fn map_shell_state(state: public_wire::ShellSessionState) -> terminal::ShellSessionState {
    match state {
        public_wire::ShellSessionState::Attached => {
            terminal::ShellSessionState::SHELL_SESSION_STATE_ATTACHED
        }
        public_wire::ShellSessionState::Detached => {
            terminal::ShellSessionState::SHELL_SESSION_STATE_DETACHED
        }
        public_wire::ShellSessionState::Killed => {
            terminal::ShellSessionState::SHELL_SESSION_STATE_KILLED
        }
        public_wire::ShellSessionState::PoolUnavailable
        | public_wire::ShellSessionState::FeatureDisabled
        | public_wire::ShellSessionState::OutputGap => {
            terminal::ShellSessionState::SHELL_SESSION_STATE_UNAVAILABLE
        }
    }
}

struct UnsafeShellOwner {
    runtime: Arc<tokio::runtime::Runtime>,
    session: String,
    backend: Arc<dyn shell_backend::ShellBackend>,
    control_sequence: u64,
    stdout_offset: u64,
    stderr_offset: u64,
    terminal: bool,
}

impl fmt::Debug for UnsafeShellOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnsafeShellOwner")
            .field("binding", &"<redacted>")
            .field("control_sequence", &self.control_sequence)
            .field("terminal", &self.terminal)
            .finish()
    }
}

#[async_trait]
impl TerminalOwner for UnsafeShellOwner {
    async fn command(
        &mut self,
        command: TerminalCommand,
    ) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
        let op = match command {
            TerminalCommand::Stdin { offset, data, eof } => {
                public_wire::ShellOp::WriteStdin(legacy_terminal::TerminalWriteStdin {
                    session: self.session.clone(),
                    offset,
                    chunk_base64: d2b_core::base64_codec::encode(&data),
                    eof,
                })
            }
            TerminalCommand::Resize {
                operation_sequence,
                rows,
                columns,
            } => {
                if operation_sequence <= self.control_sequence {
                    return Err(TerminalFailure::Conflict);
                }
                public_wire::ShellOp::Resize(legacy_terminal::TerminalResize {
                    session: self.session.clone(),
                    rows,
                    cols: columns,
                    op_id: operation_sequence,
                })
            }
            TerminalCommand::Signal { .. } => return Err(TerminalFailure::InvalidSelection),
            TerminalCommand::CloseStdin => {
                public_wire::ShellOp::CloseStdin(legacy_terminal::TerminalClose {
                    session: self.session.clone(),
                })
            }
        };
        let response = shell_backend_call(
            Arc::clone(&self.runtime),
            Arc::clone(&self.backend),
            self.control_sequence,
            op,
        )
        .await?;
        self.control_sequence = response.control_sequence;
        shell_response_events(response.response)
    }

    async fn poll(&mut self) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
        if self.terminal {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        for (stream, offset) in [
            (legacy_terminal::TerminalStream::Stdout, self.stdout_offset),
            (legacy_terminal::TerminalStream::Stderr, self.stderr_offset),
        ] {
            let response = shell_backend_call(
                Arc::clone(&self.runtime),
                Arc::clone(&self.backend),
                self.control_sequence,
                public_wire::ShellOp::ReadOutput(legacy_terminal::TerminalReadOutput {
                    session: self.session.clone(),
                    stream,
                    offset,
                    max_len: d2b_contracts::v2_services::MAX_TERMINAL_CHUNK_BYTES as u64,
                    wait: false,
                    timeout_ms: 0,
                }),
            )
            .await?;
            self.control_sequence = response.control_sequence;
            let mut output = shell_response_events(response.response)?;
            for event in &mut output {
                if let TerminalOwnerEvent::Output {
                    stream: output_stream,
                    offset,
                    data,
                    dropped_bytes,
                    ..
                } = event
                {
                    *output_stream = match stream {
                        legacy_terminal::TerminalStream::Stdout => TerminalOutputStream::Stdout,
                        legacy_terminal::TerminalStream::Stderr => TerminalOutputStream::Stderr,
                    };
                    let next = offset
                        .saturating_add(*dropped_bytes)
                        .saturating_add(data.len() as u64);
                    match output_stream {
                        TerminalOutputStream::Stdout => self.stdout_offset = next,
                        TerminalOutputStream::Stderr => self.stderr_offset = next,
                    }
                }
            }
            events.append(&mut output);
        }
        Ok(events)
    }

    async fn finish(
        &mut self,
        finish: TerminalFinish,
    ) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
        let backend = Arc::clone(&self.backend);
        let runtime = Arc::clone(&self.runtime);
        let control_sequence = self.control_sequence;
        tokio::task::spawn_blocking(move || {
            let mut sequence = control_sequence;
            backend
                .close_attachment(runtime.handle(), &mut sequence)
                .map_err(map_typed_error)
        })
        .await
        .map_err(|_| TerminalFailure::Internal)??;
        self.terminal = true;
        let outcome = match finish {
            TerminalFinish::Cancel => cancelled_outcome(),
            TerminalFinish::Close => closed_outcome(),
            TerminalFinish::Detach | TerminalFinish::Disconnect => detached_outcome(),
        };
        Ok(vec![TerminalOwnerEvent::Outcome(outcome)])
    }
}

struct ShellBackendResponse {
    control_sequence: u64,
    response: Option<public_wire::ShellOpResponse>,
}

async fn shell_backend_call(
    runtime: Arc<tokio::runtime::Runtime>,
    backend: Arc<dyn shell_backend::ShellBackend>,
    control_sequence: u64,
    op: public_wire::ShellOp,
) -> Result<ShellBackendResponse, TerminalFailure> {
    tokio::task::spawn_blocking(move || {
        let mut sequence = control_sequence;
        let response = backend
            .handle_op(runtime.handle(), &mut sequence, op)
            .map_err(map_typed_error)?;
        Ok(ShellBackendResponse {
            control_sequence: sequence,
            response,
        })
    })
    .await
    .map_err(|_| TerminalFailure::Internal)?
}

fn shell_response_events(
    response: Option<public_wire::ShellOpResponse>,
) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
    Ok(match response {
        Some(public_wire::ShellOpResponse::WriteStdin(result)) => {
            vec![TerminalOwnerEvent::Status {
                status: if result.stdin_closed {
                    terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_CLOSED
                } else if result.backpressured {
                    terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_BACKPRESSURED
                } else {
                    terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_ACCEPTED
                },
                next_stdin_offset: result.next_offset,
            }]
        }
        Some(public_wire::ShellOpResponse::ReadOutput(result)) => {
            let data = d2b_core::base64_codec::decode(&result.data_base64)
                .map_err(|_| TerminalFailure::Protocol)?;
            vec![TerminalOwnerEvent::Output {
                stream: TerminalOutputStream::Stdout,
                offset: result
                    .next_offset
                    .saturating_sub(result.dropped_bytes)
                    .saturating_sub(data.len() as u64),
                data,
                eof: result.eof,
                dropped_bytes: result.dropped_bytes,
                truncated: result.truncated,
            }]
        }
        Some(public_wire::ShellOpResponse::Resize(_)) => {
            vec![control_applied(0)]
        }
        Some(public_wire::ShellOpResponse::CloseStdin(_)) => {
            vec![TerminalOwnerEvent::Status {
                status: terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_CLOSED,
                next_stdin_offset: 0,
            }]
        }
        Some(public_wire::ShellOpResponse::Wait(_))
        | Some(public_wire::ShellOpResponse::CloseAttach(_))
        | None => Vec::new(),
        _ => return Err(TerminalFailure::Protocol),
    })
}

struct ConsoleOwner {
    sessions: Arc<std::sync::Mutex<crate::console_session::ConsoleSessionTable>>,
    handle: String,
    offset: u64,
    eof_sent: bool,
    terminal: bool,
}

#[async_trait]
impl TerminalOwner for ConsoleOwner {
    async fn command(
        &mut self,
        command: TerminalCommand,
    ) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
        match command {
            TerminalCommand::Stdin { data, .. } => {
                let accepted = self
                    .sessions
                    .lock()
                    .map_err(|_| TerminalFailure::Internal)?
                    .write_stdin(&self.handle, data)
                    .ok_or(TerminalFailure::NotFound)?;
                Ok(vec![TerminalOwnerEvent::Status {
                    status: if accepted {
                        terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_ACCEPTED
                    } else {
                        terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_BACKPRESSURED
                    },
                    next_stdin_offset: 0,
                }])
            }
            TerminalCommand::Resize { .. } => Ok(vec![control_applied(0)]),
            TerminalCommand::Signal { .. } | TerminalCommand::CloseStdin => {
                Err(TerminalFailure::InvalidSelection)
            }
        }
    }

    async fn poll(&mut self) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
        if self.terminal || self.eof_sent {
            return Ok(Vec::new());
        }
        let maximum_chunk = d2b_contracts::v2_services::MAX_TERMINAL_CHUNK_BYTES as u64;
        let output = self
            .sessions
            .lock()
            .map_err(|_| TerminalFailure::Internal)?
            .read_output(&self.handle, self.offset, maximum_chunk)
            .ok_or(TerminalFailure::NotFound)?;
        let Some(snapshot) = output.snap else {
            return Ok(Vec::new());
        };
        let offset = self.offset;
        self.offset = snapshot
            .actual_offset
            .saturating_add(snapshot.data.len() as u64);
        let final_chunk = snapshot.is_eof && snapshot.data.len() < maximum_chunk as usize;
        let mut events = vec![TerminalOwnerEvent::Output {
            stream: TerminalOutputStream::Stdout,
            offset,
            data: snapshot.data,
            eof: final_chunk,
            dropped_bytes: snapshot.actual_offset.saturating_sub(offset),
            truncated: snapshot.actual_offset != offset,
        }];
        if final_chunk {
            self.eof_sent = true;
            self.terminal = true;
            self.sessions
                .lock()
                .map_err(|_| TerminalFailure::Internal)?
                .close(&self.handle);
            events.push(TerminalOwnerEvent::Outcome(closed_outcome()));
        }
        Ok(events)
    }

    async fn finish(
        &mut self,
        finish: TerminalFinish,
    ) -> Result<Vec<TerminalOwnerEvent>, TerminalFailure> {
        if self.terminal || self.eof_sent {
            return Ok(Vec::new());
        }
        self.sessions
            .lock()
            .map_err(|_| TerminalFailure::Internal)?
            .close(&self.handle);
        self.terminal = true;
        let outcome = match finish {
            TerminalFinish::Cancel => cancelled_outcome(),
            TerminalFinish::Detach => detached_outcome(),
            TerminalFinish::Close | TerminalFinish::Disconnect => closed_outcome(),
        };
        Ok(vec![TerminalOwnerEvent::Outcome(outcome)])
    }
}

fn control_applied(next_stdin_offset: u64) -> TerminalOwnerEvent {
    TerminalOwnerEvent::Status {
        status: terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_CONTROL_APPLIED,
        next_stdin_offset,
    }
}

fn map_console_provider(
    provider: public_wire::ConsoleProviderKind,
) -> terminal::ConsoleProviderKind {
    match provider {
        public_wire::ConsoleProviderKind::LocalHypervisor => {
            terminal::ConsoleProviderKind::CONSOLE_PROVIDER_KIND_LOCAL_HYPERVISOR
        }
        public_wire::ConsoleProviderKind::QemuMedia => {
            terminal::ConsoleProviderKind::CONSOLE_PROVIDER_KIND_QEMU_MEDIA
        }
        public_wire::ConsoleProviderKind::AcaSandbox => {
            terminal::ConsoleProviderKind::CONSOLE_PROVIDER_KIND_ACA_SANDBOX
        }
    }
}

fn map_typed_error(error: TypedError) -> TerminalFailure {
    match error {
        TypedError::AuthzNotAdmin { .. } | TypedError::AuthzNotALauncher { .. } => {
            TerminalFailure::Unauthorized
        }
        TypedError::DaemonBusy | TypedError::ConsoleSessionTableFull { .. } => {
            TerminalFailure::ResourceExhausted
        }
        TypedError::ConsoleSessionStale | TypedError::WorkloadTargetNotFound { .. } => {
            TerminalFailure::NotFound
        }
        TypedError::RuntimeCapabilityUnsupported { .. } => TerminalFailure::InvalidSelection,
        _ => TerminalFailure::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use d2b_contracts::v2_services::{
        TerminalFrameDirection, TerminalStreamValidator,
        terminal::{terminal_outcome, terminal_stream_frame},
    };

    #[test]
    fn shell_management_mapping_satisfies_bound_state_machine() {
        let response = public_wire::ShellOpResponse::List(public_wire::ShellListResult {
            default_name: public_wire::ShellName::new("primary").unwrap(),
            sessions: vec![public_wire::ShellListEntry {
                name: public_wire::ShellName::new("primary").unwrap(),
                state: public_wire::ShellSessionState::Detached,
                attached: false,
                is_default: true,
            }],
        });
        let result =
            shell_management_result(terminal::ShellAction::SHELL_ACTION_LIST, response).unwrap();
        let mut validator = TerminalStreamValidator::new(
            terminal::TerminalKind::TERMINAL_KIND_SHELL,
            7,
            [1; 16],
            "operation-1",
            "shell-resource",
        )
        .unwrap();
        let selection = terminal::TerminalStreamFrame {
            session_generation: 7,
            request_id: vec![1; 16],
            sequence: 0,
            operation_id: "operation-1".to_owned(),
            resource_handle: "shell-resource".to_owned(),
            frame: Some(terminal_stream_frame::Frame::Select(
                terminal::TerminalSelection {
                    selection: Some(terminal_selection::Selection::Shell(
                        terminal::ShellSelection {
                            action: EnumOrUnknown::new(terminal::ShellAction::SHELL_ACTION_LIST),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        validator
            .accept(TerminalFrameDirection::ClientToServer, &selection)
            .unwrap();
        let shell_result = terminal::TerminalStreamFrame {
            session_generation: 7,
            request_id: vec![1; 16],
            sequence: 0,
            operation_id: "operation-1".to_owned(),
            resource_handle: "shell-resource".to_owned(),
            frame: Some(terminal_stream_frame::Frame::ShellResult(result)),
            ..Default::default()
        };
        validator
            .accept(TerminalFrameDirection::ServerToClient, &shell_result)
            .unwrap();
        let outcome = terminal::TerminalStreamFrame {
            sequence: 1,
            frame: Some(terminal_stream_frame::Frame::Outcome(
                terminal::TerminalOutcome {
                    outcome: Some(terminal_outcome::Outcome::Closed(
                        terminal::TerminalClosed::default(),
                    )),
                    ..Default::default()
                },
            )),
            ..shell_result
        };
        validator
            .accept(TerminalFrameDirection::ServerToClient, &outcome)
            .unwrap();
        assert!(validator.is_terminal());
    }

    #[tokio::test]
    async fn console_owner_bridges_output_stdin_and_close() {
        let ring = Arc::new(Mutex::new(crate::console_session::ConsoleRing::new()));
        ring.lock().unwrap().ring.push_bytes(b"console-output");
        let (stdin, mut stdin_receive) = tokio::sync::mpsc::channel(2);
        let mut table = crate::console_session::ConsoleSessionTable::new();
        table.register_session(
            "corp-vm".to_owned(),
            crate::console_session::ConsoleSession::new(
                public_wire::ConsoleProviderKind::LocalHypervisor,
                ring,
                None,
                Some(stdin),
            ),
        );
        let (handle, _, offset) = table.attach("corp-vm", 1000).unwrap().unwrap();
        let sessions = Arc::new(Mutex::new(table));
        let mut owner = ConsoleOwner {
            sessions: Arc::clone(&sessions),
            handle: handle.as_str().to_owned(),
            offset,
            eof_sent: false,
            terminal: false,
        };
        let output = owner.poll().await.unwrap();
        assert!(matches!(
            output.as_slice(),
            [TerminalOwnerEvent::Output { data, .. }] if data == b"console-output"
        ));
        owner
            .command(TerminalCommand::Stdin {
                offset: 0,
                data: b"input".to_vec(),
                eof: false,
            })
            .await
            .unwrap();
        assert_eq!(stdin_receive.recv().await.unwrap(), b"input");
        let outcome = owner.finish(TerminalFinish::Close).await.unwrap();
        assert!(matches!(
            outcome.as_slice(),
            [TerminalOwnerEvent::Outcome(terminal::TerminalOutcome {
                outcome: Some(terminal_outcome::Outcome::Closed(_)),
                ..
            })]
        ));
        assert_eq!(
            sessions.lock().unwrap().client_owner_uid(handle.as_str()),
            None
        );
    }

    #[tokio::test]
    async fn console_owner_marks_only_final_chunk_eof_and_closes_once() {
        let maximum = d2b_contracts::v2_services::MAX_TERMINAL_CHUNK_BYTES;
        let ring = Arc::new(Mutex::new(crate::console_session::ConsoleRing::new()));
        {
            let mut ring = ring.lock().unwrap();
            ring.ring.push_bytes(&vec![b'x'; maximum * 2]);
            ring.ring.is_eof = true;
        }
        let mut table = crate::console_session::ConsoleSessionTable::new();
        table.register_session(
            "corp-vm".to_owned(),
            crate::console_session::ConsoleSession::new(
                public_wire::ConsoleProviderKind::LocalHypervisor,
                ring,
                None,
                None,
            ),
        );
        let (handle, _, offset) = table.attach("corp-vm", 1000).unwrap().unwrap();
        let sessions = Arc::new(Mutex::new(table));
        let mut owner = ConsoleOwner {
            sessions: Arc::clone(&sessions),
            handle: handle.as_str().to_owned(),
            offset,
            eof_sent: false,
            terminal: false,
        };

        let first = owner.poll().await.unwrap();
        let second = owner.poll().await.unwrap();
        let final_events = owner.poll().await.unwrap();
        assert!(matches!(
            first.as_slice(),
            [TerminalOwnerEvent::Output {
                data,
                eof: false,
                ..
            }] if data.len() == maximum
        ));
        assert!(matches!(
            second.as_slice(),
            [TerminalOwnerEvent::Output {
                data,
                eof: false,
                ..
            }] if data.len() == maximum
        ));
        assert!(matches!(
            final_events.as_slice(),
            [
                TerminalOwnerEvent::Output {
                    data,
                    eof: true,
                    ..
                },
                TerminalOwnerEvent::Outcome(terminal::TerminalOutcome {
                    outcome: Some(terminal_outcome::Outcome::Closed(_)),
                    ..
                })
            ] if data.is_empty()
        ));
        assert!(owner.poll().await.unwrap().is_empty());
        assert!(
            owner
                .finish(TerminalFinish::Close)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            sessions.lock().unwrap().client_owner_uid(handle.as_str()),
            None
        );
    }

    #[test]
    fn unsafe_shell_output_mapping_preserves_bytes_without_debug_leak() {
        let secret = b"terminal-secret";
        let events = shell_response_events(Some(public_wire::ShellOpResponse::ReadOutput(
            legacy_terminal::TerminalReadOutputChunk {
                data_base64: d2b_core::base64_codec::encode(secret),
                next_offset: secret.len() as u64,
                eof: false,
                dropped_bytes: 0,
                truncated: false,
                timed_out: false,
            },
        )))
        .unwrap();
        assert!(matches!(
            events.as_slice(),
            [TerminalOwnerEvent::Output { data, .. }] if data == secret
        ));
        let owner = UnsafeShellOwner {
            runtime: Arc::new(
                tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap(),
            ),
            session: "secret-session-handle".to_owned(),
            backend: Arc::new(FailingShellBackend),
            control_sequence: 0,
            stdout_offset: 0,
            stderr_offset: 0,
            terminal: false,
        };
        let rendered = format!("{owner:?}");
        assert!(!rendered.contains("secret-session-handle"));
    }

    struct FailingShellBackend;

    impl shell_backend::ShellBackend for FailingShellBackend {
        fn handle_op(
            &self,
            _: &tokio::runtime::Handle,
            _: &mut u64,
            _: public_wire::ShellOp,
        ) -> Result<Option<public_wire::ShellOpResponse>, TypedError> {
            Err(TypedError::DaemonBusy)
        }

        fn close_attachment(
            &self,
            _: &tokio::runtime::Handle,
            _: &mut u64,
        ) -> Result<public_wire::ShellDetachResult, TypedError> {
            Err(TypedError::DaemonBusy)
        }
    }
}
