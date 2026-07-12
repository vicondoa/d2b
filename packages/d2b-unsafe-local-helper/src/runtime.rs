use crate::environment::EnvironmentError;
use crate::shell_socket::validate_runtime_directory;
use crate::systemd::{ScopeError, ScopeInspection, UserScopeManager, VerifiedScope};
use d2b_contracts::public_wire::ShellName;
use d2b_contracts::unsafe_local_wire::{
    HelperLaunchRequest, HelperOperationDisposition, HelperOperationResult, HelperScopeKind,
    HelperScopeSnapshot, HelperScopeState, HelperShellRequest, HelperShellResponse, HelperSnapshot,
    HelperSupervisorId, MAX_COMPLETED_OPERATION_AGE_SECS, MAX_COMPLETED_OPERATIONS_PER_UID,
    MAX_HELPER_SNAPSHOT_SCOPES, RealmAccentColor,
};
use d2b_core::workload_identity::WorkloadIdentity;
use d2b_realm_core::{WorkloadProviderKind, ids::OperationId};
use d2b_wayland_proxy::readiness::{
    ProxyReadinessEvent, ProxyReadinessStage, ProxyReadinessState, READINESS_PROTOCOL_VERSION,
};
use nix::libc;
use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};
use uzers::os::unix::UserExt;
use uzers::{get_current_uid, get_user_by_uid};

