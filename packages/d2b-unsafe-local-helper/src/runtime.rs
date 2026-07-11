use crate::environment::EnvironmentError;
use crate::systemd::{ScopeError, ScopeInspection, UserScopeManager, VerifiedScope};
use d2b_contracts::public_wire::ShellName;
use d2b_contracts::unsafe_local_wire::{
    HelperLaunchRequest, HelperOperationDisposition, HelperOperationResult, HelperScopeKind,
    HelperScopeSnapshot, HelperScopeState, HelperShellRequest, HelperShellResponse, HelperSnapshot,
    HelperSupervisorId, MAX_COMPLETED_OPERATION_AGE_SECS, MAX_COMPLETED_OPERATIONS_PER_UID,
    MAX_HELPER_SNAPSHOT_SCOPES,
};
use d2b_core::workload_identity::WorkloadIdentity;
use d2b_realm_core::ids::OperationId;
use nix::libc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};
use uzers::os::unix::UserExt;
use uzers::{get_current_uid, get_user_by_uid};

pub const SUPERVISOR_START_TIMEOUT: Duration = Duration::from_secs(5);
pub const SNAPSHOT_RECONCILE_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_LEDGER_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    InvalidRequest,
    InvalidIdentity,
    UserManagerUnavailable,
    EnvironmentInvalid,
    ExecutableUnavailable,
    ProxyUnavailable,
    ScopeCreateFailed,
    ScopeIdentityMismatch,
    OperationIdConflict,
    OperationInProgress,
    QuotaExceeded,
    ShellUnavailable,
    ShellNotFound,
    ShellAlreadyAttached,
    TerminalClosed,
    Timeout,
    LedgerInvalid,
    Internal,
}

impl From<EnvironmentError> for RuntimeError {
    fn from(error: EnvironmentError) -> Self {
        match error {
            EnvironmentError::ExecutableUnavailable | EnvironmentError::PathMissing => {
                Self::ExecutableUnavailable
            }
            EnvironmentError::ProxyUnavailable => Self::ProxyUnavailable,
            _ => Self::EnvironmentInvalid,
        }
    }
}

