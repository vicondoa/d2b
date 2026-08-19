//! One-shot, out-of-band cutover owner.
//!
//! The broker launches this process before control-plane drain and transfers
//! the bootstrap over one close-on-exec-controlled fd. The runner owns the
//! durable journal, OFD lock, and drain-window socket. It never accepts a
//! raw host path or performs a host mutation directly.

use std::{
    env,
    fs::OpenOptions,
    io::Read as _,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use d2b_cutover::{
    ApplyContext, AuditEvidence, CapabilityLedger, HoldReason, MAX_RUNNER_FRAME_BYTES, Operation,
    OperationKind, RUNNER_BOOTSTRAP_FD, RunnerBootstrap, RunnerCommand, RunnerPaths, RunnerPeer,
    RunnerResponse, RunnerSocket, RunnerStatus, acquire_operation_lock, persist_journal,
    write_response,
};
use nix::{libc, unistd::setsid};

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("d2b-cutover-runner: {error}");
            std::process::ExitCode::from(78)
        }
    }
}

fn run() -> Result<(), RunnerError> {
    if nix::unistd::geteuid().as_raw() != 0 {
        return Err(RunnerError::NotRoot);
    }
    setsid().map_err(|_| RunnerError::Session)?;

    let state_root = env::var_os("D2B_CUTOVER_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/d2b"));
    let socket_root = env::var_os("D2B_CUTOVER_SOCKET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/d2b"));
    let bootstrap_fd = env::var("D2B_CUTOVER_BOOTSTRAP_FD")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(RUNNER_BOOTSTRAP_FD);
    let bootstrap_bytes = read_bootstrap_fd(bootstrap_fd)?;
    let mut ledger = CapabilityLedger::default();
    let (bootstrap, consumed) =
        RunnerBootstrap::decode_and_consume(&bootstrap_bytes, now_ms(), &mut ledger)
            .map_err(|_| RunnerError::Bootstrap)?;
    let paths = RunnerPaths::new_with_socket_root(state_root, socket_root, consumed.operation_id());
    if paths.journal.exists() {
        return Err(RunnerError::Replay);
    }
    let lock = acquire_operation_lock(&paths).map_err(|_| RunnerError::Lock)?;
    let operation = Operation::new(bootstrap.request.clone(), &bootstrap.preview)
        .map_err(|_| RunnerError::Request)?;
    persist_journal(
        &paths.journal,
        &bootstrap,
        &operation
            .journal_bytes()
            .map_err(|_| RunnerError::Journal)?,
    )
    .map_err(|_| RunnerError::Journal)?;

    let socket = RunnerSocket::bind(&paths, consumed).map_err(|_| RunnerError::Socket)?;
    let mut runtime = Runtime {
        bootstrap,
        operation,
        _lock: lock,
        paths,
        socket,
    };
    runtime.serve()
}

fn read_bootstrap_fd(fd: i32) -> Result<Vec<u8>, RunnerError> {
    if fd < 3 {
        return Err(RunnerError::Bootstrap);
    }
    let path = format!("/proc/self/fd/{fd}");
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| RunnerError::Bootstrap)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_RUNNER_FRAME_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| RunnerError::Bootstrap)?;
    if bytes.len() > MAX_RUNNER_FRAME_BYTES {
        return Err(RunnerError::Bootstrap);
    }
    nix::unistd::close(fd).map_err(|_| RunnerError::Bootstrap)?;
    Ok(bytes)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

struct Runtime {
    bootstrap: RunnerBootstrap,
    operation: Operation,
    _lock: std::fs::File,
    paths: RunnerPaths,
    socket: RunnerSocket,
}

impl Runtime {
    fn serve(&mut self) -> Result<(), RunnerError> {
        loop {
            let (mut stream, command, peer) = self
                .socket
                .accept_command()
                .map_err(|_| RunnerError::Socket)?;
            let response = self.handle(command, peer);
            write_response(&mut stream, &response).map_err(|_| RunnerError::Socket)?;
            if self.operation.state().is_terminal() {
                return Ok(());
            }
        }
    }