pub const SUPERVISOR_START_TIMEOUT: Duration = Duration::from_secs(25);
pub const SNAPSHOT_RECONCILE_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_LEDGER_BYTES: u64 = 1024 * 1024;
const PROXY_READY_TIMEOUT: Duration = Duration::from_secs(5);
const FIRST_CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_READINESS_EVENT_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    InvalidRequest,
    InvalidIdentity,
    UserManagerUnavailable,
    EnvironmentInvalid,
    ExecutableUnavailable,
    ProxyUnavailable,
    WaylandUnavailable,
    FirstClientTimeout,
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
            EnvironmentError::WaylandUnavailable => Self::WaylandUnavailable,
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
    pub(crate) wayland_proxy_binary: Option<PathBuf>,
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
            .field(
                "wayland_proxy_configured",
                &self.wayland_proxy_binary.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl<M: UserScopeManager> ScopeRuntime<M> {
    pub fn new(manager: M, wayland_proxy_binary: PathBuf) -> Result<Self, RuntimeError> {
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
        validate_immutable_proxy_binary(&wayland_proxy_binary)?;
        Self::with_paths_executable_and_proxy(
            manager,
            user_home,
            ledger_path,
            executable,
            Some(wayland_proxy_binary),
        )
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
        Self::with_paths_executable_and_proxy(manager, user_home, ledger_path, executable, None)
    }

    pub(crate) fn with_paths_executable_and_proxy(
        manager: M,
        user_home: PathBuf,
        ledger_path: PathBuf,
        executable: PathBuf,
        wayland_proxy_binary: Option<PathBuf>,
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
            wayland_proxy_binary,
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
        let graphical = if request.graphical {
            let runtime_directory = environment.runtime_directory()?;
            validate_runtime_directory(&runtime_directory, self.uid)
                .map_err(|_| RuntimeError::EnvironmentInvalid)?;
            let wayland_proxy_binary = self
                .wayland_proxy_binary
                .clone()
                .ok_or(RuntimeError::ProxyUnavailable)?;
            Some(GraphicalSupervisorSpec::new(
                wayland_proxy_binary,
                runtime_directory,
                environment.wayland_display()?.to_owned(),
                request.workload.target().clone(),
                request.realm_accent_color.clone(),
                self.uid,
            )?)
        } else {
            None
        };
        let child_environment = environment.child_entries(
            request.graphical,
            graphical.as_ref().map(|g| g.display.as_str()),
        )?;
        let spec = SupervisorSpec {
            program,
            args: argv[1..].to_vec(),
            environment: child_environment,
            cwd: self.user_home.clone(),
            graphical,
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
        &request.realm_accent_color,
    ))
    .map_err(|_| RuntimeError::Internal)?;
    Ok(Sha256::digest(encoded).into())
}

fn validate_immutable_proxy_binary(path: &Path) -> Result<(), RuntimeError> {
    if !path.is_absolute()
        || !path.starts_with("/nix/store")
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(RuntimeError::ProxyUnavailable);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| RuntimeError::ProxyUnavailable)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(RuntimeError::ProxyUnavailable);
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupervisorSpec {
    program: PathBuf,
    args: Vec<String>,
    environment: BTreeMap<String, String>,
    cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graphical: Option<GraphicalSupervisorSpec>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GraphicalSupervisorSpec {
    proxy_binary: PathBuf,
    runtime_directory: PathBuf,
    display: String,
    upstream_display: String,
    target: d2b_core::workload_identity::WorkloadTarget,
    realm_accent_color: RealmAccentColor,
    uid: u32,
    first_client_timeout_ms: u64,
}

impl GraphicalSupervisorSpec {
    fn new(
        proxy_binary: PathBuf,
        runtime_directory: PathBuf,
        upstream_display: String,
        target: d2b_core::workload_identity::WorkloadTarget,
        realm_accent_color: RealmAccentColor,
        uid: u32,
    ) -> Result<Self, RuntimeError> {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).map_err(|_| RuntimeError::Internal)?;
        let display = format!("d2b-unsafe-local-{}/wayland.sock", hex(&random));
        let spec = Self {
            proxy_binary,
            runtime_directory,
            display,
            upstream_display,
            target,
            realm_accent_color,
            uid,
            first_client_timeout_ms: FIRST_CLIENT_TIMEOUT.as_millis() as u64,
        };
        spec.validate()?;
        Ok(spec)
    }

    fn validate(&self) -> Result<(), RuntimeError> {
        if !self.proxy_binary.is_absolute()
            || !self.runtime_directory.is_absolute()
            || self.upstream_display.is_empty()
            || self.upstream_display.contains('\0')
            || self
                .upstream_display
                .split('/')
                .any(|component| component == "..")
            || self.uid == 0
            || self.first_client_timeout_ms == 0
            || self.first_client_timeout_ms > FIRST_CLIENT_TIMEOUT.as_millis() as u64
        {
            return Err(RuntimeError::EnvironmentInvalid);
        }
        crate::environment::valid_proxy_display(&self.display)
            .then_some(())
            .ok_or(RuntimeError::ProxyUnavailable)
    }

    fn private_directory(&self) -> Result<PathBuf, RuntimeError> {
        self.validate()?;
        let directory = self
            .display
            .split_once('/')
            .map(|(directory, _)| directory)
            .ok_or(RuntimeError::EnvironmentInvalid)?;
        Ok(self.runtime_directory.join(directory))
    }

    fn wayland_socket(&self) -> Result<PathBuf, RuntimeError> {
        Ok(self.runtime_directory.join(&self.display))
    }

    fn readiness_socket(&self) -> Result<PathBuf, RuntimeError> {
        Ok(self.private_directory()?.join("readiness.sock"))
    }
}

impl fmt::Debug for SupervisorSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SupervisorSpec")
            .field("program", &"<redacted>")
            .field("arg_count", &self.args.len())
            .field("environment_count", &self.environment.len())
            .field("cwd", &"<redacted>")
            .field("graphical", &self.graphical.is_some())
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

    run_supervisor_spec(spec, &mut std::io::stdout())
}

fn run_supervisor_spec(
    spec: SupervisorSpec,
    started_ack: &mut impl Write,
) -> Result<(), RuntimeError> {
    match spec.graphical.clone() {
        Some(graphical) => run_graphical_supervisor(&spec, &graphical, started_ack),
        None => run_plain_supervisor(&spec, started_ack),
    }
}