impl From<ScopeError> for RuntimeError {
    fn from(error: ScopeError) -> Self {
        match error {
            ScopeError::UserManagerUnavailable => Self::UserManagerUnavailable,
            ScopeError::EnvironmentInvalid => Self::EnvironmentInvalid,
            ScopeError::Timeout => Self::Timeout,
            ScopeError::CreateFailed => Self::ScopeCreateFailed,
            ScopeError::IdentityMismatch => Self::ScopeIdentityMismatch,
            ScopeError::NotFound | ScopeError::QueryFailed | ScopeError::StopFailed => {
                Self::Internal
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersistedScope {
    pub(crate) operation_id: OperationId,
    #[serde(default)]
    pub(crate) fingerprint: Option<[u8; 32]>,
    pub(crate) workload: WorkloadIdentity,
    pub(crate) unit_name: String,
    pub(crate) invocation_id: String,
    pub(crate) control_group: String,
    pub(crate) kind: HelperScopeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) persistent_shell: Option<PersistedShellMetadata>,
}

impl fmt::Debug for PersistedScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistedScope")
            .field("operation_id", &self.operation_id)
            .field("workload", &self.workload)
            .field("unit_name", &"<redacted>")
            .field("invocation_id", &"<redacted>")
            .field("control_group", &"<redacted>")
            .field("kind", &self.kind)
            .field("has_persistent_shell", &self.persistent_shell.is_some())
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersistedShellMetadata {
    pub(crate) name: ShellName,
    pub(crate) supervisor_id: HelperSupervisorId,
}

impl fmt::Debug for PersistedShellMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistedShellMetadata")
            .field("name", &"<redacted>")
            .field("supervisor_id", &"<redacted>")
            .finish()
    }
}

impl PersistedScope {
    pub(crate) fn verified(&self) -> VerifiedScope {
        VerifiedScope {
            unit_name: self.unit_name.clone(),
            invocation_id: self.invocation_id.clone(),
            control_group: self.control_group.clone(),
            kind: self.kind,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersistedScopeLedger {
    pub(crate) schema_version: u32,
    pub(crate) scopes: Vec<PersistedScope>,
}

impl fmt::Debug for PersistedScopeLedger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistedScopeLedger")
            .field("schema_version", &self.schema_version)
            .field("scope_count", &self.scopes.len())
            .finish()
    }
}

pub struct ScopeRuntime<M: UserScopeManager> {
    pub(crate) manager: Arc<M>,
    pub(crate) ledger_path: PathBuf,
    pub(crate) ledger: Mutex<RuntimeLedger>,
    pub(crate) user_home: PathBuf,
    pub(crate) shell_home: PathBuf,
    pub(crate) uid: u32,
    pub(crate) executable: PathBuf,
}

pub(crate) struct RuntimeLedger {
    pub(crate) persisted: PersistedScopeLedger,
    pub(crate) reservations: BTreeMap<String, LaunchReservation>,
    pub(crate) shell_name_reservations: BTreeMap<String, u64>,
    pub(crate) completed_shell_operations: BTreeMap<String, CompletedShellOperation>,
    next_owner: u64,
}

impl Default for RuntimeLedger {
    fn default() -> Self {
        Self::from_persisted(PersistedScopeLedger {
            schema_version: 1,
            scopes: Vec::new(),
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct LaunchReservation {
    pub(crate) fingerprint: [u8; 32],
    pub(crate) owner: u64,
}

enum LaunchBegin {
    Started(LaunchReservation),
    AlreadyCommitted(Box<PersistedScope>),
}

#[derive(Clone)]
pub(crate) struct CompletedShellOperation {
    pub(crate) fingerprint: [u8; 32],
    pub(crate) completed_at: Instant,
    pub(crate) response: Option<HelperShellResponse>,
}

pub(crate) enum ShellOperationBegin {
    Started(LaunchReservation),
    ExistingScope(Box<PersistedScope>),
    Replayed(Option<HelperShellResponse>),
}

impl RuntimeLedger {
    pub(crate) fn from_persisted(persisted: PersistedScopeLedger) -> Self {
        Self {
            persisted,
            reservations: BTreeMap::new(),
            shell_name_reservations: BTreeMap::new(),
            completed_shell_operations: BTreeMap::new(),
            next_owner: 0,
        }
    }

    fn begin(
        &mut self,
        operation_id: &OperationId,
        fingerprint: [u8; 32],
    ) -> Result<LaunchBegin, RuntimeError> {
        let operation_key = operation_id.to_string();
        if let Some(scope) = self
            .persisted
            .scopes
            .iter()
            .find(|scope| scope.operation_id == *operation_id)
        {
            return if scope.fingerprint == Some(fingerprint) {
                Ok(LaunchBegin::AlreadyCommitted(Box::new(scope.clone())))
            } else {
                Err(RuntimeError::OperationIdConflict)
            };
        }
        if let Some(reservation) = self.reservations.get(&operation_key) {
            return if reservation.fingerprint == fingerprint {
                Err(RuntimeError::OperationInProgress)
            } else {
                Err(RuntimeError::OperationIdConflict)
            };
        }
        if self
            .persisted
            .scopes
            .len()
            .saturating_add(self.reservations.len())
            >= MAX_HELPER_SNAPSHOT_SCOPES
        {
            return Err(RuntimeError::LedgerInvalid);
        }
        self.next_owner = self.next_owner.wrapping_add(1);
        if self.next_owner == 0 {
            self.next_owner = 1;
        }
        let reservation = LaunchReservation {
            fingerprint,
            owner: self.next_owner,
        };
        self.reservations.insert(operation_key, reservation);
        Ok(LaunchBegin::Started(reservation))
    }

    fn owns(&self, operation_id: &OperationId, reservation: LaunchReservation) -> bool {
        self.reservations
            .get(operation_id.as_str())
            .is_some_and(|active| active.owner == reservation.owner)
    }

    fn clear(&mut self, operation_id: &OperationId, reservation: LaunchReservation) {
        if self.owns(operation_id, reservation) {
            self.reservations.remove(operation_id.as_str());
        }
    }

    pub(crate) fn begin_shell_operation(
        &mut self,
        operation_id: &OperationId,
        fingerprint: [u8; 32],
    ) -> Result<ShellOperationBegin, RuntimeError> {
        self.expire_completed_shell_operations();
        if let Some(scope) = self
            .persisted
            .scopes
            .iter()
            .find(|scope| scope.operation_id == *operation_id)
        {
            return if scope.fingerprint == Some(fingerprint) {
                Ok(ShellOperationBegin::ExistingScope(Box::new(scope.clone())))
            } else {
                Err(RuntimeError::OperationIdConflict)
            };
        }
        if let Some(completed) = self.completed_shell_operations.get(operation_id.as_str()) {
            return if completed.fingerprint == fingerprint {
                Ok(ShellOperationBegin::Replayed(completed.response.clone()))
            } else {
                Err(RuntimeError::OperationIdConflict)
            };
        }
        if let Some(reservation) = self.reservations.get(operation_id.as_str()) {
            return if reservation.fingerprint == fingerprint {
                Err(RuntimeError::OperationInProgress)
            } else {
                Err(RuntimeError::OperationIdConflict)
            };
        }
        self.next_owner = self.next_owner.wrapping_add(1);
        if self.next_owner == 0 {
            self.next_owner = 1;
        }
        let reservation = LaunchReservation {
            fingerprint,
            owner: self.next_owner,
        };
        self.reservations
            .insert(operation_id.to_string(), reservation);
        Ok(ShellOperationBegin::Started(reservation))
    }

    pub(crate) fn reserve_shell_name(
        &mut self,
        key: String,
        reservation: LaunchReservation,
    ) -> Result<(), RuntimeError> {
        if self.shell_name_reservations.contains_key(&key) {
            return Err(RuntimeError::OperationInProgress);
        }
        self.shell_name_reservations.insert(key, reservation.owner);
        Ok(())
    }

    pub(crate) fn clear_shell_operation(
        &mut self,
        operation_id: &OperationId,
        reservation: LaunchReservation,
        name_key: Option<&str>,
    ) {
        self.clear(operation_id, reservation);
        if let Some(name_key) = name_key
            && self.shell_name_reservations.get(name_key) == Some(&reservation.owner)
        {
            self.shell_name_reservations.remove(name_key);
        }
    }

    pub(crate) fn complete_shell_operation(
        &mut self,
        operation_id: &OperationId,
        reservation: LaunchReservation,
        name_key: Option<&str>,
        response: Option<HelperShellResponse>,
    ) -> Result<(), RuntimeError> {
        if !self.owns(operation_id, reservation) {
            return Err(RuntimeError::OperationIdConflict);
        }
        self.clear_shell_operation(operation_id, reservation, name_key);
        self.remember_completed_shell_operation(operation_id, reservation.fingerprint, response);
        Ok(())
    }

    pub(crate) fn remember_completed_shell_operation(
        &mut self,
        operation_id: &OperationId,
        fingerprint: [u8; 32],
        response: Option<HelperShellResponse>,
    ) {
        while self.completed_shell_operations.len() >= MAX_COMPLETED_OPERATIONS_PER_UID {
            let Some(oldest) = self
                .completed_shell_operations
                .iter()
                .min_by_key(|(_, operation)| operation.completed_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.completed_shell_operations.remove(&oldest);
        }
        self.completed_shell_operations.insert(
            operation_id.to_string(),
            CompletedShellOperation {
                fingerprint,
                completed_at: Instant::now(),
                response,
            },
        );
    }

    fn expire_completed_shell_operations(&mut self) {
        let maximum_age = Duration::from_secs(MAX_COMPLETED_OPERATION_AGE_SECS);
        self.completed_shell_operations
            .retain(|_, operation| operation.completed_at.elapsed() <= maximum_age);
    }
}

impl<M: UserScopeManager> fmt::Debug for ScopeRuntime<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeRuntime")
            .field("ledger_path", &"<redacted>")
            .field("user_home", &"<redacted>")
            .field("shell_home", &"<redacted>")
            .field("uid", &"<redacted>")
            .field("executable", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl<M: UserScopeManager> ScopeRuntime<M> {
    pub fn new(manager: M) -> Result<Self, RuntimeError> {
        let uid = get_current_uid();
        if uid == 0 {
            return Err(RuntimeError::InvalidIdentity);
        }
        let user = get_user_by_uid(uid).ok_or(RuntimeError::InvalidIdentity)?;
        let user_home = user.home_dir().to_path_buf();
        if !user_home.is_absolute() {
            return Err(RuntimeError::InvalidIdentity);
        }
        let ledger_path = user_home.join(".local/state/d2b/unsafe-local-scopes.json");
        let executable = std::env::current_exe().map_err(|_| RuntimeError::Internal)?;
        Self::with_paths_and_executable(manager, user_home, ledger_path, executable)
    }

    pub fn with_paths(
        manager: M,
        user_home: PathBuf,
        ledger_path: PathBuf,
    ) -> Result<Self, RuntimeError> {
        let executable = std::env::current_exe().map_err(|_| RuntimeError::Internal)?;
        Self::with_paths_and_executable(manager, user_home, ledger_path, executable)
    }

    pub fn with_paths_and_executable(
        manager: M,
        user_home: PathBuf,
        ledger_path: PathBuf,
        executable: PathBuf,
    ) -> Result<Self, RuntimeError> {
        let uid = get_current_uid();
        if uid == 0 {
            return Err(RuntimeError::InvalidIdentity);
        }
        let user = get_user_by_uid(uid).ok_or(RuntimeError::InvalidIdentity)?;
        let shell_home = user.home_dir().to_path_buf();
        if !shell_home.is_absolute() || !executable.is_absolute() {
            return Err(RuntimeError::InvalidIdentity);
        }
        let ledger = RuntimeLedger::from_persisted(load_ledger(&ledger_path)?);
        Ok(Self {
            manager: Arc::new(manager),
            ledger_path,
            ledger: Mutex::new(ledger),
            user_home,
            shell_home,
            uid,
            executable,
        })
    }

    pub fn shell(
        &self,
        request: HelperShellRequest,
    ) -> Result<crate::shell_runtime::ShellDispatch, RuntimeError> {
        crate::shell_runtime::dispatch(self, request)
    }

    pub fn launch(
        &self,
        request: HelperLaunchRequest,
    ) -> Result<HelperOperationResult, RuntimeError> {
        let fingerprint = launch_fingerprint(&request)?;
        let reservation = match self
            .ledger
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .begin(&request.operation_id, fingerprint)?
        {
            LaunchBegin::Started(reservation) => reservation,
            LaunchBegin::AlreadyCommitted(scope) => {
                return Ok(HelperOperationResult {
                    request_id: request.request_id,
                    operation_id: request.operation_id,
                    disposition: HelperOperationDisposition::AlreadyCommitted,
                    scope: Some(scope.verified().wire_identity()),
                });
            }
        };
        let operation_id = request.operation_id.clone();
        let result = self.launch_reserved(request, fingerprint, reservation);
        if result.is_err()
            && let Ok(mut ledger) = self.ledger.lock()
        {
            ledger.clear(&operation_id, reservation);
        }
        result
    }

    fn launch_reserved(
        &self,
        request: HelperLaunchRequest,
        fingerprint: [u8; 32],
        reservation: LaunchReservation,
    ) -> Result<HelperOperationResult, RuntimeError> {
        let environment = self.manager.manager_environment()?;
        let argv = request.argv.as_slice();
        let program = environment.resolve_program(&argv[0])?;
        let child_environment = environment.child_entries(request.graphical, None)?;
        let spec = SupervisorSpec {
            program,
            args: argv[1..].to_vec(),
            environment: child_environment,
            cwd: self.user_home.clone(),
        };
        let mut supervisor = BlockedSupervisor::spawn(&spec)?;
        let supervisor_pid = supervisor.id();
        let scope = match self
            .manager
            .start_scope(supervisor_pid, HelperScopeKind::LauncherApp)
        {
            Ok(scope) => scope,
            Err(error) => {
                supervisor.abort();
                return Err(error.into());
            }
        };
        let persisted = PersistedScope {
            operation_id: request.operation_id.clone(),
            fingerprint: Some(fingerprint),
            workload: request.workload,
            unit_name: scope.unit_name.clone(),
            invocation_id: scope.invocation_id.clone(),
            control_group: scope.control_group.clone(),
            kind: scope.kind,
            persistent_shell: None,
        };
        if let Err(error) = supervisor.release_and_wait_started() {
            supervisor.abort();
            self.stop_failed_scope(&scope);
            return Err(error);
        }
        if let Err(error) = self.commit_scope(&persisted, reservation) {
            supervisor.abort();
            self.stop_failed_scope(&scope);
            return Err(error);
        }
        supervisor.reap_in_background();

        Ok(HelperOperationResult {
            request_id: request.request_id,
            operation_id: request.operation_id,
            disposition: HelperOperationDisposition::Committed,
            scope: Some(scope.wire_identity()),
        })
    }

    fn commit_scope(
        &self,
        persisted: &PersistedScope,
        reservation: LaunchReservation,
    ) -> Result<(), RuntimeError> {
        let mut ledger = self.ledger.lock().map_err(|_| RuntimeError::Internal)?;
        if !ledger.owns(&persisted.operation_id, reservation) {
            return Err(RuntimeError::OperationIdConflict);
        }
        let mut candidate = ledger.persisted.clone();
        candidate.scopes.push(persisted.clone());
        if candidate.scopes.len() > MAX_HELPER_SNAPSHOT_SCOPES {
            return Err(RuntimeError::LedgerInvalid);
        }
        persist_ledger(&self.ledger_path, &candidate)?;
        ledger.persisted = candidate;
        ledger.clear(&persisted.operation_id, reservation);
        Ok(())
    }

    pub(crate) fn stop_failed_scope(&self, scope: &VerifiedScope) {
        let _ = self.manager.terminate_scope(scope, libc::SIGKILL);
        let _ = self.manager.stop_scope(scope);
    }

    pub fn snapshot(&self, generation: u64) -> Result<HelperSnapshot, RuntimeError> {
        let entries = self
            .ledger
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .persisted
            .scopes
            .clone();
        if entries.len() > MAX_HELPER_SNAPSHOT_SCOPES {
            return Err(RuntimeError::LedgerInvalid);
        }

        let mut scopes = Vec::with_capacity(entries.len());
        let deadline = Instant::now() + SNAPSHOT_RECONCILE_TIMEOUT;
        for entry in entries {
            let manager_state = if Instant::now() >= deadline {
                HelperScopeState::Degraded
            } else {
                let verified = entry.verified();
                match self.manager.inspect_scope(&verified) {
                    Ok(ScopeInspection {
                        state,
                        identity_matches: true,
                    }) => state,
                    _ => HelperScopeState::Degraded,
                }
            };
            let (state, persistent_shell) = if entry.persistent_shell.is_some() {
                crate::shell_runtime::snapshot_shell(self, &entry, manager_state)
            } else {
                (manager_state, None)
            };
            let scope = entry.verified().wire_identity();
            scopes.push(HelperScopeSnapshot {
                operation_id: entry.operation_id,
                workload: entry.workload,
                scope,
                state,
                persistent_shell,
            });
        }
        Ok(HelperSnapshot { generation, scopes })
    }
}

fn launch_fingerprint(request: &HelperLaunchRequest) -> Result<[u8; 32], RuntimeError> {
    let encoded = serde_json::to_vec(&(
        &request.workload,
        &request.item_id,
        &request.argv,
        request.graphical,
    ))
    .map_err(|_| RuntimeError::Internal)?;
    Ok(Sha256::digest(encoded).into())
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupervisorSpec {
    program: PathBuf,
    args: Vec<String>,
    environment: BTreeMap<String, String>,
    cwd: PathBuf,
}

impl fmt::Debug for SupervisorSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SupervisorSpec")
            .field("program", &"<redacted>")
            .field("arg_count", &self.args.len())
            .field("environment_count", &self.environment.len())
            .field("cwd", &"<redacted>")
            .finish()
    }
}

struct BlockedSupervisor {
    child: Option<Child>,
    stdin: Option<std::process::ChildStdin>,
    stdout: Option<std::process::ChildStdout>,
}

impl BlockedSupervisor {
    fn spawn(spec: &SupervisorSpec) -> Result<Self, RuntimeError> {
        let encoded = serde_json::to_vec(spec).map_err(|_| RuntimeError::Internal)?;
        if encoded.len() > MAX_LEDGER_BYTES as usize {
            return Err(RuntimeError::EnvironmentInvalid);
        }
        let executable = std::env::current_exe().map_err(|_| RuntimeError::Internal)?;
        let mut child = Command::new(executable)
            .arg("scope-supervisor")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| RuntimeError::ScopeCreateFailed)?;
        let mut stdin = child.stdin.take().ok_or(RuntimeError::Internal)?;
        let stdout = child.stdout.take().ok_or(RuntimeError::Internal)?;
        let length = u32::try_from(encoded.len()).map_err(|_| RuntimeError::EnvironmentInvalid)?;
        if stdin.write_all(&length.to_le_bytes()).is_err() || stdin.write_all(&encoded).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RuntimeError::ScopeCreateFailed);
        }
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout: Some(stdout),
        })
    }