    fn handle(&mut self, command: RunnerCommand, peer: RunnerPeer) -> RunnerResponse {
        match command {
            RunnerCommand::Status => RunnerResponse {
                accepted: true,
                status: Some(self.status()),
                error: None,
            },
            RunnerCommand::Hold { reason } => {
                let reason = match HoldReason::new(reason) {
                    Ok(reason) => reason,
                    Err(_) => {
                        return RunnerResponse {
                            accepted: false,
                            status: Some(self.status()),
                            error: Some(d2b_cutover::RunnerSocketError::Malformed),
                        };
                    }
                };
                let requested_by = match peer {
                    RunnerPeer::Owner => self.bootstrap.request.operator_id().clone(),
                    RunnerPeer::Admin => d2b_cutover::OperatorId::new("admin-peer")
                        .expect("static operator identity"),
                };
                // A local record id is not privileged audit evidence. Until
                // the typed audited boundary is available, hold fails closed.
                let audit = AuditEvidence::unavailable();
                match self.operation.request_hold(requested_by, reason, audit) {
                    Ok(()) => {}
                    Err(d2b_cutover::OperationError::AuditNotDurable) => {
                        return self.failure(d2b_cutover::RunnerSocketError::AuditUnavailable);
                    }
                    Err(_) => {
                        return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                    }
                }
                if self.persist().is_err() {
                    return self.failure(d2b_cutover::RunnerSocketError::JournalUnavailable);
                }
                RunnerResponse {
                    accepted: true,
                    status: Some(self.status()),
                    error: None,
                }
            }
            RunnerCommand::Resume { .. } => {
                let context = match self.bootstrap.request.operation_kind() {
                    OperationKind::ScopedReset(_) => ApplyContext::reset(
                        now_ms(),
                        self.bootstrap.request.inventory_digest().clone(),
                        true,
                        true,
                        true,
                    ),
                    OperationKind::Cutover => {
                        return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                    }
                };
                // Resume has the same durability requirement as hold.
                let audit = AuditEvidence::unavailable();
                match self
                    .operation
                    .resume(self.bootstrap.request.operator_id(), &context, audit)
                {
                    Ok(()) => {}
                    Err(d2b_cutover::OperationError::AuditNotDurable) => {
                        return self.failure(d2b_cutover::RunnerSocketError::AuditUnavailable);
                    }
                    Err(_) => {
                        return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                    }
                }
                if self.persist().is_err() {
                    return self.failure(d2b_cutover::RunnerSocketError::JournalUnavailable);
                }
                RunnerResponse {
                    accepted: true,
                    status: Some(self.status()),
                    error: None,
                }
            }
        }
    }

    fn persist(&self) -> Result<(), RunnerError> {
        persist_journal(
            &self.paths.journal,
            &self.bootstrap,
            &self
                .operation
                .journal_bytes()
                .map_err(|_| RunnerError::Journal)?,
        )
        .map_err(|_| RunnerError::Journal)
    }

    fn status(&self) -> RunnerStatus {
        RunnerStatus {
            operation_id: self.bootstrap.request.operation_id().clone(),
            state: self.operation.state(),
            phase: self.operation.phase(),
            hold_active: matches!(self.operation.state(), d2b_cutover::OperationState::Held),
            terminal: self.operation.state().is_terminal(),
        }
    }

    fn failure(&self, error: d2b_cutover::RunnerSocketError) -> RunnerResponse {
        RunnerResponse {
            accepted: false,
            status: Some(self.status()),
            error: Some(error),
        }
    }
}

#[derive(Debug)]
enum RunnerError {
    NotRoot,
    Session,
    Bootstrap,
    Replay,
    Lock,
    Request,
    Journal,
    Socket,
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotRoot => "runner must start as uid 0",
            Self::Session => "runner could not create a new session",
            Self::Bootstrap => "bootstrap capability was rejected",
            Self::Replay => "bootstrap capability or journal was already consumed",
            Self::Lock => "operation lock was unavailable",
            Self::Request => "operation request was rejected",
            Self::Journal => "journal durability failed",
            Self::Socket => "runner socket failed",
        })
    }
}