fn spawn_app(spec: &SupervisorSpec) -> Result<Child, RuntimeError> {
    Command::new(&spec.program)
        .args(&spec.args)
        .env_clear()
        .envs(&spec.environment)
        .current_dir(&spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| RuntimeError::ExecutableUnavailable)
}

fn run_plain_supervisor(
    spec: &SupervisorSpec,
    started_ack: &mut impl Write,
) -> Result<(), RuntimeError> {
    let mut child = spawn_app(spec)?;
    started_ack
        .write_all(&[1])
        .and_then(|()| started_ack.flush())
        .map_err(|_| RuntimeError::Internal)?;
    child.wait().map_err(|_| RuntimeError::Internal)?;
    Ok(())
}

fn run_graphical_supervisor(
    spec: &SupervisorSpec,
    graphical: &GraphicalSupervisorSpec,
    started_ack: &mut impl Write,
) -> Result<(), RuntimeError> {
    graphical.validate()?;
    validate_runtime_directory(&graphical.runtime_directory, graphical.uid)
        .map_err(|_| RuntimeError::EnvironmentInvalid)?;
    let runtime = PrivateGraphicalRuntime::prepare(graphical)?;
    let mut proxy = spawn_proxy(graphical)?;
    let mut app = None;
    let startup = (|| {
        let mut readiness = ReadinessChannel::new(runtime.readiness_listener(), graphical.uid)?;
        readiness.expect(
            ProxyReadinessStage::Upstream,
            graphical,
            Instant::now() + PROXY_READY_TIMEOUT,
            &mut proxy,
            None,
        )?;
        readiness.expect(
            ProxyReadinessStage::Listener,
            graphical,
            Instant::now() + PROXY_READY_TIMEOUT,
            &mut proxy,
            None,
        )?;
        app = Some(spawn_app(spec)?);
        readiness.expect(
            ProxyReadinessStage::FirstClient,
            graphical,
            Instant::now() + Duration::from_millis(graphical.first_client_timeout_ms),
            &mut proxy,
            app.as_mut(),
        )?;
        started_ack
            .write_all(&[1])
            .and_then(|()| started_ack.flush())
            .map_err(|_| RuntimeError::Internal)
    })();
    if let Err(error) = startup {
        if let Some(child) = app.as_mut() {
            terminate_and_reap(child);
        }
        terminate_and_reap(&mut proxy);
        return Err(error);
    }

    wait_for_graphical_exit(app.as_mut().ok_or(RuntimeError::Internal)?, &mut proxy)
}

fn spawn_proxy(graphical: &GraphicalSupervisorSpec) -> Result<Child, RuntimeError> {
    Command::new(&graphical.proxy_binary)
        .args(proxy_arguments(graphical)?)
        .env_clear()
        .env("XDG_RUNTIME_DIR", &graphical.runtime_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| RuntimeError::ProxyUnavailable)
}

fn proxy_arguments(graphical: &GraphicalSupervisorSpec) -> Result<Vec<OsString>, RuntimeError> {
    Ok(vec![
        "--listen".into(),
        graphical.wayland_socket()?.into_os_string(),
        "--connect".into(),
        graphical.upstream_display.clone().into(),
        "--target".into(),
        graphical.target.to_canonical().into(),
        "--provider-kind".into(),
        "unsafe-local".into(),
        "--border-enable".into(),
        "--border-color-active".into(),
        graphical.realm_accent_color.as_str().into(),
        "--readiness-socket".into(),
        graphical.readiness_socket()?.into_os_string(),
        "--first-client-timeout-ms".into(),
        graphical.first_client_timeout_ms.to_string().into(),
        "--clipd-bridge-user-uid".into(),
        graphical.uid.to_string().into(),
    ])
}