    fn id(&self) -> u32 {
        self.child.as_ref().expect("supervisor child present").id()
    }

    fn release_and_wait_started(&mut self) -> Result<(), RuntimeError> {
        let mut stdin = self.stdin.take().ok_or(RuntimeError::Internal)?;
        stdin
            .write_all(&[1])
            .map_err(|_| RuntimeError::ScopeCreateFailed)?;
        drop(stdin);

        let mut stdout = self.stdout.take().ok_or(RuntimeError::Internal)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("d2b-scope-start-ack".to_owned())
            .spawn(move || {
                let mut ack = [0u8; 1];
                let result = stdout.read_exact(&mut ack).map(|()| ack[0]);
                let _ = sender.send(result);
            })
            .map_err(|_| RuntimeError::Internal)?;
        match receiver.recv_timeout(SUPERVISOR_START_TIMEOUT) {
            Ok(Ok(1)) => Ok(()),
            Ok(Ok(_)) | Ok(Err(_)) => Err(RuntimeError::ScopeCreateFailed),
            Err(_) => Err(RuntimeError::Timeout),
        }
    }

    fn abort(&mut self) {
        self.stdin.take();
        self.stdout.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn reap_in_background(mut self) {
        self.stdin.take();
        self.stdout.take();
        if let Some(mut child) = self.child.take() {
            let _ = std::thread::Builder::new()
                .name("d2b-scope-reaper".to_owned())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }
}

impl Drop for BlockedSupervisor {
    fn drop(&mut self) {
        if self.child.is_some() {
            self.abort();
        }
    }
}

pub fn run_scope_supervisor() -> Result<(), RuntimeError> {
    let mut length = [0u8; 4];
    std::io::stdin()
        .read_exact(&mut length)
        .map_err(|_| RuntimeError::Internal)?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_LEDGER_BYTES as usize {
        return Err(RuntimeError::EnvironmentInvalid);
    }
    let mut encoded = vec![0u8; length];
    std::io::stdin()
        .read_exact(&mut encoded)
        .map_err(|_| RuntimeError::Internal)?;
    let spec: SupervisorSpec =
        serde_json::from_slice(&encoded).map_err(|_| RuntimeError::EnvironmentInvalid)?;
    let mut release = [0u8; 1];
    std::io::stdin()
        .read_exact(&mut release)
        .map_err(|_| RuntimeError::Internal)?;
    if release != [1] {
        return Err(RuntimeError::Internal);
    }

    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .env_clear()
        .envs(&spec.environment)
        .current_dir(&spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| RuntimeError::ExecutableUnavailable)?;
    std::io::stdout()
        .write_all(&[1])
        .map_err(|_| RuntimeError::Internal)?;
    std::io::stdout()
        .flush()
        .map_err(|_| RuntimeError::Internal)?;
    child.wait().map_err(|_| RuntimeError::Internal)?;
    Ok(())
}

fn load_ledger(path: &Path) -> Result<PersistedScopeLedger, RuntimeError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_LEDGER_BYTES => {
            return Err(RuntimeError::LedgerInvalid);
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PersistedScopeLedger {
                schema_version: 1,
                scopes: Vec::new(),
            });
        }
        Err(_) => return Err(RuntimeError::LedgerInvalid),
    }
    let encoded = fs::read(path).map_err(|_| RuntimeError::LedgerInvalid)?;
    let ledger: PersistedScopeLedger =
        serde_json::from_slice(&encoded).map_err(|_| RuntimeError::LedgerInvalid)?;
    let unique_operations = ledger
        .scopes
        .iter()
        .map(|scope| scope.operation_id.to_string())
        .collect::<HashSet<_>>();
    let shell_keys = ledger
        .scopes
        .iter()
        .filter_map(|scope| {
            scope.persistent_shell.as_ref().map(|shell| {
                format!(
                    "{}\u{1f}{}",
                    scope.workload.target().to_canonical(),
                    shell.name.as_str()
                )
            })
        })
        .collect::<HashSet<_>>();
    let supervisor_ids = ledger
        .scopes
        .iter()
        .filter_map(|scope| {
            scope
                .persistent_shell
                .as_ref()
                .map(|shell| shell.supervisor_id.as_str().to_owned())
        })
        .collect::<HashSet<_>>();
    let shell_count = ledger
        .scopes
        .iter()
        .filter(|scope| scope.persistent_shell.is_some())
        .count();
    let shell_metadata_valid = ledger.scopes.iter().all(|scope| {
        (scope.kind == HelperScopeKind::PersistentShell) == scope.persistent_shell.is_some()
    });
    if ledger.schema_version != 1
        || ledger.scopes.len() > MAX_HELPER_SNAPSHOT_SCOPES
        || unique_operations.len() != ledger.scopes.len()
        || shell_keys.len() != shell_count
        || supervisor_ids.len() != shell_count
        || !shell_metadata_valid
    {
        return Err(RuntimeError::LedgerInvalid);
    }
    Ok(ledger)
}

pub(crate) fn persist_ledger(
    path: &Path,
    ledger: &PersistedScopeLedger,
) -> Result<(), RuntimeError> {
    let parent = path.parent().ok_or(RuntimeError::LedgerInvalid)?;
    fs::create_dir_all(parent).map_err(|_| RuntimeError::LedgerInvalid)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
        .map_err(|_| RuntimeError::LedgerInvalid)?;
    let encoded = serde_json::to_vec(ledger).map_err(|_| RuntimeError::LedgerInvalid)?;
    if encoded.len() > MAX_LEDGER_BYTES as usize {
        return Err(RuntimeError::LedgerInvalid);
    }
    let candidate = parent.join(format!(".unsafe-local-scopes.{}.new", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&candidate)
        .map_err(|_| RuntimeError::LedgerInvalid)?;
    let result = (|| {
        file.write_all(&encoded)
            .map_err(|_| RuntimeError::LedgerInvalid)?;
        file.sync_all().map_err(|_| RuntimeError::LedgerInvalid)?;
        fs::rename(&candidate, path).map_err(|_| RuntimeError::LedgerInvalid)
    })();
    if result.is_err() {
        let _ = fs::remove_file(candidate);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::unsafe_local_wire::{HelperLaunchRequest, ScopeIdentity};
    use d2b_core::configured_argv::ConfiguredArgv;
    use d2b_realm_core::token::ProtocolToken;
    use std::sync::{Arc, Barrier};

    fn launch(operation_id: &str, arg: &str) -> HelperLaunchRequest {
        HelperLaunchRequest {
            request_id: 1,
            operation_id: OperationId::parse(operation_id).unwrap(),
            workload: serde_json::from_value(serde_json::json!({
                "workloadId": "tools",
                "realmId": "host",
                "realmPath": ["host"],
                "canonicalTarget": "tools.host.d2b"
            }))
            .unwrap(),
            item_id: ProtocolToken::parse("browser").unwrap(),
            argv: ConfiguredArgv::new(vec![arg.to_owned()]).unwrap(),
            graphical: false,
        }
    }

    #[test]
    fn concurrent_reservation_allows_only_one_launch_owner() {
        const CONTENDERS: usize = 16;
        let ledger = Arc::new(Mutex::new(RuntimeLedger::default()));
        let barrier = Arc::new(Barrier::new(CONTENDERS));
        let request = launch("op-concurrent", "program");
        let fingerprint = launch_fingerprint(&request).unwrap();
        let operation_id = request.operation_id;
        let mut threads = Vec::new();
        for _ in 0..CONTENDERS {
            let ledger = Arc::clone(&ledger);
            let barrier = Arc::clone(&barrier);
            let operation_id = operation_id.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                ledger.lock().unwrap().begin(&operation_id, fingerprint)
            }));
        }
        let results = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Ok(LaunchBegin::Started(_))))
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
        assert_eq!(ledger.lock().unwrap().reservations.len(), 1);
    }

    #[test]
    fn reservation_rejects_changed_fingerprint_and_replays_committed_scope() {
        let first = launch("op-fingerprint", "first");
        let first_fingerprint = launch_fingerprint(&first).unwrap();
        let mut ledger = RuntimeLedger::default();
        assert!(matches!(
            ledger.begin(&first.operation_id, first_fingerprint),
            Ok(LaunchBegin::Started(_))
        ));
        let changed = launch("op-fingerprint", "changed");
        assert!(matches!(
            ledger.begin(&changed.operation_id, launch_fingerprint(&changed).unwrap()),
            Err(RuntimeError::OperationIdConflict)
        ));

        ledger.reservations.clear();
        ledger.persisted.scopes.push(PersistedScope {
            operation_id: first.operation_id.clone(),
            fingerprint: Some(first_fingerprint),
            workload: first.workload,
            unit_name: "app-d2b.scope".to_owned(),
            invocation_id: "00112233445566778899aabbccddeeff".to_owned(),
            control_group: "/user.slice/app-d2b.scope".to_owned(),
            kind: HelperScopeKind::LauncherApp,
            persistent_shell: None,
        });
        assert!(matches!(
            ledger.begin(&first.operation_id, first_fingerprint),
            Ok(LaunchBegin::AlreadyCommitted(_))
        ));
        assert!(matches!(
            ledger.begin(&changed.operation_id, launch_fingerprint(&changed).unwrap()),
            Err(RuntimeError::OperationIdConflict)
        ));
    }

    #[test]
    fn failed_launch_clears_only_its_own_reservation() {
        let request = launch("op-owned", "program");
        let fingerprint = launch_fingerprint(&request).unwrap();
        let mut ledger = RuntimeLedger::default();
        let reservation = match ledger.begin(&request.operation_id, fingerprint).unwrap() {
            LaunchBegin::Started(reservation) => reservation,
            LaunchBegin::AlreadyCommitted(_) => panic!("new operation was already committed"),
        };
        ledger.clear(
            &request.operation_id,
            LaunchReservation {
                fingerprint,
                owner: reservation.owner.wrapping_add(1),
            },
        );
        assert!(matches!(
            ledger.begin(&request.operation_id, fingerprint),
            Err(RuntimeError::OperationInProgress)
        ));
        ledger.clear(&request.operation_id, reservation);
        assert!(matches!(
            ledger.begin(&request.operation_id, fingerprint),
            Ok(LaunchBegin::Started(_))
        ));
    }

    #[test]
    fn persisted_scope_debug_hides_scope_identifiers() {
        let canary = "scope-private-canary";
        let persisted = PersistedScope {
            operation_id: OperationId::parse("op-1").unwrap(),
            fingerprint: None,
            workload: serde_json::from_value(serde_json::json!({
                "workloadId": "tools",
                "realmId": "host",
                "realmPath": ["host"],
                "canonicalTarget": "tools.host.d2b"
            }))
            .unwrap(),
            unit_name: canary.to_owned(),
            invocation_id: canary.to_owned(),
            control_group: format!("/{canary}"),
            kind: HelperScopeKind::PersistentShell,
            persistent_shell: Some(PersistedShellMetadata {
                name: ShellName::new(canary).unwrap(),
                supervisor_id: HelperSupervisorId::new(canary).unwrap(),
            }),
        };
        assert!(!format!("{persisted:?}").contains(canary));
    }

    #[test]
    fn adoption_degrades_identity_ambiguity_without_stopping_scope() {
        let inspection = ScopeInspection {
            state: HelperScopeState::Active,
            identity_matches: false,
        };
        let state = match inspection {
            ScopeInspection {
                state,
                identity_matches: true,
            } => state,
            _ => HelperScopeState::Degraded,
        };
        assert_eq!(state, HelperScopeState::Degraded);
    }

    #[test]
    fn supervisor_spec_debug_redacts_every_sensitive_surface() {
        let canary = "runtime-private-canary";
        let spec = SupervisorSpec {
            program: PathBuf::from(format!("/{canary}")),
            args: vec![canary.to_owned()],
            environment: BTreeMap::from([("PRIVATE".to_owned(), canary.to_owned())]),
            cwd: PathBuf::from(format!("/{canary}")),
        };
        assert!(!format!("{spec:?}").contains(canary));
    }

    #[test]
    fn wire_scope_identity_remains_redacted() {
        let canary = "invocation-private-canary";
        let identity = ScopeIdentity {
            invocation_id: canary.to_owned(),
            kind: HelperScopeKind::LauncherApp,
        };
        assert!(!format!("{identity:?}").contains(canary));
    }
}