impl std::error::Error for RunnerError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use d2b_cutover::{
        CandidateId, CutoverPreview, HoldReason, OperationRequest, OperationState, ResetInventory,
        ResetScope, RunnerSocketError,
    };

    fn runtime(label: &str) -> Runtime {
        let operation_id = d2b_cutover::OperationId::new("op-audit-boundary").expect("operation");
        let candidate_id = CandidateId::new("candidate-audit-boundary").expect("candidate");
        let revision_plan_id =
            d2b_cutover::RevisionPlanId::new("plan-audit-boundary").expect("plan");
        let operator_id =
            d2b_cutover::OperatorId::new("operator-audit-boundary").expect("operator");
        let inventory =
            ResetInventory::new(ResetScope::Zone, "zone-audit-boundary").expect("inventory");
        let preview = CutoverPreview::new_reset(
            operation_id.clone(),
            OperationKind::ScopedReset(ResetScope::Zone),
            candidate_id.clone(),
            revision_plan_id.clone(),
            inventory.clone(),
        )
        .expect("preview");
        let request = OperationRequest::new_reset(
            operation_id.clone(),
            ResetScope::Zone,
            candidate_id,
            revision_plan_id,
            operator_id.clone(),
            preview.digest().expect("preview digest"),
            inventory,
        )
        .expect("request");
        let capability = d2b_cutover::BootstrapCapability::new_with_identity(
            operation_id.clone(),
            request.candidate_id().clone(),
            operator_id,
            OperationKind::ScopedReset(ResetScope::Zone),
            d2b_cutover::Digest::derive("d2b:test:runner-audit", b"nonce"),
            100,
            200,
            1000,
            BTreeSet::new(),
        )
        .expect("capability");
        let bootstrap = RunnerBootstrap {
            capability,
            request: request.clone(),
            preview,
        };
        let mut ledger = CapabilityLedger::default();
        let capability_bytes = bootstrap
            .capability
            .canonical_bytes()
            .expect("capability bytes");
        let consumed = d2b_cutover::BootstrapCapability::decode_and_consume(
            &capability_bytes,
            150,
            &mut ledger,
        )
        .expect("consume capability");
        let root =
            PathBuf::from(".scratch").join(format!("runner-audit-{label}-{}", std::process::id()));
        let paths = RunnerPaths::new(&root, &operation_id);
        let lock = acquire_operation_lock(&paths).expect("lock");
        let socket = RunnerSocket::bind(&paths, consumed).expect("socket");
        let operation = Operation::new(request, &bootstrap.preview).expect("engine");
        persist_journal(
            &paths.journal,
            &bootstrap,
            &operation.journal_bytes().expect("initial journal"),
        )
        .expect("persist initial journal");
        Runtime {
            bootstrap,
            operation,
            _lock: lock,
            paths,
            socket,
        }
    }

    #[test]
    fn hold_refuses_without_privileged_audit_and_does_not_advance() {
        let mut runtime = runtime("hold");
        let response = runtime.handle(
            RunnerCommand::Hold {
                reason: "incident".to_owned(),
            },
            RunnerPeer::Owner,
        );
        assert!(!response.accepted);
        assert_eq!(response.error, Some(RunnerSocketError::AuditUnavailable));
        assert_eq!(runtime.operation.state(), OperationState::Planned);
        assert!(runtime.operation.journal().records().is_empty());
        let (_, records) = d2b_cutover::load_journal(&runtime.paths.journal).expect("journal");
        assert!(records.is_empty());
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn resume_refuses_without_privileged_audit_and_does_not_advance() {
        let mut runtime = runtime("resume");
        runtime
            .operation
            .request_hold(
                runtime.bootstrap.request.operator_id().clone(),
                HoldReason::new("setup").expect("reason"),
                // Test fixture only: establish a pre-existing held journal.
                AuditEvidence::durable("test-setup-audit").expect("setup evidence"),
            )
            .expect("setup hold");
        let journal_bytes = runtime
            .operation
            .journal_bytes()
            .expect("setup journal bytes");
        d2b_cutover::persist_journal(&runtime.paths.journal, &runtime.bootstrap, &journal_bytes)
            .unwrap_or_else(|error| panic!("persist setup hold failed: {error}"));
        let records_before = runtime.operation.journal().records().len();
        let (_, records_on_disk_before) =
            d2b_cutover::load_journal(&runtime.paths.journal).expect("journal before resume");
        let response = runtime.handle(
            RunnerCommand::Resume {
                fresh_consent: None,
            },
            RunnerPeer::Owner,
        );
        assert!(!response.accepted);
        assert_eq!(response.error, Some(RunnerSocketError::AuditUnavailable));
        assert_eq!(runtime.operation.state(), OperationState::Held);
        assert_eq!(runtime.operation.journal().records().len(), records_before);
        let (_, records_on_disk_after) =
            d2b_cutover::load_journal(&runtime.paths.journal).expect("journal after resume");
        assert_eq!(records_on_disk_after, records_on_disk_before);
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }
}