fn terminate_and_reap(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn wait_for_graphical_exit(app: &mut Child, proxy: &mut Child) -> Result<(), RuntimeError> {
    let app_pid = Pid::from_raw(i32::try_from(app.id()).map_err(|_| RuntimeError::Internal)?);
    let proxy_pid = Pid::from_raw(i32::try_from(proxy.id()).map_err(|_| RuntimeError::Internal)?);
    loop {
        match waitpid(None, None) {
            Ok(WaitStatus::Exited(pid, _) | WaitStatus::Signaled(pid, _, _)) if pid == app_pid => {
                terminate_and_reap(proxy);
                return Ok(());
            }
            Ok(WaitStatus::Exited(pid, _) | WaitStatus::Signaled(pid, _, _))
                if pid == proxy_pid =>
            {
                terminate_and_reap(app);
                return Ok(());
            }
            Ok(
                WaitStatus::StillAlive
                | WaitStatus::Continued(_)
                | WaitStatus::Stopped(_, _)
                | WaitStatus::PtraceEvent(_, _, _)
                | WaitStatus::PtraceSyscall(_),
            ) => {}
            Ok(_) => return Err(RuntimeError::Internal),
            Err(nix::errno::Errno::EINTR) => {}
            Err(_) => return Err(RuntimeError::Internal),
        }
    }
}

struct PrivateGraphicalRuntime {
    _directory: PrivateGraphicalDirectory,
    readiness_listener: UnixListener,
}

impl PrivateGraphicalRuntime {
    fn prepare(spec: &GraphicalSupervisorSpec) -> Result<Self, RuntimeError> {
        let directory = PrivateGraphicalDirectory::prepare(spec)?;
        let readiness_path = spec.readiness_socket()?;
        let readiness_listener =
            UnixListener::bind(&readiness_path).map_err(|_| RuntimeError::ProxyUnavailable)?;
        fs::set_permissions(&readiness_path, fs::Permissions::from_mode(0o600))
            .map_err(|_| RuntimeError::ProxyUnavailable)?;
        let metadata =
            fs::symlink_metadata(&readiness_path).map_err(|_| RuntimeError::ProxyUnavailable)?;
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        if !metadata.file_type().is_socket()
            || metadata.uid() != spec.uid
            || metadata.permissions().mode() & 0o7777 != 0o600
        {
            return Err(RuntimeError::ProxyUnavailable);
        }
        let flags = rustix::io::fcntl_getfd(&readiness_listener)
            .map_err(|_| RuntimeError::ProxyUnavailable)?;
        rustix::io::fcntl_setfd(&readiness_listener, flags | rustix::io::FdFlags::CLOEXEC)
            .map_err(|_| RuntimeError::ProxyUnavailable)?;
        readiness_listener
            .set_nonblocking(true)
            .map_err(|_| RuntimeError::ProxyUnavailable)?;
        Ok(Self {
            _directory: directory,
            readiness_listener,
        })
    }

    fn readiness_listener(&self) -> &UnixListener {
        &self.readiness_listener
    }
}

struct PrivateGraphicalDirectory {
    path: PathBuf,
}

impl PrivateGraphicalDirectory {
    fn prepare(spec: &GraphicalSupervisorSpec) -> Result<Self, RuntimeError> {
        let directory = spec.private_directory()?;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(&directory)
            .map_err(|_| RuntimeError::ProxyUnavailable)?;
        let result = (|| {
            let metadata =
                fs::symlink_metadata(&directory).map_err(|_| RuntimeError::ProxyUnavailable)?;
            use std::os::unix::fs::MetadataExt;
            if !metadata.file_type().is_dir()
                || metadata.file_type().is_symlink()
                || metadata.uid() != spec.uid
                || metadata.permissions().mode() & 0o7777 != 0o700
            {
                return Err(RuntimeError::ProxyUnavailable);
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(Self { path: directory }),
            Err(error) => {
                let _ = fs::remove_dir_all(directory);
                Err(error)
            }
        }
    }
}

impl Drop for PrivateGraphicalDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct ReadinessChannel<'a> {
    listener: &'a UnixListener,
    stream: Option<UnixStream>,
    buffered: Vec<u8>,
    expected_uid: u32,
}

impl<'a> ReadinessChannel<'a> {
    fn new(listener: &'a UnixListener, expected_uid: u32) -> Result<Self, RuntimeError> {
        Ok(Self {
            listener,
            stream: None,
            buffered: Vec::new(),
            expected_uid,
        })
    }

    fn expect(
        &mut self,
        expected_stage: ProxyReadinessStage,
        spec: &GraphicalSupervisorSpec,
        deadline: Instant,
        proxy: &mut Child,
        mut app: Option<&mut Child>,
    ) -> Result<(), RuntimeError> {
        loop {
            if let Some(app) = app.as_deref_mut()
                && app
                    .try_wait()
                    .map_err(|_| RuntimeError::Internal)?
                    .is_some()
            {
                return Err(RuntimeError::FirstClientTimeout);
            }
            if proxy
                .try_wait()
                .map_err(|_| RuntimeError::Internal)?
                .is_some()
            {
                return Err(stage_failure(expected_stage));
            }
            if Instant::now() >= deadline {
                return Err(stage_failure(expected_stage));
            }
            if self.stream.is_none() {
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        let peer = getsockopt(&stream, PeerCredentials)
                            .map_err(|_| RuntimeError::ProxyUnavailable)?;
                        if peer.uid() != self.expected_uid {
                            return Err(RuntimeError::ProxyUnavailable);
                        }
                        let flags = rustix::io::fcntl_getfd(&stream)
                            .map_err(|_| RuntimeError::ProxyUnavailable)?;
                        rustix::io::fcntl_setfd(&stream, flags | rustix::io::FdFlags::CLOEXEC)
                            .map_err(|_| RuntimeError::ProxyUnavailable)?;
                        stream
                            .set_nonblocking(true)
                            .map_err(|_| RuntimeError::ProxyUnavailable)?;
                        self.stream = Some(stream);
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                        ) => {}
                    Err(_) => return Err(RuntimeError::ProxyUnavailable),
                }
            }
            if let Some(event) = self.read_event()? {
                validate_readiness_event(&event, expected_stage, spec)?;
                return Ok(());
            }
            std::thread::sleep(
                READINESS_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    fn read_event(&mut self) -> Result<Option<ProxyReadinessEvent>, RuntimeError> {
        if let Some(event) = decode_buffered_event(&mut self.buffered)? {
            return Ok(Some(event));
        }
        let Some(stream) = self.stream.as_mut() else {
            return Ok(None);
        };
        let mut chunk = [0u8; 1024];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => return Err(RuntimeError::ProxyUnavailable),
                Ok(read) => {
                    self.buffered.extend_from_slice(&chunk[..read]);
                    if self.buffered.len() > MAX_READINESS_EVENT_BYTES {
                        return Err(RuntimeError::ProxyUnavailable);
                    }
                    if let Some(event) = decode_buffered_event(&mut self.buffered)? {
                        return Ok(Some(event));
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Ok(None);
                }
                Err(_) => return Err(RuntimeError::ProxyUnavailable),
            }
        }
    }
}

fn decode_buffered_event(
    buffered: &mut Vec<u8>,
) -> Result<Option<ProxyReadinessEvent>, RuntimeError> {
    let Some(newline) = buffered.iter().position(|byte| *byte == b'\n') else {
        return Ok(None);
    };
    let frame = buffered.drain(..=newline).collect::<Vec<_>>();
    let body = &frame[..frame.len() - 1];
    if body.is_empty() {
        return Err(RuntimeError::ProxyUnavailable);
    }
    serde_json::from_slice(body)
        .map(Some)
        .map_err(|_| RuntimeError::ProxyUnavailable)
}

fn validate_readiness_event(
    event: &ProxyReadinessEvent,
    expected_stage: ProxyReadinessStage,
    spec: &GraphicalSupervisorSpec,
) -> Result<(), RuntimeError> {
    if event.protocol_version != READINESS_PROTOCOL_VERSION
        || event.target != spec.target
        || event.provider_kind != WorkloadProviderKind::UnsafeLocal
        || event.stage != expected_stage
        || event.state != ProxyReadinessState::Ready
        || event.failure.is_some()
    {
        return Err(stage_failure(expected_stage));
    }
    Ok(())
}

fn stage_failure(stage: ProxyReadinessStage) -> RuntimeError {
    match stage {
        ProxyReadinessStage::Upstream => RuntimeError::WaylandUnavailable,
        ProxyReadinessStage::Listener => RuntimeError::ProxyUnavailable,
        ProxyReadinessStage::FirstClient => RuntimeError::FirstClientTimeout,
    }
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
    use d2b_core::workload_identity::WorkloadTarget;
    use d2b_realm_core::token::ProtocolToken;
    use nix::unistd::Uid;
    use std::sync::{Arc, Barrier};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let mut random = [0u8; 8];
            getrandom::getrandom(&mut random).unwrap();
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .unwrap();
            let path = root.join(format!(".d2bt-{}", hex(&random)));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn graphical_spec(runtime_directory: PathBuf) -> GraphicalSupervisorSpec {
        GraphicalSupervisorSpec::new(
            PathBuf::from("/nix/store/fake-proxy/bin/d2b-wayland-proxy"),
            runtime_directory,
            "wayland-1".to_owned(),
            WorkloadTarget::parse("tools.host.d2b").unwrap(),
            RealmAccentColor::new("#cc3344").unwrap(),
            Uid::current().as_raw(),
        )
        .unwrap()
    }

    fn ready(spec: &GraphicalSupervisorSpec, stage: ProxyReadinessStage) -> ProxyReadinessEvent {
        ProxyReadinessEvent {
            protocol_version: READINESS_PROTOCOL_VERSION,
            target: spec.target.clone(),
            provider_kind: WorkloadProviderKind::UnsafeLocal,
            stage,
            state: ProxyReadinessState::Ready,
            failure: None,
        }
    }

    fn read_test_event(channel: &mut ReadinessChannel<'_>) -> ProxyReadinessEvent {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(event) = channel.read_event().unwrap() {
                return event;
            }
            assert!(Instant::now() < deadline, "readiness event timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

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
            realm_accent_color: d2b_contracts::unsafe_local_wire::RealmAccentColor::new("#336699")
                .unwrap(),
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
            graphical: None,
        };
        assert!(!format!("{spec:?}").contains(canary));
        assert!(!format!("{:?}", launch("op-debug", canary)).contains(canary));
    }

    #[test]
    fn graphical_spec_paths_and_proxy_arguments_are_strict_and_argv_free() {
        let scratch = Scratch::new();
        let mut spec = graphical_spec(scratch.0.clone());
        let app_canary = "private-app-argv-canary";
        let args = proxy_arguments(&spec).unwrap();
        let rendered = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\u{1f}");
        assert!(!rendered.contains(app_canary));
        for required in [
            "--connect",
            "--target",
            "--provider-kind",
            "unsafe-local",
            "--border-enable",
            "--border-color-active",
            "#cc3344",
            "--readiness-socket",
            "--first-client-timeout-ms",
            "--clipd-bridge-user-uid",
        ] {
            assert!(rendered.contains(required), "{required}");
        }
        assert!(spec.display.ends_with("/wayland.sock"));
        assert_eq!(
            spec.private_directory().unwrap().parent(),
            Some(scratch.0.as_path())
        );

        for invalid in [
            "/absolute/wayland.sock",
            "../wayland.sock",
            "d2b-unsafe-local-00112233445566778899aabbccddeeff/../wayland.sock",
            "d2b-unsafe-local-short/wayland.sock",
        ] {
            spec.display = invalid.to_owned();
            assert_eq!(spec.validate(), Err(RuntimeError::ProxyUnavailable));
        }
        assert!(SUPERVISOR_START_TIMEOUT > FIRST_CLIENT_TIMEOUT);
    }

    #[test]
    fn readiness_validation_rejects_order_identity_protocol_and_failure_drift() {
        let scratch = Scratch::new();
        let spec = graphical_spec(scratch.0.clone());
        let listener_event = ready(&spec, ProxyReadinessStage::Listener);
        assert_eq!(
            validate_readiness_event(&listener_event, ProxyReadinessStage::Upstream, &spec),
            Err(RuntimeError::WaylandUnavailable)
        );

        let mut mismatch = ready(&spec, ProxyReadinessStage::Upstream);
        mismatch.target = WorkloadTarget::parse("other.host.d2b").unwrap();
        assert_eq!(
            validate_readiness_event(&mismatch, ProxyReadinessStage::Upstream, &spec),
            Err(RuntimeError::WaylandUnavailable)
        );
        mismatch = ready(&spec, ProxyReadinessStage::Upstream);
        mismatch.protocol_version += 1;
        assert!(validate_readiness_event(&mismatch, ProxyReadinessStage::Upstream, &spec).is_err());
        mismatch = ready(&spec, ProxyReadinessStage::FirstClient);
        mismatch.state = ProxyReadinessState::Failed;
        assert_eq!(
            validate_readiness_event(&mismatch, ProxyReadinessStage::FirstClient, &spec),
            Err(RuntimeError::FirstClientTimeout)
        );
    }

    #[test]
    fn readiness_parser_is_bounded_and_rejects_malformed_frames() {
        let scratch = Scratch::new();
        let socket = scratch.0.join("unused.sock");
        let listener = UnixListener::bind(socket).unwrap();
        let (mut writer, reader) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let mut channel = ReadinessChannel {
            listener: &listener,
            stream: Some(reader),
            buffered: Vec::new(),
            expected_uid: Uid::current().as_raw(),
        };
        writer.write_all(b"{not-json}\n").unwrap();
        assert_eq!(channel.read_event(), Err(RuntimeError::ProxyUnavailable));

        let (mut writer, reader) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let mut channel = ReadinessChannel {
            listener: &listener,
            stream: Some(reader),
            buffered: Vec::new(),
            expected_uid: Uid::current().as_raw(),
        };
        writer
            .write_all(&vec![b'x'; MAX_READINESS_EVENT_BYTES + 1])
            .unwrap();
        assert_eq!(channel.read_event(), Err(RuntimeError::ProxyUnavailable));
    }

    #[test]
    fn fake_proxy_and_app_complete_typed_readiness_and_cleanup() {
        let scratch = Scratch::new();
        let spec = graphical_spec(scratch.0.clone());
        let private_directory = spec.private_directory().unwrap();
        let runtime = PrivateGraphicalDirectory::prepare(&spec).unwrap();
        let metadata = fs::symlink_metadata(&private_directory).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o700);
        let (mut readiness_writer, readiness_reader) = UnixStream::pair().unwrap();
        readiness_reader.set_nonblocking(true).unwrap();
        assert!(
            rustix::io::fcntl_getfd(&readiness_reader)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        let unused_listener = UnixListener::bind(scratch.0.join("unused.sock")).unwrap();
        let wayland_path = PathBuf::from(&spec.display);
        let generated_private_directory = wayland_path.parent().unwrap().to_path_buf();
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(&generated_private_directory).unwrap();

        let proxy_spec = spec.clone();
        let proxy_wayland_path = wayland_path.clone();
        let proxy = std::thread::spawn(move || {
            for stage in [ProxyReadinessStage::Upstream, ProxyReadinessStage::Listener] {
                serde_json::to_writer(&mut readiness_writer, &ready(&proxy_spec, stage)).unwrap();
                readiness_writer.write_all(b"\n").unwrap();
            }
            let listener = UnixListener::bind(proxy_wayland_path).unwrap();
            let _client = listener.accept().unwrap().0;
            serde_json::to_writer(
                &mut readiness_writer,
                &ready(&proxy_spec, ProxyReadinessStage::FirstClient),
            )
            .unwrap();
            readiness_writer.write_all(b"\n").unwrap();
        });
        let mut channel = ReadinessChannel {
            listener: &unused_listener,
            stream: Some(readiness_reader),
            buffered: Vec::new(),
            expected_uid: Uid::current().as_raw(),
        };
        let event = read_test_event(&mut channel);
        validate_readiness_event(&event, ProxyReadinessStage::Upstream, &spec).unwrap();
        let event = read_test_event(&mut channel);
        validate_readiness_event(&event, ProxyReadinessStage::Listener, &spec).unwrap();

        let app_path = PathBuf::from(&spec.display);
        let app = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match UnixStream::connect(&app_path) {
                    Ok(stream) => return stream,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            }
        });
        let event = read_test_event(&mut channel);
        validate_readiness_event(&event, ProxyReadinessStage::FirstClient, &spec).unwrap();
        drop(app.join().unwrap());
        proxy.join().unwrap();
        drop(channel);
        drop(runtime);
        assert!(!private_directory.exists());
        fs::remove_file(&wayland_path).unwrap();
        fs::remove_dir(&generated_private_directory).unwrap();
    }

    #[test]
    fn plain_supervisor_behavior_is_unchanged() {
        let spec = SupervisorSpec {
            program: std::env::current_exe().unwrap(),
            args: vec!["--list".to_owned()],
            environment: BTreeMap::new(),
            cwd: std::env::current_dir().unwrap(),
            graphical: None,
        };
        let mut ack = Vec::new();
        run_plain_supervisor(&spec, &mut ack).unwrap();
        assert_eq!(ack, [1]);
    }

    #[test]
    fn first_client_wait_fails_immediately_when_app_exits() {
        let scratch = Scratch::new();
        let spec = graphical_spec(scratch.0.clone());
        let listener = UnixListener::bind(scratch.0.join("readiness.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut channel = ReadinessChannel::new(&listener, Uid::current().as_raw()).unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut proxy = Command::new(&executable)
            .args(["--exact", "runtime::tests::test_child_hold", "--nocapture"])
            .env("D2B_TEST_HOLD", "1")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let mut app = Command::new(executable)
            .arg("--list")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        assert_eq!(
            channel.expect(
                ProxyReadinessStage::FirstClient,
                &spec,
                Instant::now() + FIRST_CLIENT_TIMEOUT,
                &mut proxy,
                Some(&mut app),
            ),
            Err(RuntimeError::FirstClientTimeout)
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        terminate_and_reap(&mut proxy);
        terminate_and_reap(&mut app);
    }

    #[test]
    fn readiness_wait_uses_an_absolute_deadline() {
        let scratch = Scratch::new();
        let spec = graphical_spec(scratch.0.clone());
        let listener = UnixListener::bind(scratch.0.join("readiness.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut channel = ReadinessChannel::new(&listener, Uid::current().as_raw()).unwrap();
        let mut proxy = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "runtime::tests::test_child_hold", "--nocapture"])
            .env("D2B_TEST_HOLD", "1")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        assert_eq!(
            channel.expect(
                ProxyReadinessStage::Upstream,
                &spec,
                Instant::now() + Duration::from_millis(30),
                &mut proxy,
                None,
            ),
            Err(RuntimeError::WaylandUnavailable)
        );
        assert!(started.elapsed() < Duration::from_millis(250));
        terminate_and_reap(&mut proxy);
    }

    #[test]
    fn test_child_hold() {
        if std::env::var_os("D2B_TEST_HOLD").is_some() {
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    #[test]
    fn immutable_proxy_path_rejects_mutable_and_non_executable_paths() {
        assert_eq!(
            validate_immutable_proxy_binary(Path::new("/usr/bin/d2b-wayland-proxy")),
            Err(RuntimeError::ProxyUnavailable)
        );
        assert_eq!(
            validate_immutable_proxy_binary(Path::new("relative/proxy")),
            Err(RuntimeError::ProxyUnavailable)
        );
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
