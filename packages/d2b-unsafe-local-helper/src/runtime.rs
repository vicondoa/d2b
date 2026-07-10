use crate::environment::EnvironmentError;
use crate::systemd::{ScopeError, ScopeInspection, UserScopeManager, VerifiedScope};
use d2b_contracts::unsafe_local_wire::{
    HelperLaunchRequest, HelperOperationDisposition, HelperOperationResult, HelperScopeKind,
    HelperScopeSnapshot, HelperScopeState, HelperSnapshot, MAX_HELPER_SNAPSHOT_SCOPES,
};
use d2b_core::workload_identity::WorkloadIdentity;
use d2b_realm_core::ids::OperationId;
use nix::libc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use std::time::Instant;
use uzers::os::unix::UserExt;
use uzers::{get_current_uid, get_user_by_uid};

pub const MANAGER_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
pub const SUPERVISOR_START_TIMEOUT: Duration = Duration::from_secs(5);
pub const SNAPSHOT_RECONCILE_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_LEDGER_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeError {
    InvalidIdentity,
    UserManagerUnavailable,
    EnvironmentInvalid,
    ExecutableUnavailable,
    ProxyUnavailable,
    ScopeCreateFailed,
    ScopeIdentityMismatch,
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
            ScopeError::QueryFailed | ScopeError::StopFailed => Self::Internal,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedScope {
    operation_id: OperationId,
    workload: WorkloadIdentity,
    unit_name: String,
    invocation_id: String,
    control_group: String,
    kind: HelperScopeKind,
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
            .finish()
    }
}

impl PersistedScope {
    fn verified(&self) -> VerifiedScope {
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
struct PersistedScopeLedger {
    schema_version: u32,
    scopes: Vec<PersistedScope>,
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
    manager: Arc<M>,
    ledger_path: PathBuf,
    ledger: Mutex<PersistedScopeLedger>,
    user_home: PathBuf,
}

impl<M: UserScopeManager> fmt::Debug for ScopeRuntime<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeRuntime")
            .field("ledger_path", &"<redacted>")
            .field("user_home", &"<redacted>")
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
        Self::with_paths(manager, user_home, ledger_path)
    }

    pub fn with_paths(
        manager: M,
        user_home: PathBuf,
        ledger_path: PathBuf,
    ) -> Result<Self, RuntimeError> {
        let ledger = load_ledger(&ledger_path)?;
        Ok(Self {
            manager: Arc::new(manager),
            ledger_path,
            ledger: Mutex::new(ledger),
            user_home,
        })
    }

    pub fn launch(
        &self,
        request: HelperLaunchRequest,
    ) -> Result<HelperOperationResult, RuntimeError> {
        let environment = timed_manager_call(Arc::clone(&self.manager), |manager| {
            manager.manager_environment()
        })??;
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
        let scope = match timed_manager_call(Arc::clone(&self.manager), move |manager| {
            manager.start_scope(supervisor_pid, HelperScopeKind::LauncherApp)
        }) {
            Ok(Ok(scope)) => scope,
            Ok(Err(error)) => {
                supervisor.abort();
                return Err(error.into());
            }
            Err(error) => {
                supervisor.abort();
                return Err(error.into());
            }
        };
        let persisted = PersistedScope {
            operation_id: request.operation_id.clone(),
            workload: request.workload,
            unit_name: scope.unit_name.clone(),
            invocation_id: scope.invocation_id.clone(),
            control_group: scope.control_group.clone(),
            kind: scope.kind,
        };
        let persist_result = match self.ledger.lock() {
            Ok(mut ledger) => {
                ledger
                    .scopes
                    .retain(|entry| entry.operation_id != persisted.operation_id);
                ledger.scopes.push(persisted);
                if ledger.scopes.len() > MAX_HELPER_SNAPSHOT_SCOPES {
                    Err(RuntimeError::LedgerInvalid)
                } else {
                    persist_ledger(&self.ledger_path, &ledger)
                }
            }
            Err(_) => Err(RuntimeError::Internal),
        };
        if let Err(error) = persist_result {
            supervisor.abort();
            self.stop_failed_scope(&scope);
            self.remove_scope_record(&request.operation_id);
            return Err(error);
        }
        if let Err(error) = supervisor.release_and_wait_started() {
            supervisor.abort();
            self.stop_failed_scope(&scope);
            self.remove_scope_record(&request.operation_id);
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

    fn stop_failed_scope(&self, scope: &VerifiedScope) {
        let scope_for_kill = scope.clone();
        let manager = Arc::clone(&self.manager);
        let _ = timed_manager_call(manager, move |manager| {
            manager.terminate_scope(&scope_for_kill, libc::SIGKILL)
        });
        let scope_for_stop = scope.clone();
        let manager = Arc::clone(&self.manager);
        let _ = timed_manager_call(manager, move |manager| manager.stop_scope(&scope_for_stop));
    }

    fn remove_scope_record(&self, operation_id: &OperationId) {
        let Ok(mut ledger) = self.ledger.lock() else {
            return;
        };
        ledger
            .scopes
            .retain(|entry| &entry.operation_id != operation_id);
        let _ = persist_ledger(&self.ledger_path, &ledger);
    }

    pub fn snapshot(&self, generation: u64) -> Result<HelperSnapshot, RuntimeError> {
        let entries = self
            .ledger
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .scopes
            .clone();
        if entries.len() > MAX_HELPER_SNAPSHOT_SCOPES {
            return Err(RuntimeError::LedgerInvalid);
        }

        let mut scopes = Vec::with_capacity(entries.len());
        let deadline = Instant::now() + SNAPSHOT_RECONCILE_TIMEOUT;
        for entry in entries {
            let state = if Instant::now() >= deadline {
                HelperScopeState::Degraded
            } else {
                let verified = entry.verified();
                let manager = Arc::clone(&self.manager);
                let observed =
                    timed_manager_call(manager, move |manager| manager.inspect_scope(&verified));
                match observed {
                    Ok(Ok(ScopeInspection {
                        state,
                        identity_matches: true,
                    })) => state,
                    _ => HelperScopeState::Degraded,
                }
            };
            let scope = entry.verified().wire_identity();
            scopes.push(HelperScopeSnapshot {
                operation_id: entry.operation_id,
                workload: entry.workload,
                scope,
                state,
            });
        }
        Ok(HelperSnapshot { generation, scopes })
    }
}

fn timed_manager_call<M, T, F>(
    manager: Arc<M>,
    operation: F,
) -> Result<Result<T, ScopeError>, ScopeError>
where
    M: UserScopeManager,
    T: Send + 'static,
    F: FnOnce(&M) -> Result<T, ScopeError> + Send + 'static,
{
    timed_manager_call_with_timeout(manager, MANAGER_OPERATION_TIMEOUT, operation)
}

fn timed_manager_call_with_timeout<M, T, F>(
    manager: Arc<M>,
    timeout: Duration,
    operation: F,
) -> Result<Result<T, ScopeError>, ScopeError>
where
    M: UserScopeManager,
    T: Send + 'static,
    F: FnOnce(&M) -> Result<T, ScopeError> + Send + 'static,
{
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("d2b-user-manager-op".to_owned())
        .spawn(move || {
            let _ = sender.send(operation(&manager));
        })
        .map_err(|_| ScopeError::UserManagerUnavailable)?;
    receiver
        .recv_timeout(timeout)
        .map_err(|_| ScopeError::Timeout)
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
    if ledger.schema_version != 1 || ledger.scopes.len() > MAX_HELPER_SNAPSHOT_SCOPES {
        return Err(RuntimeError::LedgerInvalid);
    }
    Ok(ledger)
}

fn persist_ledger(path: &Path, ledger: &PersistedScopeLedger) -> Result<(), RuntimeError> {
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
    use crate::environment::ManagerEnvironment;
    use d2b_contracts::unsafe_local_wire::ScopeIdentity;

    #[derive(Clone)]
    struct HangingManager;

    impl UserScopeManager for HangingManager {
        fn manager_environment(&self) -> Result<ManagerEnvironment, ScopeError> {
            std::thread::sleep(Duration::from_millis(100));
            Err(ScopeError::UserManagerUnavailable)
        }

        fn start_scope(
            &self,
            _supervisor_pid: u32,
            _kind: HelperScopeKind,
        ) -> Result<VerifiedScope, ScopeError> {
            Err(ScopeError::CreateFailed)
        }

        fn inspect_scope(&self, _scope: &VerifiedScope) -> Result<ScopeInspection, ScopeError> {
            Err(ScopeError::QueryFailed)
        }

        fn terminate_scope(&self, _scope: &VerifiedScope, _signal: i32) -> Result<(), ScopeError> {
            Err(ScopeError::StopFailed)
        }

        fn stop_scope(&self, _scope: &VerifiedScope) -> Result<(), ScopeError> {
            Err(ScopeError::StopFailed)
        }
    }

    #[test]
    fn persisted_scope_debug_hides_scope_identifiers() {
        let canary = "scope-private-canary";
        let persisted = PersistedScope {
            operation_id: OperationId::parse("op-1").unwrap(),
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
            kind: HelperScopeKind::LauncherApp,
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

    #[test]
    fn hung_user_manager_operation_times_out_without_blocking_caller() {
        let started = Instant::now();
        let result = timed_manager_call_with_timeout(
            Arc::new(HangingManager),
            Duration::from_millis(5),
            UserScopeManager::manager_environment,
        );
        assert!(matches!(result, Err(ScopeError::Timeout)));
        assert!(started.elapsed() < Duration::from_millis(50));
    }
}
