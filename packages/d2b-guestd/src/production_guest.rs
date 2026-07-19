//! Production owners behind the typed guest service.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::OwnedFd,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::guest_proto as legacy_guest;
use d2b_contracts::v2_services::{
    MAX_TERMINAL_CHUNK_BYTES, TerminalFrameDirection, TerminalStreamValidator,
    common::{self, ErrorKind, Outcome, RetryClass},
    guest,
    guest_contract::{
        FileTransferStreamValidator, GuestStreamDirection, MAX_GUEST_FILE_CHUNK_BYTES,
        SecurityKeyStreamValidator,
    },
    terminal,
};
use d2b_session::Cancellation;
use protobuf::{Enum, EnumOrUnknown, Message, MessageField};
use rustix::fs::{AtFlags, Mode, OFlags, RenameFlags, openat, renameat_with, unlinkat};
use sha2::{Digest, Sha256};

use crate::{
    activation::{ActivationRuntime, ActivationRuntimeConfig},
    detached::{RunnerUnitPaths, SystemdRunUnitManager},
    detached_registry::{
        DetachedRegistry, RegistryConfig, RunSlotStore, SystemWallClock, TokioSleeper,
    },
    exec::{
        ConnectionKey, ExecCreateInput, ExecError, ExecIdSource, ExecRuntime, ExecSnapshot,
        ExecState, ExitOutcome, RingChunk, Stream as ExecStream, TtyStdinSnapshot,
        validate_and_authorize_detached,
    },
    exec_linux::{LinuxProcessSpawner, WorkloadUserSpawn},
    exec_pty::linux::LinuxPtyProcessSpawner,
    guest_service::{GuestOperationHandler, GuestStream, GuestStreamBinding, GuestStreamInput},
    login_session::{WorkloadUserUid, classify_workload_user},
    service_v2::GuestSessionError,
    shell::{ShellRuntime, ShellRuntimeConfig},
};

type AttachedRuntime = ExecRuntime<LinuxProcessSpawner, RandomExecIds>;

#[derive(Clone)]
pub struct ProductionExecConfig {
    pub exec_user: String,
    pub systemd_run_path: PathBuf,
    pub exec_runner_path: PathBuf,
    pub login_shell_path: PathBuf,
    pub detached_max_runtime_sec: u64,
    pub interactive_max_runtime_sec: u64,
}

#[derive(Clone)]
pub struct ProductionShellConfig {
    pub default_name: String,
    pub max_sessions: u32,
    pub max_attached: u32,
    pub runner_path: PathBuf,
    pub systemctl_path: PathBuf,
    pub socket_path: PathBuf,
}

#[derive(Clone)]
pub struct ProductionGuestConfig {
    pub workload_id: String,
    pub exec: Option<ProductionExecConfig>,
    pub shell: Option<ProductionShellConfig>,
    pub guest_config_path: Option<PathBuf>,
    pub shutdown_systemctl_path: Option<PathBuf>,
    pub activation: Option<ActivationRuntimeConfig>,
    pub configured_launches: BTreeMap<String, Vec<String>>,
    pub configured_launch_realm_id: Option<String>,
    pub configured_launch_workload_digest: Option<[u8; 32]>,
    pub security_key: Option<Arc<dyn SecurityKeyBackend>>,
}

impl ProductionGuestConfig {
    pub fn disabled(workload_id: String) -> Self {
        Self {
            workload_id,
            exec: None,
            shell: None,
            guest_config_path: None,
            shutdown_systemctl_path: None,
            activation: None,
            configured_launches: BTreeMap::new(),
            configured_launch_realm_id: None,
            configured_launch_workload_digest: None,
            security_key: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyExchangeOutcome {
    Continue,
    Succeeded,
    CtapError,
}

pub struct SecurityKeyExchange {
    pub guest_report: [u8; 64],
    pub outcome: SecurityKeyExchangeOutcome,
}

#[async_trait]
pub trait SecurityKeyCeremony: Send {
    async fn exchange(
        &mut self,
        device_report: [u8; 64],
    ) -> Result<SecurityKeyExchange, GuestSessionError>;
    async fn cancel(&mut self);
}

#[async_trait]
pub trait SecurityKeyBackend: Send + Sync {
    fn ready(&self) -> bool;
    async fn begin(
        &self,
        request: &guest::GuestSecurityKeyRequest,
    ) -> Result<Box<dyn SecurityKeyCeremony>, GuestSessionError>;
}

#[async_trait]
pub trait ShutdownBackend: Send + Sync {
    fn ready(&self) -> bool;
    async fn request(
        &self,
        action: guest::GuestPowerAction,
        timeout: Duration,
    ) -> Result<(), GuestSessionError>;
}

struct SystemdShutdownBackend {
    systemctl_path: PathBuf,
}

#[async_trait]
impl ShutdownBackend for SystemdShutdownBackend {
    fn ready(&self) -> bool {
        executable_ready(&self.systemctl_path)
    }

    async fn request(
        &self,
        action: guest::GuestPowerAction,
        timeout: Duration,
    ) -> Result<(), GuestSessionError> {
        let verb = match action {
            guest::GuestPowerAction::GUEST_POWER_ACTION_POWER_OFF => "poweroff",
            guest::GuestPowerAction::GUEST_POWER_ACTION_REBOOT => "reboot",
            guest::GuestPowerAction::GUEST_POWER_ACTION_HALT => "halt",
            guest::GuestPowerAction::GUEST_POWER_ACTION_UNSPECIFIED => {
                return Err(GuestSessionError::Service);
            }
        };
        let status = tokio::time::timeout(
            timeout,
            tokio::process::Command::new(&self.systemctl_path)
                .args(["--no-block", "--no-ask-password", verb])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status(),
        )
        .await
        .map_err(|_| GuestSessionError::Service)?
        .map_err(|_| GuestSessionError::Service)?;
        if status.success() {
            Ok(())
        } else {
            Err(GuestSessionError::Service)
        }
    }
}

struct RandomExecIds;

impl ExecIdSource for RandomExecIds {
    fn next_exec_id(&self) -> Result<String, ExecError> {
        let mut bytes = [0_u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(&mut bytes))
            .map_err(|_| ExecError::Internal)?;
        if bytes == [0; 16] {
            return Err(ExecError::Internal);
        }
        Ok(hex(&bytes))
    }
}

#[derive(Clone)]
enum ExecOwnerKind {
    Attached {
        runtime_id: String,
        owner_key: ConnectionKey,
    },
    Detached,
}

#[derive(Clone)]
struct ExecRecord {
    kind: ExecOwnerKind,
    created_at_unix_ms: u64,
    argv_digest: [u8; 32],
    last_cancel_sequence: u64,
}

impl std::fmt::Debug for ExecRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExecRecord")
            .field("kind", &"<redacted>")
            .field("created_at_unix_ms", &self.created_at_unix_ms)
            .field("argv_digest", &"<redacted>")
            .field("last_cancel_sequence", &self.last_cancel_sequence)
            .finish()
    }
}

#[derive(Clone)]
struct ArtifactEndpoint {
    path: PathBuf,
    direction: guest::GuestFileTransferDirection,
}

struct AtomicArtifactWriter {
    directory: File,
    file: File,
    staging_name: OsString,
    target_name: OsString,
    committed: bool,
}

impl fmt::Debug for AtomicArtifactWriter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AtomicArtifactWriter")
            .field("paths", &"<redacted>")
            .field("committed", &self.committed)
            .finish()
    }
}

impl AtomicArtifactWriter {
    fn prepare(path: &Path, resource_handle: &str) -> Result<Self, GuestSessionError> {
        if !artifact_ready(path) {
            return Err(GuestSessionError::Service);
        }
        let parent = path.parent().ok_or(GuestSessionError::Service)?;
        let target_name = path
            .file_name()
            .map(OsString::from)
            .ok_or(GuestSessionError::Service)?;
        let directory = File::from(
            rustix::fs::open(
                parent,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(|_| GuestSessionError::Service)?,
        );
        let target = File::from(
            openat(
                &directory,
                &target_name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(|_| GuestSessionError::Service)?,
        );
        let target_metadata = target.metadata().map_err(|_| GuestSessionError::Service)?;
        if !target_metadata.is_file()
            || target_metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(GuestSessionError::Service);
        }
        let staging_name = OsString::from(format!(".d2b-transfer-{resource_handle}.staging"));
        let file = File::from(
            openat(
                &directory,
                &staging_name,
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::from_bits_truncate(target_metadata.mode() & 0o777),
            )
            .map_err(|_| GuestSessionError::Service)?,
        );
        Ok(Self {
            directory,
            file,
            staging_name,
            target_name,
            committed: false,
        })
    }

    fn copy_prefix(&mut self, offset: u64) -> Result<Sha256, GuestSessionError> {
        let mut source = File::from(
            openat(
                &self.directory,
                &self.target_name,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .map_err(|_| GuestSessionError::Service)?,
        );
        if source
            .metadata()
            .map_err(|_| GuestSessionError::Service)?
            .len()
            < offset
        {
            return Err(GuestSessionError::Service);
        }
        let mut digest = Sha256::new();
        let mut remaining = offset;
        let mut buffer = [0_u8; 64 * 1024];
        while remaining != 0 {
            let take = remaining.min(buffer.len() as u64) as usize;
            source
                .read_exact(&mut buffer[..take])
                .map_err(|_| GuestSessionError::Service)?;
            self.file
                .write_all(&buffer[..take])
                .map_err(|_| GuestSessionError::Service)?;
            digest.update(&buffer[..take]);
            remaining -= take as u64;
        }
        Ok(digest)
    }

    fn commit(&mut self, total_size: u64) -> Result<(), GuestSessionError> {
        self.file
            .set_len(total_size)
            .and_then(|()| self.file.sync_all())
            .map_err(|_| GuestSessionError::Service)?;
        renameat_with(
            &self.directory,
            &self.staging_name,
            &self.directory,
            &self.target_name,
            RenameFlags::EXCHANGE,
        )
        .map_err(|_| GuestSessionError::Service)?;
        if self.directory.sync_all().is_err() {
            let _ = renameat_with(
                &self.directory,
                &self.staging_name,
                &self.directory,
                &self.target_name,
                RenameFlags::EXCHANGE,
            );
            let _ = self.directory.sync_all();
            return Err(GuestSessionError::Service);
        }
        if unlinkat(&self.directory, &self.staging_name, AtFlags::empty()).is_err() {
            let _ = renameat_with(
                &self.directory,
                &self.staging_name,
                &self.directory,
                &self.target_name,
                RenameFlags::EXCHANGE,
            );
            let _ = self.directory.sync_all();
            return Err(GuestSessionError::Service);
        }
        let _ = self.directory.sync_all();
        self.committed = true;
        Ok(())
    }
}

impl Drop for AtomicArtifactWriter {
    fn drop(&mut self) {
        if !self.committed {
            let _ = unlinkat(&self.directory, &self.staging_name, AtFlags::empty());
        }
    }
}

#[derive(Clone)]
struct ShellOwnerRecord {
    name: String,
    owner_key: ConnectionKey,
    runtime_id: String,
}

impl std::fmt::Debug for ShellOwnerRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ShellOwnerRecord(<redacted>)")
    }
}

impl std::fmt::Debug for ArtifactEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArtifactEndpoint")
            .field("path", &"<redacted>")
            .field("direction", &self.direction)
            .finish()
    }
}

#[derive(Clone)]
struct ProductionTaskContext {
    attached: Option<Arc<AttachedRuntime>>,
    detached: Option<Arc<DetachedRegistry>>,
    boot_id: String,
    records: Arc<Mutex<BTreeMap<String, ExecRecord>>>,
    adopted_cancel_sequences: Arc<Mutex<BTreeMap<String, u64>>>,
    configured_launches: Arc<BTreeMap<String, Vec<String>>>,
    configured_launch_realm_id: Option<String>,
    configured_launch_workload_digest: Option<[u8; 32]>,
    shell: Option<Arc<ShellRuntime>>,
    shell_handles: Arc<Mutex<BTreeMap<String, ShellOwnerRecord>>>,
    artifacts: Arc<BTreeMap<(i32, String), ArtifactEndpoint>>,
    security_key: Option<Arc<dyn SecurityKeyBackend>>,
}

pub struct ProductionGuestOperations {
    workload_id: String,
    capabilities: Vec<guest::GuestCapability>,
    tasks: ProductionTaskContext,
    shutdown: Option<Arc<dyn ShutdownBackend>>,
    activation: Arc<ActivationRuntime>,
    shutdown_inflight: Mutex<BTreeMap<String, Arc<ShutdownFlight>>>,
    shutdown_success: Mutex<BTreeMap<String, Vec<u8>>>,
}

struct ShutdownFlight {
    result: Mutex<Option<Result<(), ErrorKind>>>,
    notify: tokio::sync::Notify,
}

impl std::fmt::Debug for ProductionGuestOperations {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductionGuestOperations")
            .field("workload_id", &"<redacted>")
            .field("capability_count", &self.capabilities.len())
            .finish()
    }
}

impl ProductionGuestOperations {
    pub async fn new(config: ProductionGuestConfig) -> Result<Self, GuestSessionError> {
        let boot_id = read_boot_id()?;
        let activation =
            ActivationRuntime::production(config.workload_id.clone(), config.activation.clone());
        let mut capabilities = Vec::new();
        let (attached, detached) = if let Some(exec) = config.exec.as_ref() {
            match classify_workload_user(&exec.exec_user) {
                WorkloadUserUid::NonRoot(uid) => {
                    let exec_ready = [
                        &exec.systemd_run_path,
                        &exec.exec_runner_path,
                        &exec.login_shell_path,
                    ]
                    .into_iter()
                    .all(|path| executable_ready(path))
                        && pam_login_ready()
                        && directory_ready(Path::new(d2b_exec_runner::paths::RUN_DIR));
                    if !exec_ready {
                        (None, None)
                    } else {
                        let policy = crate::exec::ExecPolicy {
                            enabled: true,
                            exec_user: Some(exec.exec_user.clone()),
                        };
                        let attached = Arc::new(
                            ExecRuntime::new(
                                LinuxProcessSpawner::new(WorkloadUserSpawn {
                                    systemd_run_path: exec.systemd_run_path.clone(),
                                    login_shell_path: exec.login_shell_path.clone(),
                                    exec_user: exec.exec_user.clone(),
                                }),
                                RandomExecIds,
                                policy,
                            )
                            .with_pty_spawner(Arc::new(LinuxPtyProcessSpawner::new(
                                exec.exec_runner_path.clone(),
                                exec.systemd_run_path.clone(),
                                exec.login_shell_path.clone(),
                                exec.exec_user.clone(),
                            )))
                            .with_interactive_ceiling(
                                (exec.interactive_max_runtime_sec != 0)
                                    .then(|| Duration::from_secs(exec.interactive_max_runtime_sec)),
                            ),
                        );
                        let detached = Arc::new(DetachedRegistry::new(
                            Arc::new(SystemdRunUnitManager::new(exec.systemd_run_path.clone())),
                            Arc::new(RunSlotStore::new()),
                            Arc::new(SystemWallClock),
                            Arc::new(TokioSleeper),
                            Arc::new(RandomExecIds),
                            RegistryConfig {
                                paths: RunnerUnitPaths::new(exec.exec_runner_path.clone()),
                                boot_id: boot_id.clone(),
                                max_runtime_sec: exec.detached_max_runtime_sec,
                                exec_user: exec.exec_user.clone(),
                                exec_uid: uid,
                                systemd_run_path: exec
                                    .systemd_run_path
                                    .to_string_lossy()
                                    .into_owned(),
                                login_shell_path: exec
                                    .login_shell_path
                                    .to_string_lossy()
                                    .into_owned(),
                            },
                        ));
                        detached.reconcile_on_startup().await;
                        capabilities.extend([
                            guest::GuestCapability::GUEST_CAPABILITY_EXEC_ATTACHED,
                            guest::GuestCapability::GUEST_CAPABILITY_EXEC_DETACHED,
                            guest::GuestCapability::GUEST_CAPABILITY_EXEC_TTY,
                            guest::GuestCapability::GUEST_CAPABILITY_EXEC_RETAINED_LOGS,
                            guest::GuestCapability::GUEST_CAPABILITY_TTY_RESIZE,
                            guest::GuestCapability::GUEST_CAPABILITY_SIGNALS,
                        ]);
                        (Some(attached), Some(detached))
                    }
                }
                WorkloadUserUid::Root => {
                    return Err(GuestSessionError::InvalidConfiguration);
                }
                WorkloadUserUid::Unresolved => (None, None),
            }
        } else {
            (None, None)
        };

        let shell = match (
            config.shell.as_ref(),
            attached.as_ref(),
            config.exec.as_ref(),
        ) {
            (Some(shell), Some(_), Some(exec))
                if executable_ready(&shell.runner_path)
                    && executable_ready(&shell.systemctl_path)
                    && unix_socket_ready(
                        &shell.socket_path,
                        classify_nonroot_uid(&exec.exec_user),
                    ) =>
            {
                capabilities.extend([
                    guest::GuestCapability::GUEST_CAPABILITY_SHELL_ATTACHED,
                    guest::GuestCapability::GUEST_CAPABILITY_SHELL_MANAGEMENT,
                ]);
                Some(Arc::new(ShellRuntime::enabled(ShellRuntimeConfig {
                    default_name: shell.default_name.clone(),
                    max_sessions: shell.max_sessions,
                    max_attached: shell.max_attached,
                    workload_user: Some(exec.exec_user.clone()),
                    workload_uid: match classify_workload_user(&exec.exec_user) {
                        WorkloadUserUid::NonRoot(uid) => Some(uid),
                        WorkloadUserUid::Root | WorkloadUserUid::Unresolved => None,
                    },
                    guest_boot_id: boot_id.clone(),
                    guestd_instance_id: random_token()?,
                    daemon_instance_id: random_token()?,
                    runner_path: shell.runner_path.clone(),
                    systemctl_path: shell.systemctl_path.clone(),
                    socket_path: shell.socket_path.clone(),
                })))
            }
            (None, _, _) => None,
            _ => None,
        };

        let mut artifacts = BTreeMap::new();
        if let Some(path) = config.guest_config_path
            && artifact_ready(&path)
        {
            artifacts.insert(
                (
                guest::GuestArtifactId::GUEST_ARTIFACT_ID_GUEST_CONFIG.value(),
                "guest-config".to_owned(),
                ),
                ArtifactEndpoint {
                path,
                direction:
                    guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_HOST_TO_GUEST,
                },
            );
            capabilities.push(guest::GuestCapability::GUEST_CAPABILITY_FILE_TRANSFER);
        }
        if let Some((intent_id, path)) = activation.payload_endpoint() {
            artifacts.insert(
                (
                    guest::GuestArtifactId::GUEST_ARTIFACT_ID_ACTIVATION_PAYLOAD.value(),
                    intent_id,
                ),
                ArtifactEndpoint {
                    path,
                    direction:
                        guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_HOST_TO_GUEST,
                },
            );
            capabilities.push(guest::GuestCapability::GUEST_CAPABILITY_FILE_TRANSFER);
        }
        if config
            .security_key
            .as_ref()
            .is_some_and(|backend| backend.ready())
        {
            capabilities.push(guest::GuestCapability::GUEST_CAPABILITY_SECURITY_KEY);
        }
        let shutdown = config.shutdown_systemctl_path.map(|systemctl_path| {
            Arc::new(SystemdShutdownBackend { systemctl_path }) as Arc<dyn ShutdownBackend>
        });
        if shutdown.as_ref().is_some_and(|backend| backend.ready()) {
            capabilities.push(guest::GuestCapability::GUEST_CAPABILITY_SHUTDOWN);
        }
        capabilities.sort_by_key(|capability| capability.value());
        capabilities.dedup();
        Ok(Self {
            workload_id: config.workload_id,
            capabilities,
            tasks: ProductionTaskContext {
                attached,
                detached,
                boot_id,
                records: Arc::new(Mutex::new(BTreeMap::new())),
                adopted_cancel_sequences: Arc::new(Mutex::new(BTreeMap::new())),
                configured_launches: Arc::new(config.configured_launches),
                configured_launch_realm_id: config.configured_launch_realm_id,
                configured_launch_workload_digest: config.configured_launch_workload_digest,
                shell,
                shell_handles: Arc::new(Mutex::new(BTreeMap::new())),
                artifacts: Arc::new(artifacts),
                security_key: config.security_key,
            },
            shutdown,
            activation,
            shutdown_inflight: Mutex::new(BTreeMap::new()),
            shutdown_success: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn activation_runtime(&self) -> Arc<ActivationRuntime> {
        Arc::clone(&self.activation)
    }
}

#[async_trait]
impl GuestOperationHandler for ProductionGuestOperations {
    fn capabilities(&self) -> Vec<guest::GuestCapability> {
        self.capabilities.clone()
    }

    fn scope_authorized(&self, scope: &common::IdentityScope) -> bool {
        scope.workload_id == self.workload_id
            && !scope.realm_id.is_empty()
            && scope.provider_id.is_empty()
            && scope.role_id.is_empty()
    }

    fn stream_ready(&self, method: crate::guest_service::GuestStreamMethod) -> bool {
        use crate::guest_service::GuestStreamMethod;
        match method {
            GuestStreamMethod::Exec => self.tasks.attached.is_some(),
            GuestStreamMethod::RetainedLog => {
                self.tasks.attached.is_some() || self.tasks.detached.is_some()
            }
            GuestStreamMethod::Shell => self.tasks.shell.is_some(),
            GuestStreamMethod::FileTransfer => !self.tasks.artifacts.is_empty(),
            GuestStreamMethod::SecurityKey => self
                .tasks
                .security_key
                .as_ref()
                .is_some_and(|backend| backend.ready()),
        }
    }

    async fn serve_exec(
        &self,
        _: guest::GuestExecRequest,
        _: terminal::TerminalOpenResponse,
        binding: GuestStreamBinding,
        stream: GuestStream,
    ) -> Result<(), GuestSessionError> {
        if self.tasks.attached.is_none() {
            return Err(GuestSessionError::Service);
        }
        let tasks = self.tasks.clone();
        tokio::spawn(async move {
            let _ = drive_exec_stream(tasks, binding, stream).await;
        });
        Ok(())
    }

    async fn cancel_exec(
        &self,
        request: guest::GuestCancelExecRequest,
        binding: GuestStreamBinding,
    ) -> Result<guest::GuestCancelExecResponse, GuestSessionError> {
        cancel_exec(&self.tasks, &binding, request).await
    }

    async fn inspect_exec(
        &self,
        request: guest::GuestInspectExecRequest,
        binding: GuestStreamBinding,
    ) -> Result<guest::GuestInspectExecResponse, GuestSessionError> {
        inspect_exec(&self.tasks, &binding, request).await
    }

    async fn serve_retained_log(
        &self,
        request: guest::GuestOpenExecRetainedLogRequest,
        _: terminal::TerminalOpenResponse,
        binding: GuestStreamBinding,
        stream: GuestStream,
    ) -> Result<terminal::TerminalRetainedLogRange, GuestSessionError> {
        let (range, chunk) = prepare_retained_log(&self.tasks, &binding, &request).await?;
        let served_range = range.clone();
        tokio::spawn(async move {
            let _ = drive_retained_log_stream(binding, request, served_range, chunk, stream).await;
        });
        Ok(range)
    }

    async fn serve_shell(
        &self,
        _: guest::GuestOpenShellRequest,
        _: terminal::TerminalOpenResponse,
        binding: GuestStreamBinding,
        stream: GuestStream,
    ) -> Result<(), GuestSessionError> {
        if self.tasks.shell.is_none() {
            return Err(GuestSessionError::Service);
        }
        let tasks = self.tasks.clone();
        tokio::spawn(async move {
            let _ = drive_shell_stream(tasks, binding, stream).await;
        });
        Ok(())
    }

    async fn serve_file_transfer(
        &self,
        request: guest::GuestFileTransferRequest,
        response: terminal::TerminalOpenResponse,
        binding: GuestStreamBinding,
        stream: GuestStream,
    ) -> Result<(), GuestSessionError> {
        let key = (
            request.artifact.value(),
            request.configured_intent_id.clone(),
        );
        let endpoint = self
            .tasks
            .artifacts
            .get(&key)
            .filter(|endpoint| endpoint.direction.value() == request.direction.value())
            .cloned()
            .ok_or(GuestSessionError::Service)?;
        tokio::spawn(async move {
            let _ = drive_file_transfer(request, response, binding, endpoint, stream).await;
        });
        Ok(())
    }

    async fn serve_security_key(
        &self,
        request: guest::GuestSecurityKeyRequest,
        response: terminal::TerminalOpenResponse,
        binding: GuestStreamBinding,
        stream: GuestStream,
    ) -> Result<(), GuestSessionError> {
        let backend = self
            .tasks
            .security_key
            .as_ref()
            .filter(|backend| backend.ready())
            .cloned()
            .ok_or(GuestSessionError::Service)?;
        let ceremony = backend.begin(&request).await?;
        tokio::spawn(async move {
            let _ = drive_security_key(request, response, binding, ceremony, stream).await;
        });
        Ok(())
    }

    async fn shutdown(
        &self,
        request: guest::GuestShutdownRequest,
    ) -> Result<guest::GuestShutdownResponse, GuestSessionError> {
        let backend = self
            .shutdown
            .as_ref()
            .filter(|backend| backend.ready())
            .ok_or(GuestSessionError::Service)?;
        let context = request.context.as_ref().ok_or(GuestSessionError::Service)?;
        let operation_id = context.operation_id.clone();
        let request_digest = Sha256::digest(
            request
                .write_to_bytes()
                .map_err(|_| GuestSessionError::Service)?,
        )
        .to_vec();
        if let Some(success_digest) = self
            .shutdown_success
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&operation_id)
            .cloned()
        {
            if success_digest != request_digest {
                return Err(GuestSessionError::Service);
            }
            return Ok(shutdown_succeeded(
                &request,
                guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_ALREADY_APPLIED,
            ));
        }
        let (flight, owns_dispatch) = {
            let mut flights = self
                .shutdown_inflight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match flights.get(&operation_id) {
                Some(flight) => (Arc::clone(flight), false),
                None => {
                    let flight = Arc::new(ShutdownFlight {
                        result: Mutex::new(None),
                        notify: tokio::sync::Notify::new(),
                    });
                    flights.insert(operation_id.clone(), Arc::clone(&flight));
                    (flight, true)
                }
            }
        };
        let remaining_ms = request.deadline_unix_ms.saturating_sub(unix_time_ms());
        if !owns_dispatch {
            let resolution = tokio::time::timeout(Duration::from_millis(remaining_ms), async {
                loop {
                    let notified = flight.notify.notified();
                    if let Some(result) = *flight
                        .result
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                    {
                        return result;
                    }
                    notified.await;
                }
            })
            .await
            .map_err(|_| GuestSessionError::Service)?;
            return Ok(match resolution {
                Ok(()) => shutdown_succeeded(
                    &request,
                    guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_COMPLETED,
                ),
                Err(kind) => shutdown_failed(&request, kind),
            });
        }
        let resolution = if remaining_ms == 0 {
            Err(ErrorKind::ERROR_KIND_DEADLINE_EXCEEDED)
        } else {
            match backend
                .request(
                    request.action.enum_value_or_default(),
                    Duration::from_millis(remaining_ms),
                )
                .await
            {
                Ok(()) => Ok(()),
                Err(_) => Err(ErrorKind::ERROR_KIND_UNAVAILABLE),
            }
        };
        *flight
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(resolution);
        if resolution.is_ok() {
            self.shutdown_success
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(operation_id.clone(), request_digest);
        }
        self.shutdown_inflight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&operation_id);
        flight.notify.notify_waiters();
        Ok(match resolution {
            Ok(()) => shutdown_succeeded(
                &request,
                guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_COMPLETED,
            ),
            Err(kind) => shutdown_failed(&request, kind),
        })
    }

    fn disconnect(&self, owner_key: &[u8]) {
        if let Some(attached) = self.tasks.attached.as_ref() {
            attached.close_connection(&owner_key.to_vec());
        }
        if let Some(shell) = self.tasks.shell.as_ref() {
            shell.close_connection(owner_key);
        }
        self.tasks
            .records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, record| {
                !matches!(
                    &record.kind,
                    ExecOwnerKind::Attached {
                        owner_key: record_owner,
                        ..
                    } if record_owner == owner_key
                )
            });
    }
}

async fn drive_exec_stream(
    tasks: ProductionTaskContext,
    binding: GuestStreamBinding,
    mut stream: GuestStream,
) -> Result<(), GuestSessionError> {
    let mut validator = TerminalStreamValidator::new(
        terminal::TerminalKind::TERMINAL_KIND_EXEC,
        binding.generation,
        fixed_request_id(&binding.request_id)?,
        binding.operation_id.clone(),
        binding.resource_handle.clone(),
    )
    .map_err(|_| GuestSessionError::Service)?;
    let selection =
        match receive_terminal(&mut stream, &mut validator, &binding.cancellation).await? {
            Some(frame) => match frame.frame {
                Some(terminal::terminal_stream_frame::Frame::Select(selection)) => selection,
                _ => return reset_stream(stream).await,
            },
            None => return reset_stream(stream).await,
        };
    let exec = match selection.selection {
        Some(terminal::terminal_selection::Selection::Exec(exec)) => exec,
        _ => return reset_stream(stream).await,
    };
    let argv = resolve_exec_argv(&tasks, &binding, &exec)?;
    let input = ExecCreateInput {
        argv: argv.clone(),
        user: None,
        cwd: None,
        env: Vec::new(),
        tty: exec.tty,
        stdin_open: exec.tty,
        detached: exec.detached,
        has_terminal_size: exec.initial_size.is_some(),
        max_chunk_bytes: MAX_TERMINAL_CHUNK_BYTES as u64,
        direct_workload_tty: false,
    };
    let initial_size = exec
        .initial_size
        .as_ref()
        .map(|size| (size.rows, size.columns));
    let argv_digest = digest_argv(&argv);
    let created_at_unix_ms = unix_time_ms();
    let owner = binding.owner_key.clone();
    let (record, snapshot) = if exec.detached {
        let attached = tasks.attached.as_ref().ok_or(GuestSessionError::Service)?;
        let detached = tasks.detached.as_ref().ok_or(GuestSessionError::Service)?;
        let command = validate_and_authorize_detached(&input, attached.policy())
            .map_err(|_| GuestSessionError::Service)?;
        let (_, snapshot) = detached
            .create_with_exec_id(
                &tasks.boot_id,
                command,
                detached.default_caps(),
                binding.resource_handle.clone(),
            )
            .await
            .map_err(|_| GuestSessionError::Service)?;
        (
            ExecRecord {
                kind: ExecOwnerKind::Detached,
                created_at_unix_ms,
                argv_digest,
                last_cancel_sequence: 0,
            },
            snapshot,
        )
    } else {
        let attached = tasks.attached.as_ref().ok_or(GuestSessionError::Service)?;
        let (runtime_id, snapshot) = if exec.tty {
            let (runtime_id, snapshot, _) = attached
                .create_tty(owner.clone(), tasks.boot_id.clone(), input, initial_size)
                .await
                .map_err(|_| GuestSessionError::Service)?;
            (runtime_id, snapshot)
        } else {
            attached
                .create(owner.clone(), tasks.boot_id.clone(), input)
                .await
                .map_err(|_| GuestSessionError::Service)?
        };
        (
            ExecRecord {
                kind: ExecOwnerKind::Attached {
                    runtime_id,
                    owner_key: owner,
                },
                created_at_unix_ms,
                argv_digest,
                last_cancel_sequence: 0,
            },
            snapshot,
        )
    };
    tasks
        .records
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(binding.resource_handle.clone(), record.clone());

    let mut server_sequence = 0;
    send_terminal(
        &stream,
        &mut validator,
        &binding,
        &mut server_sequence,
        terminal::terminal_stream_frame::Frame::Started(terminal::TerminalStarted {
            kind: EnumOrUnknown::new(terminal::TerminalKind::TERMINAL_KIND_EXEC),
            tty: exec.tty,
            stdout_offset: snapshot.stdout_start_offset,
            stderr_offset: snapshot.stderr_start_offset,
            ..Default::default()
        }),
    )
    .await?;
    if exec.detached {
        send_terminal(
            &stream,
            &mut validator,
            &binding,
            &mut server_sequence,
            terminal::terminal_stream_frame::Frame::Outcome(terminal::TerminalOutcome {
                outcome: Some(terminal::terminal_outcome::Outcome::Detached(
                    terminal::TerminalDetached::new(),
                )),
                ..Default::default()
            }),
        )
        .await?;
        return stream.close().await;
    }
    let result = drive_attached_exec(
        &tasks,
        &binding,
        &record,
        &mut validator,
        &mut server_sequence,
        snapshot,
        &mut stream,
    )
    .await;
    if result.is_err()
        && let ExecOwnerKind::Attached {
            runtime_id,
            owner_key,
        } = &record.kind
        && let Some(runtime) = tasks.attached.as_ref()
    {
        let _ = runtime.cancel_exec(owner_key, runtime_id, &tasks.boot_id);
    }
    result
}

#[derive(Debug, Clone, Copy)]
struct OutputEofState {
    stdout: bool,
    stderr: bool,
}

impl OutputEofState {
    fn new(merged_tty: bool) -> Self {
        Self {
            stdout: false,
            stderr: merged_tty,
        }
    }

    fn should_poll(self, stream: ExecStream) -> bool {
        match stream {
            ExecStream::Stdout => !self.stdout,
            ExecStream::Stderr => !self.stderr,
        }
    }

    fn observe(&mut self, stream: ExecStream) {
        match stream {
            ExecStream::Stdout => self.stdout = true,
            ExecStream::Stderr => self.stderr = true,
        }
    }

    fn complete(self) -> bool {
        self.stdout && self.stderr
    }
}

async fn drive_attached_exec(
    tasks: &ProductionTaskContext,
    binding: &GuestStreamBinding,
    record: &ExecRecord,
    validator: &mut TerminalStreamValidator,
    server_sequence: &mut u64,
    initial: ExecSnapshot,
    stream: &mut GuestStream,
) -> Result<(), GuestSessionError> {
    let ExecOwnerKind::Attached {
        runtime_id,
        owner_key,
    } = &record.kind
    else {
        return Err(GuestSessionError::Service);
    };
    let runtime = tasks.attached.as_ref().ok_or(GuestSessionError::Service)?;
    let mut stdout_offset = initial.stdout_start_offset;
    let mut stderr_offset = initial.stderr_start_offset;
    let mut stdin_offset = 0_u64;
    let mut eof = OutputEofState::new(matches!(
        initial.stdin,
        TtyStdinSnapshot::Open | TtyStdinSnapshot::Closing | TtyStdinSnapshot::Closed
    ));
    loop {
        for output in [ExecStream::Stdout, ExecStream::Stderr] {
            if !eof.should_poll(output) {
                continue;
            }
            let offset = match output {
                ExecStream::Stdout => stdout_offset,
                ExecStream::Stderr => stderr_offset,
            };
            match runtime
                .read_output(
                    owner_key,
                    runtime_id,
                    &tasks.boot_id,
                    output,
                    offset,
                    MAX_TERMINAL_CHUNK_BYTES as u64,
                    false,
                    0,
                )
                .await
            {
                Ok((chunk, _)) if !chunk.data.is_empty() || chunk.eof => {
                    let frame = terminal_output_frame(output, &chunk);
                    send_terminal(stream, validator, binding, server_sequence, frame).await?;
                    match output {
                        ExecStream::Stdout => stdout_offset = chunk.next_offset,
                        ExecStream::Stderr => stderr_offset = chunk.next_offset,
                    }
                    if chunk.eof {
                        eof.observe(output);
                    }
                }
                Ok(_) => {}
                Err(ExecError::TtyStderrUnavailable) => eof.observe(ExecStream::Stderr),
                Err(_) => {
                    return cancel_and_reset(runtime, owner_key, runtime_id, tasks, stream).await;
                }
            }
        }

        let snapshot = runtime
            .inspect(owner_key, runtime_id, &tasks.boot_id)
            .map_err(|_| GuestSessionError::Service)?;
        if !matches!(snapshot.state, ExecState::Running)
            && eof.complete()
            && stdout_offset >= snapshot.stdout_end_offset
            && stderr_offset >= snapshot.stderr_end_offset
        {
            send_terminal(
                stream,
                validator,
                binding,
                server_sequence,
                terminal::terminal_stream_frame::Frame::Outcome(snapshot_outcome(snapshot)),
            )
            .await?;
            return stream.close().await;
        }

        tokio::select! {
            () = binding.cancellation.cancelled() => {
                return cancel_and_reset(runtime, owner_key, runtime_id, tasks, stream).await;
            }
            input = stream.receive() => {
                match input {
                    GuestStreamInput::Message(bytes) => {
                        let len = bytes.len();
                        let frame = terminal::TerminalStreamFrame::parse_from_bytes(&bytes)
                            .map_err(|_| GuestSessionError::Service)?;
                        validator
                            .accept(TerminalFrameDirection::ClientToServer, &frame)
                            .map_err(|_| GuestSessionError::Service)?;
                        stream.consume(len).await?;
                        use terminal::terminal_stream_frame::Frame;
                        match frame.frame {
                            Some(Frame::Stdin(stdin)) => {
                                let accepted = runtime
                                    .write_stdin(
                                        owner_key,
                                        runtime_id,
                                        &tasks.boot_id,
                                        stdin.offset,
                                        &stdin.data,
                                        stdin.eof,
                                    )
                                    .await
                                    .map_err(|_| GuestSessionError::Service)?;
                                stdin_offset = accepted.next_offset;
                                send_terminal(
                                    stream,
                                    validator,
                                    binding,
                                    server_sequence,
                                    Frame::Status(terminal::TerminalStatus {
                                        status: EnumOrUnknown::new(
                                            if accepted.closed {
                                                terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_CLOSED
                                            } else {
                                                terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_ACCEPTED
                                            },
                                        ),
                                        next_stdin_offset: stdin_offset,
                                        ..Default::default()
                                    }),
                                )
                                .await?;
                            }
                            Some(Frame::CloseStdin(_)) => {
                                let (next, _) = runtime
                                    .close_stdin(owner_key, runtime_id, &tasks.boot_id, stdin_offset)
                                    .await
                                    .map_err(|_| GuestSessionError::Service)?;
                                stdin_offset = next;
                                send_terminal(
                                    stream,
                                    validator,
                                    binding,
                                    server_sequence,
                                    Frame::Status(terminal::TerminalStatus {
                                        status: EnumOrUnknown::new(
                                            terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_STDIN_CLOSED,
                                        ),
                                        next_stdin_offset: stdin_offset,
                                        ..Default::default()
                                    }),
                                )
                                .await?;
                            }
                            Some(Frame::Resize(resize)) => {
                                let size = resize.size.as_ref().ok_or(GuestSessionError::Service)?;
                                runtime
                                    .tty_resize(
                                        owner_key,
                                        runtime_id,
                                        &tasks.boot_id,
                                        resize.operation_sequence,
                                        size.rows,
                                        size.columns,
                                    )
                                    .map_err(|_| GuestSessionError::Service)?;
                                send_control_applied(
                                    stream,
                                    validator,
                                    binding,
                                    server_sequence,
                                    stdin_offset,
                                )
                                .await?;
                            }
                            Some(Frame::Signal(signal)) => {
                                runtime
                                    .tty_signal(
                                        owner_key,
                                        runtime_id,
                                        &tasks.boot_id,
                                        signal.operation_sequence,
                                        terminal_signal_raw(signal.signal.enum_value_or_default())?,
                                    )
                                    .map_err(|_| GuestSessionError::Service)?;
                                send_control_applied(
                                    stream,
                                    validator,
                                    binding,
                                    server_sequence,
                                    stdin_offset,
                                )
                                .await?;
                            }
                            Some(Frame::Detach(_) | Frame::Cancel(_) | Frame::Close(_)) => {
                                let _ = runtime.cancel_exec(owner_key, runtime_id, &tasks.boot_id);
                                send_terminal(
                                    stream,
                                    validator,
                                    binding,
                                    server_sequence,
                                    Frame::Outcome(terminal::TerminalOutcome {
                                        outcome: Some(terminal::terminal_outcome::Outcome::Cancelled(
                                            terminal::TerminalCancelled::new(),
                                        )),
                                        ..Default::default()
                                    }),
                                )
                                .await?;
                                return stream.close().await;
                            }
                            _ => return cancel_and_reset(runtime, owner_key, runtime_id, tasks, stream).await,
                        }
                    }
                    GuestStreamInput::RemoteClosed
                    | GuestStreamInput::Reset
                    | GuestStreamInput::Disconnected => {
                        return cancel_and_reset(runtime, owner_key, runtime_id, tasks, stream).await;
                    }
                }
            }
            () = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }
}

fn resolve_exec_argv(
    tasks: &ProductionTaskContext,
    binding: &GuestStreamBinding,
    exec: &terminal::ExecSelection,
) -> Result<Vec<String>, GuestSessionError> {
    match exec.selection.as_ref().ok_or(GuestSessionError::Service)? {
        terminal::exec_selection::Selection::Arbitrary(arbitrary)
            if exec.authority.enum_value().ok()
                == Some(terminal::ExecAuthority::EXEC_AUTHORITY_ADMIN_ARBITRARY) =>
        {
            arbitrary
                .argv
                .iter()
                .map(|argument| {
                    std::str::from_utf8(argument)
                        .map(str::to_owned)
                        .map_err(|_| GuestSessionError::Service)
                })
                .collect()
        }
        terminal::exec_selection::Selection::ConfiguredLaunch(configured)
            if exec.authority.enum_value().ok()
                == Some(terminal::ExecAuthority::EXEC_AUTHORITY_CONFIGURED_LAUNCH) =>
        {
            if tasks.configured_launch_realm_id.as_deref() != Some(binding.scope.realm_id.as_str())
                || tasks.configured_launch_workload_digest.is_none()
            {
                return Err(GuestSessionError::Service);
            }
            tasks
                .configured_launches
                .get(&configured.configured_item_id)
                .cloned()
                .ok_or(GuestSessionError::Service)
        }
        _ => Err(GuestSessionError::Service),
    }
}

async fn receive_terminal(
    stream: &mut GuestStream,
    validator: &mut TerminalStreamValidator,
    cancellation: &Cancellation,
) -> Result<Option<terminal::TerminalStreamFrame>, GuestSessionError> {
    let input = tokio::select! {
        () = cancellation.cancelled() => return Ok(None),
        input = stream.receive() => input,
    };
    match input {
        GuestStreamInput::Message(bytes) => {
            let len = bytes.len();
            let frame = terminal::TerminalStreamFrame::parse_from_bytes(&bytes)
                .map_err(|_| GuestSessionError::Service)?;
            validator
                .accept(TerminalFrameDirection::ClientToServer, &frame)
                .map_err(|_| GuestSessionError::Service)?;
            stream.consume(len).await?;
            Ok(Some(frame))
        }
        GuestStreamInput::RemoteClosed
        | GuestStreamInput::Reset
        | GuestStreamInput::Disconnected => Ok(None),
    }
}

async fn send_terminal(
    stream: &GuestStream,
    validator: &mut TerminalStreamValidator,
    binding: &GuestStreamBinding,
    sequence: &mut u64,
    frame: terminal::terminal_stream_frame::Frame,
) -> Result<(), GuestSessionError> {
    let message = terminal::TerminalStreamFrame {
        session_generation: binding.generation,
        request_id: binding.request_id.clone(),
        sequence: *sequence,
        operation_id: binding.operation_id.clone(),
        resource_handle: binding.resource_handle.clone(),
        frame: Some(frame),
        ..Default::default()
    };
    validator
        .accept(TerminalFrameDirection::ServerToClient, &message)
        .map_err(|_| GuestSessionError::Service)?;
    stream.send(&message).await?;
    *sequence = sequence.checked_add(1).ok_or(GuestSessionError::Service)?;
    Ok(())
}

async fn send_control_applied(
    stream: &GuestStream,
    validator: &mut TerminalStreamValidator,
    binding: &GuestStreamBinding,
    sequence: &mut u64,
    stdin_offset: u64,
) -> Result<(), GuestSessionError> {
    send_terminal(
        stream,
        validator,
        binding,
        sequence,
        terminal::terminal_stream_frame::Frame::Status(terminal::TerminalStatus {
            status: EnumOrUnknown::new(
                terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_CONTROL_APPLIED,
            ),
            next_stdin_offset: stdin_offset,
            ..Default::default()
        }),
    )
    .await
}

fn terminal_output_frame(
    output: ExecStream,
    chunk: &RingChunk,
) -> terminal::terminal_stream_frame::Frame {
    let value = terminal::TerminalOutput {
        offset: chunk.next_offset.saturating_sub(chunk.data.len() as u64),
        data: chunk.data.clone(),
        eof: chunk.eof,
        dropped_bytes: chunk.dropped_bytes,
        truncated: chunk.truncated,
        ..Default::default()
    };
    match output {
        ExecStream::Stdout => terminal::terminal_stream_frame::Frame::Stdout(value),
        ExecStream::Stderr => terminal::terminal_stream_frame::Frame::Stderr(value),
    }
}

fn snapshot_outcome(snapshot: ExecSnapshot) -> terminal::TerminalOutcome {
    let outcome = match (snapshot.state, snapshot.outcome) {
        (ExecState::Exited, Some(ExitOutcome::Exited(code))) if (0..=255).contains(&code) => {
            terminal::terminal_outcome::Outcome::Exited(terminal::TerminalExited {
                exit_code: code,
                ..Default::default()
            })
        }
        (ExecState::Signaled, Some(ExitOutcome::Signaled(signal)))
            if (1..=64).contains(&signal) =>
        {
            terminal::terminal_outcome::Outcome::Signaled(terminal::TerminalSignaled {
                signal,
                ..Default::default()
            })
        }
        (ExecState::Cancelled, _) => {
            terminal::terminal_outcome::Outcome::Cancelled(terminal::TerminalCancelled::new())
        }
        _ => terminal::terminal_outcome::Outcome::Failed(terminal::TerminalFailed {
            error: EnumOrUnknown::new(terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_INTERNAL),
            retry: EnumOrUnknown::new(RetryClass::RETRY_CLASS_NEVER),
            ..Default::default()
        }),
    };
    terminal::TerminalOutcome {
        outcome: Some(outcome),
        ..Default::default()
    }
}

async fn cancel_and_reset(
    runtime: &Arc<AttachedRuntime>,
    owner: &ConnectionKey,
    runtime_id: &str,
    tasks: &ProductionTaskContext,
    stream: &mut GuestStream,
) -> Result<(), GuestSessionError> {
    let _ = runtime.cancel_exec(owner, runtime_id, &tasks.boot_id);
    stream.reset().await
}

async fn reset_stream(mut stream: GuestStream) -> Result<(), GuestSessionError> {
    stream.reset().await
}

async fn cancel_exec(
    tasks: &ProductionTaskContext,
    binding: &GuestStreamBinding,
    request: guest::GuestCancelExecRequest,
) -> Result<guest::GuestCancelExecResponse, GuestSessionError> {
    let context = request.context.as_ref().ok_or(GuestSessionError::Service)?;
    let metadata = context
        .metadata
        .as_ref()
        .ok_or(GuestSessionError::Service)?;
    let record = tasks
        .records
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&request.resource_handle)
        .cloned();
    let Some(mut record) = record else {
        let Some(detached) = tasks.detached.as_ref() else {
            return Ok(cancel_error(
                &request,
                guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_UNKNOWN_RESOURCE,
                ErrorKind::ERROR_KIND_NOT_FOUND,
            ));
        };
        let duplicate = {
            let mut sequences = tasks
                .adopted_cancel_sequences
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous = sequences
                .get(&request.resource_handle)
                .copied()
                .unwrap_or(0);
            if request.control_sequence <= previous {
                true
            } else {
                sequences.insert(request.resource_handle.clone(), request.control_sequence);
                false
            }
        };
        let already_terminal = if duplicate {
            true
        } else {
            match detached
                .cancel_verified(&request.resource_handle, &tasks.boot_id)
                .await
            {
                Ok(already_terminal) => already_terminal,
                Err(_) => {
                    tasks
                        .adopted_cancel_sequences
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .remove(&request.resource_handle);
                    return Ok(cancel_error(
                        &request,
                        guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_UNKNOWN_RESOURCE,
                        ErrorKind::ERROR_KIND_NOT_FOUND,
                    ));
                }
            }
        };
        return Ok(cancel_success(&request, already_terminal));
    };
    if let ExecOwnerKind::Attached { owner_key, .. } = &record.kind
        && owner_key != &binding.owner_key
    {
        return Ok(cancel_error(
            &request,
            guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_UNKNOWN_RESOURCE,
            ErrorKind::ERROR_KIND_NOT_FOUND,
        ));
    }
    if request.control_sequence <= record.last_cancel_sequence {
        return Ok(guest::GuestCancelExecResponse {
                        outcome: EnumOrUnknown::new(Outcome::OUTCOME_NOT_APPLICABLE),
                        operation_id: context.operation_id.clone(),
                        session_generation: metadata.session_generation,
                        request_id: metadata.request_id.clone(),
                        resource_handle: request.resource_handle,
                        cancellation: EnumOrUnknown::new(
                            guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_ALREADY_TERMINAL,
                        ),
                        ..Default::default()
                    });
    }
    record.last_cancel_sequence = request.control_sequence;
    tasks
        .records
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(request.resource_handle.clone(), record.clone());
    let already_terminal = match &record.kind {
        ExecOwnerKind::Attached {
            runtime_id,
            owner_key,
        } => tasks
            .attached
            .as_ref()
            .ok_or(GuestSessionError::Service)?
            .cancel_exec(owner_key, runtime_id, &tasks.boot_id)
            .map_err(|_| GuestSessionError::Service)?,
        ExecOwnerKind::Detached => tasks
            .detached
            .as_ref()
            .ok_or(GuestSessionError::Service)?
            .cancel(&request.resource_handle, &tasks.boot_id)
            .await
            .map_err(|_| GuestSessionError::Service)?,
    };
    Ok(cancel_success(&request, already_terminal))
}

fn cancel_success(
    request: &guest::GuestCancelExecRequest,
    already_terminal: bool,
) -> guest::GuestCancelExecResponse {
    let context = request.context.as_ref().expect("validated request");
    let metadata = context.metadata.as_ref().expect("validated request");
    guest::GuestCancelExecResponse {
        outcome: EnumOrUnknown::new(if already_terminal {
            Outcome::OUTCOME_NOT_APPLICABLE
        } else {
            Outcome::OUTCOME_ACCEPTED
        }),
        operation_id: context.operation_id.clone(),
        session_generation: metadata.session_generation,
        request_id: metadata.request_id.clone(),
        resource_handle: request.resource_handle.clone(),
        cancellation: EnumOrUnknown::new(if already_terminal {
            guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_ALREADY_TERMINAL
        } else {
            guest::GuestExecCancellationOutcome::GUEST_EXEC_CANCELLATION_OUTCOME_SIGNALLED
        }),
        ..Default::default()
    }
}

async fn inspect_exec(
    tasks: &ProductionTaskContext,
    binding: &GuestStreamBinding,
    request: guest::GuestInspectExecRequest,
) -> Result<guest::GuestInspectExecResponse, GuestSessionError> {
    let context = request.context.as_ref().ok_or(GuestSessionError::Service)?;
    let metadata = context
        .metadata
        .as_ref()
        .ok_or(GuestSessionError::Service)?;
    let query = request
        .query
        .as_ref()
        .and_then(|query| query.query.as_ref())
        .ok_or(GuestSessionError::Service)?;
    let result = match query {
        guest::guest_inspect_exec_query::Query::Status(status) => {
            let snapshot = match snapshot_for(tasks, binding, &status.resource_handle).await {
                Ok(snapshot) => snapshot,
                Err(_) => {
                    return Ok(inspect_error(&request, ErrorKind::ERROR_KIND_NOT_FOUND));
                }
            };
            guest::guest_inspect_exec_response::Result::Status(status_from_snapshot(
                status.resource_handle.clone(),
                snapshot,
                false,
            ))
        }
        guest::guest_inspect_exec_query::Query::Wait(wait) => {
            let (snapshot, timed_out) = match wait_for(tasks, binding, wait).await {
                Ok(result) => result,
                Err(_) => {
                    return Ok(inspect_error(&request, ErrorKind::ERROR_KIND_NOT_FOUND));
                }
            };
            guest::guest_inspect_exec_response::Result::Status(status_from_snapshot(
                wait.resource_handle.clone(),
                snapshot,
                timed_out,
            ))
        }
        guest::guest_inspect_exec_query::Query::ListPage(page) => {
            let mut entries = match list_entries(tasks, binding).await {
                Ok(entries) => entries,
                Err(_) => {
                    return Ok(inspect_error(&request, ErrorKind::ERROR_KIND_UNAVAILABLE));
                }
            };
            entries.sort_by(|left, right| left.resource_handle.cmp(&right.resource_handle));
            let start = if page.page_cursor.is_empty() {
                0
            } else {
                page.page_cursor
                    .parse::<usize>()
                    .map_err(|_| GuestSessionError::Service)?
            };
            if start > entries.len() {
                return Err(GuestSessionError::Service);
            }
            let end = start
                .saturating_add(page.page_size as usize)
                .min(entries.len());
            let truncated = end < entries.len();
            guest::guest_inspect_exec_response::Result::ListPage(guest::GuestExecListPage {
                entries: entries[start..end].to_vec(),
                truncated,
                next_page_cursor: if truncated {
                    end.to_string()
                } else {
                    String::new()
                },
                ..Default::default()
            })
        }
        _ => return Err(GuestSessionError::Service),
    };
    Ok(guest::GuestInspectExecResponse {
        outcome: EnumOrUnknown::new(Outcome::OUTCOME_SUCCEEDED),
        operation_id: context.operation_id.clone(),
        session_generation: metadata.session_generation,
        request_id: metadata.request_id.clone(),
        result: Some(result),
        ..Default::default()
    })
}

async fn snapshot_for(
    tasks: &ProductionTaskContext,
    binding: &GuestStreamBinding,
    handle: &str,
) -> Result<ExecSnapshot, GuestSessionError> {
    let record = tasks
        .records
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(handle)
        .cloned();
    match record.map(|record| record.kind) {
        Some(ExecOwnerKind::Attached {
            runtime_id,
            owner_key,
        }) if owner_key == binding.owner_key => tasks
            .attached
            .as_ref()
            .ok_or(GuestSessionError::Service)?
            .inspect(&owner_key, &runtime_id, &tasks.boot_id)
            .map_err(|_| GuestSessionError::Service),
        Some(ExecOwnerKind::Attached { .. }) => Err(GuestSessionError::Service),
        Some(ExecOwnerKind::Detached) | None => tasks
            .detached
            .as_ref()
            .ok_or(GuestSessionError::Service)?
            .inspect(handle, &tasks.boot_id)
            .await
            .map_err(|_| GuestSessionError::Service),
    }
}

async fn wait_for(
    tasks: &ProductionTaskContext,
    binding: &GuestStreamBinding,
    query: &guest::GuestExecWaitQuery,
) -> Result<(ExecSnapshot, bool), GuestSessionError> {
    let record = tasks
        .records
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&query.resource_handle)
        .cloned();
    match record.map(|record| record.kind) {
        Some(ExecOwnerKind::Attached {
            runtime_id,
            owner_key,
        }) if owner_key == binding.owner_key => tasks
            .attached
            .as_ref()
            .ok_or(GuestSessionError::Service)?
            .wait(
                &owner_key,
                &runtime_id,
                &tasks.boot_id,
                Some(query.known_state_generation),
                u64::from(query.timeout_ms),
            )
            .await
            .map_err(|_| GuestSessionError::Service),
        Some(ExecOwnerKind::Attached { .. }) => Err(GuestSessionError::Service),
        Some(ExecOwnerKind::Detached) | None => tasks
            .detached
            .as_ref()
            .ok_or(GuestSessionError::Service)?
            .wait(
                &query.resource_handle,
                &tasks.boot_id,
                Some(query.known_state_generation),
                u64::from(query.timeout_ms),
            )
            .await
            .map_err(|_| GuestSessionError::Service),
    }
}

async fn list_entries(
    tasks: &ProductionTaskContext,
    binding: &GuestStreamBinding,
) -> Result<Vec<guest::GuestExecListEntry>, GuestSessionError> {
    let records = tasks
        .records
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let mut entries = Vec::new();
    for (handle, record) in &records {
        if let ExecOwnerKind::Attached {
            runtime_id,
            owner_key,
        } = &record.kind
        {
            if owner_key != &binding.owner_key {
                continue;
            }
            if let Ok(snapshot) = tasks
                .attached
                .as_ref()
                .ok_or(GuestSessionError::Service)?
                .inspect(owner_key, runtime_id, &tasks.boot_id)
            {
                entries.push(list_entry_from_snapshot(handle, record, snapshot));
            }
        }
    }
    if let Some(detached) = tasks.detached.as_ref() {
        for detached_entry in detached
            .list(&tasks.boot_id)
            .await
            .map_err(|_| GuestSessionError::Service)?
        {
            entries.push(guest::GuestExecListEntry {
                resource_handle: detached_entry.exec_id,
                state: EnumOrUnknown::new(exec_state(detached_entry.state)),
                created_at_unix_ms: detached_entry.create_time_unix,
                argv_digest: decode_digest(&detached_entry.argv_sha256)?,
                dropped_bytes: detached_entry.dropped_bytes,
                stdout_truncated: detached_entry.stdout_truncated,
                stderr_truncated: detached_entry.stderr_truncated,
                ..Default::default()
            });
        }
    }
    Ok(entries)
}

fn list_entry_from_snapshot(
    handle: &str,
    record: &ExecRecord,
    snapshot: ExecSnapshot,
) -> guest::GuestExecListEntry {
    guest::GuestExecListEntry {
        resource_handle: handle.to_owned(),
        state: EnumOrUnknown::new(exec_state(snapshot.state)),
        created_at_unix_ms: record.created_at_unix_ms,
        argv_digest: record.argv_digest.to_vec(),
        stdout_bytes: snapshot.stdout_end_offset,
        stderr_bytes: snapshot.stderr_end_offset,
        dropped_bytes: snapshot
            .stdout_dropped_bytes
            .saturating_add(snapshot.stderr_dropped_bytes),
        stdout_truncated: snapshot.stdout_truncated,
        stderr_truncated: snapshot.stderr_truncated,
        ..Default::default()
    }
}

fn status_from_snapshot(
    handle: String,
    snapshot: ExecSnapshot,
    timed_out: bool,
) -> guest::GuestExecStatus {
    let terminal = !matches!(snapshot.state, ExecState::Running);
    guest::GuestExecStatus {
        resource_handle: handle,
        state: EnumOrUnknown::new(exec_state(snapshot.state)),
        stdin_state: EnumOrUnknown::new(match snapshot.stdin {
            TtyStdinSnapshot::Open => guest::GuestStdinState::GUEST_STDIN_STATE_OPEN,
            TtyStdinSnapshot::Closing => guest::GuestStdinState::GUEST_STDIN_STATE_CLOSING,
            TtyStdinSnapshot::Closed => guest::GuestStdinState::GUEST_STDIN_STATE_CLOSED,
            TtyStdinSnapshot::NotInteractive => {
                guest::GuestStdinState::GUEST_STDIN_STATE_NOT_INTERACTIVE
            }
        }),
        terminal_outcome: if terminal {
            MessageField::some(snapshot_outcome(snapshot))
        } else {
            MessageField::none()
        },
        stdout_start_offset: snapshot.stdout_start_offset,
        stdout_end_offset: snapshot.stdout_end_offset,
        stderr_start_offset: snapshot.stderr_start_offset,
        stderr_end_offset: snapshot.stderr_end_offset,
        stdout_dropped_bytes: snapshot.stdout_dropped_bytes,
        stderr_dropped_bytes: snapshot.stderr_dropped_bytes,
        stdout_truncated: snapshot.stdout_truncated,
        stderr_truncated: snapshot.stderr_truncated,
        state_generation: snapshot.state_generation,
        timed_out,
        ..Default::default()
    }
}

fn exec_state(state: ExecState) -> guest::GuestExecState {
    match state {
        ExecState::Running => guest::GuestExecState::GUEST_EXEC_STATE_RUNNING,
        ExecState::Exited => guest::GuestExecState::GUEST_EXEC_STATE_EXITED,
        ExecState::Signaled => guest::GuestExecState::GUEST_EXEC_STATE_SIGNALED,
        ExecState::Cancelled => guest::GuestExecState::GUEST_EXEC_STATE_CANCELLED,
        ExecState::Reaped => guest::GuestExecState::GUEST_EXEC_STATE_REAPED,
        ExecState::LostGuestd => guest::GuestExecState::GUEST_EXEC_STATE_LOST,
    }
}

fn cancel_error(
    request: &guest::GuestCancelExecRequest,
    cancellation: guest::GuestExecCancellationOutcome,
    error: ErrorKind,
) -> guest::GuestCancelExecResponse {
    let context = request.context.as_ref().expect("validated request");
    let metadata = context.metadata.as_ref().expect("validated request");
    guest::GuestCancelExecResponse {
        outcome: EnumOrUnknown::new(Outcome::OUTCOME_FAILED),
        operation_id: context.operation_id.clone(),
        session_generation: metadata.session_generation,
        request_id: metadata.request_id.clone(),
        resource_handle: request.resource_handle.clone(),
        cancellation: EnumOrUnknown::new(cancellation),
        error: MessageField::some(error_envelope(error)),
        ..Default::default()
    }
}

fn inspect_error(
    request: &guest::GuestInspectExecRequest,
    error: ErrorKind,
) -> guest::GuestInspectExecResponse {
    let context = request.context.as_ref().expect("validated request");
    let metadata = context.metadata.as_ref().expect("validated request");
    guest::GuestInspectExecResponse {
        outcome: EnumOrUnknown::new(Outcome::OUTCOME_FAILED),
        operation_id: context.operation_id.clone(),
        session_generation: metadata.session_generation,
        request_id: metadata.request_id.clone(),
        error: MessageField::some(error_envelope(error)),
        ..Default::default()
    }
}

#[derive(Clone)]
struct RetainedChunk {
    offset: u64,
    data: Vec<u8>,
    eof: bool,
    dropped_bytes: u64,
    truncated: bool,
}

impl std::fmt::Debug for RetainedChunk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetainedChunk")
            .field("offset", &self.offset)
            .field("data", &"<redacted>")
            .field("len", &self.data.len())
            .field("eof", &self.eof)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .finish()
    }
}

async fn prepare_retained_log(
    tasks: &ProductionTaskContext,
    binding: &GuestStreamBinding,
    request: &guest::GuestOpenExecRetainedLogRequest,
) -> Result<(terminal::TerminalRetainedLogRange, RetainedChunk), GuestSessionError> {
    let output = match request.output.enum_value_or_default() {
        terminal::OutputStream::OUTPUT_STREAM_STDOUT => ExecStream::Stdout,
        terminal::OutputStream::OUTPUT_STREAM_STDERR => ExecStream::Stderr,
        terminal::OutputStream::OUTPUT_STREAM_UNSPECIFIED => {
            return Err(GuestSessionError::Service);
        }
    };
    let record = tasks
        .records
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&request.resource_handle)
        .cloned();
    let chunk = match record.map(|record| record.kind) {
        Some(ExecOwnerKind::Attached {
            runtime_id,
            owner_key,
        }) if owner_key == binding.owner_key => {
            let (chunk, _) = tasks
                .attached
                .as_ref()
                .ok_or(GuestSessionError::Service)?
                .read_output(
                    &owner_key,
                    &runtime_id,
                    &tasks.boot_id,
                    output,
                    request.offset,
                    u64::from(request.max_bytes),
                    false,
                    0,
                )
                .await
                .map_err(|_| GuestSessionError::Service)?;
            RetainedChunk {
                offset: request.offset,
                data: chunk.data,
                eof: chunk.eof,
                dropped_bytes: chunk.dropped_bytes,
                truncated: chunk.truncated,
            }
        }
        Some(ExecOwnerKind::Attached { .. }) => return Err(GuestSessionError::Service),
        Some(ExecOwnerKind::Detached) | None => {
            let chunk = tasks
                .detached
                .as_ref()
                .ok_or(GuestSessionError::Service)?
                .read_logs(
                    &request.resource_handle,
                    &tasks.boot_id,
                    output,
                    request.offset,
                    u64::from(request.max_bytes),
                )
                .await
                .map_err(|_| GuestSessionError::Service)?;
            RetainedChunk {
                offset: request.offset,
                data: chunk.data,
                eof: chunk.eof,
                dropped_bytes: chunk.dropped_bytes,
                truncated: chunk.truncated,
            }
        }
    };
    let end_offset = chunk
        .offset
        .checked_add(chunk.data.len() as u64)
        .ok_or(GuestSessionError::Service)?;
    Ok((
        terminal::TerminalRetainedLogRange {
            output: request.output,
            requested_offset: request.offset,
            start_offset: chunk.offset,
            end_offset,
            max_bytes: request.max_bytes,
            eof: chunk.eof,
            ..Default::default()
        },
        chunk,
    ))
}

async fn drive_retained_log_stream(
    binding: GuestStreamBinding,
    request: guest::GuestOpenExecRetainedLogRequest,
    range: terminal::TerminalRetainedLogRange,
    chunk: RetainedChunk,
    mut stream: GuestStream,
) -> Result<(), GuestSessionError> {
    let mut validator = TerminalStreamValidator::new(
        terminal::TerminalKind::TERMINAL_KIND_RETAINED_LOG,
        binding.generation,
        fixed_request_id(&binding.request_id)?,
        binding.operation_id.clone(),
        binding.resource_handle.clone(),
    )
    .map_err(|_| GuestSessionError::Service)?;
    validator
        .bind_retained_log_range(&range)
        .map_err(|_| GuestSessionError::Service)?;
    let selected = receive_terminal(&mut stream, &mut validator, &binding.cancellation).await?;
    if !matches!(
        selected.and_then(|frame| frame.frame),
        Some(terminal::terminal_stream_frame::Frame::Select(
            terminal::TerminalSelection {
                selection: Some(terminal::terminal_selection::Selection::RetainedLog(_)),
                ..
            }
        ))
    ) {
        return stream.reset().await;
    }
    let mut sequence = 0;
    send_terminal(
        &stream,
        &mut validator,
        &binding,
        &mut sequence,
        terminal::terminal_stream_frame::Frame::Started(terminal::TerminalStarted {
            kind: EnumOrUnknown::new(terminal::TerminalKind::TERMINAL_KIND_RETAINED_LOG),
            stdout_offset: if range.output.enum_value_or_default()
                == terminal::OutputStream::OUTPUT_STREAM_STDOUT
            {
                range.start_offset
            } else {
                0
            },
            stderr_offset: if range.output.enum_value_or_default()
                == terminal::OutputStream::OUTPUT_STREAM_STDERR
            {
                range.start_offset
            } else {
                0
            },
            ..Default::default()
        }),
    )
    .await?;
    if !chunk.data.is_empty() || chunk.eof {
        let output = terminal::TerminalOutput {
            offset: chunk.offset,
            data: chunk.data,
            eof: chunk.eof,
            dropped_bytes: chunk.dropped_bytes,
            truncated: chunk.truncated,
            ..Default::default()
        };
        let frame = if request.output.enum_value_or_default()
            == terminal::OutputStream::OUTPUT_STREAM_STDOUT
        {
            terminal::terminal_stream_frame::Frame::Stdout(output)
        } else {
            terminal::terminal_stream_frame::Frame::Stderr(output)
        };
        send_terminal(&stream, &mut validator, &binding, &mut sequence, frame).await?;
    }
    send_terminal(
        &stream,
        &mut validator,
        &binding,
        &mut sequence,
        terminal::terminal_stream_frame::Frame::Outcome(terminal::TerminalOutcome {
            outcome: Some(terminal::terminal_outcome::Outcome::Closed(
                terminal::TerminalClosed::new(),
            )),
            ..Default::default()
        }),
    )
    .await?;
    stream.close().await
}

async fn drive_shell_stream(
    tasks: ProductionTaskContext,
    binding: GuestStreamBinding,
    mut stream: GuestStream,
) -> Result<(), GuestSessionError> {
    let shell_runtime = tasks.shell.as_ref().ok_or(GuestSessionError::Service)?;
    let mut validator = TerminalStreamValidator::new(
        terminal::TerminalKind::TERMINAL_KIND_SHELL,
        binding.generation,
        fixed_request_id(&binding.request_id)?,
        binding.operation_id.clone(),
        binding.resource_handle.clone(),
    )
    .map_err(|_| GuestSessionError::Service)?;
    let selection =
        match receive_terminal(&mut stream, &mut validator, &binding.cancellation).await? {
            Some(frame) => match frame.frame {
                Some(terminal::terminal_stream_frame::Frame::Select(selection)) => selection,
                _ => return stream.reset().await,
            },
            None => return stream.reset().await,
        };
    let shell = match selection.selection {
        Some(terminal::terminal_selection::Selection::Shell(shell)) => shell,
        _ => return stream.reset().await,
    };
    let action = shell
        .action
        .enum_value()
        .map_err(|_| GuestSessionError::Service)?;
    match action {
        terminal::ShellAction::SHELL_ACTION_ATTACH_DEFAULT
        | terminal::ShellAction::SHELL_ACTION_ATTACH_CONFIGURED => {
            let name = if action == terminal::ShellAction::SHELL_ACTION_ATTACH_DEFAULT {
                shell_runtime.default_name()
            } else if shell.configured_shell_id == shell_runtime.default_name() {
                shell.configured_shell_id.clone()
            } else {
                return stream.reset().await;
            };
            let legacy = legacy_guest::ShellAttachRequest {
                name: Some(name.clone()),
                force: shell.force,
                ..Default::default()
            };
            let attached = shell_runtime.attach_with_owner(legacy, binding.owner_key.clone());
            if attached.error.is_some() {
                return stream.reset().await;
            }
            let shell_handle = attached.session_id.ok_or(GuestSessionError::Service)?;
            let runner_path = shell_runtime
                .runner_path()
                .ok_or(GuestSessionError::Service)?;
            let socket_path = shell_runtime
                .socket_path()
                .ok_or(GuestSessionError::Service)?;
            let initial = shell
                .initial_size
                .as_ref()
                .ok_or(GuestSessionError::Service)?;
            let mut argv = vec![
                runner_path.to_string_lossy().into_owned(),
                "attach".to_owned(),
                "--socket".to_owned(),
                socket_path.to_string_lossy().into_owned(),
                "--name".to_owned(),
                name.clone(),
            ];
            if shell.force {
                argv.push("--force".to_owned());
            }
            let input = ExecCreateInput {
                argv: argv.clone(),
                user: None,
                cwd: None,
                env: Vec::new(),
                tty: true,
                stdin_open: true,
                detached: false,
                has_terminal_size: true,
                max_chunk_bytes: MAX_TERMINAL_CHUNK_BYTES as u64,
                direct_workload_tty: false,
            };
            let runtime = tasks.attached.as_ref().ok_or(GuestSessionError::Service)?;
            let (runtime_id, snapshot, _) = runtime
                .create_tty(
                    binding.owner_key.clone(),
                    tasks.boot_id.clone(),
                    input,
                    Some((initial.rows, initial.columns)),
                )
                .await
                .map_err(|_| GuestSessionError::Service)?;
            shell_runtime
                .set_terminal_exec_id(&shell_handle, runtime_id.clone())
                .map_err(|_| GuestSessionError::Service)?;
            tasks
                .shell_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(
                    shell_handle.clone(),
                    ShellOwnerRecord {
                        name,
                        owner_key: binding.owner_key.clone(),
                        runtime_id: runtime_id.clone(),
                    },
                );
            let record = ExecRecord {
                kind: ExecOwnerKind::Attached {
                    runtime_id,
                    owner_key: binding.owner_key.clone(),
                },
                created_at_unix_ms: unix_time_ms(),
                argv_digest: digest_argv(&argv),
                last_cancel_sequence: 0,
            };
            let mut sequence = 0;
            send_terminal(
                &stream,
                &mut validator,
                &binding,
                &mut sequence,
                terminal::terminal_stream_frame::Frame::Started(terminal::TerminalStarted {
                    kind: EnumOrUnknown::new(terminal::TerminalKind::TERMINAL_KIND_SHELL),
                    tty: true,
                    stdout_offset: snapshot.stdout_start_offset,
                    stderr_offset: snapshot.stderr_start_offset,
                    ..Default::default()
                }),
            )
            .await?;
            let result = drive_attached_exec(
                &tasks,
                &binding,
                &record,
                &mut validator,
                &mut sequence,
                snapshot,
                &mut stream,
            )
            .await;
            if result.is_err()
                && let ExecOwnerKind::Attached {
                    runtime_id,
                    owner_key,
                } = &record.kind
            {
                let _ = runtime.cancel_exec(owner_key, runtime_id, &tasks.boot_id);
            }
            let _ = shell_runtime.close_attach(&shell_handle);
            result
        }
        terminal::ShellAction::SHELL_ACTION_LIST
        | terminal::ShellAction::SHELL_ACTION_DETACH
        | terminal::ShellAction::SHELL_ACTION_KILL => {
            let result = shell_management_result(&tasks, shell_runtime, action, &shell)?;
            let mut sequence = 0;
            send_terminal(
                &stream,
                &mut validator,
                &binding,
                &mut sequence,
                terminal::terminal_stream_frame::Frame::ShellResult(result),
            )
            .await?;
            send_terminal(
                &stream,
                &mut validator,
                &binding,
                &mut sequence,
                terminal::terminal_stream_frame::Frame::Outcome(terminal::TerminalOutcome {
                    outcome: Some(terminal::terminal_outcome::Outcome::Closed(
                        terminal::TerminalClosed::new(),
                    )),
                    ..Default::default()
                }),
            )
            .await?;
            stream.close().await
        }
        terminal::ShellAction::SHELL_ACTION_UNSPECIFIED => stream.reset().await,
    }
}

fn shell_management_result(
    tasks: &ProductionTaskContext,
    runtime: &ShellRuntime,
    action: terminal::ShellAction,
    selection: &terminal::ShellSelection,
) -> Result<terminal::ShellManagementResult, GuestSessionError> {
    match action {
        terminal::ShellAction::SHELL_ACTION_LIST => {
            let listed = runtime.list();
            if listed.error.is_some() {
                return Err(GuestSessionError::Service);
            }
            let handles = tasks
                .shell_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut sessions = Vec::new();
            for entry in listed.sessions {
                for (handle, _) in handles
                    .iter()
                    .filter(|(_, record)| record.name == entry.name)
                {
                    sessions.push(terminal::ShellSession {
                        shell_handle: handle.clone(),
                        state: EnumOrUnknown::new(if entry.attached {
                            terminal::ShellSessionState::SHELL_SESSION_STATE_ATTACHED
                        } else {
                            terminal::ShellSessionState::SHELL_SESSION_STATE_DETACHED
                        }),
                        is_default: entry.is_default,
                        ..Default::default()
                    });
                }
            }
            Ok(terminal::ShellManagementResult {
                action: EnumOrUnknown::new(action),
                sessions,
                ..Default::default()
            })
        }
        terminal::ShellAction::SHELL_ACTION_DETACH => {
            let owner = tasks
                .shell_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&selection.shell_handle)
                .cloned()
                .ok_or(GuestSessionError::Service)?;
            if let Some(attached) = tasks.attached.as_ref() {
                let _ = attached.cancel_exec(&owner.owner_key, &owner.runtime_id, &tasks.boot_id);
            }
            let response = runtime.close_attach(&selection.shell_handle);
            if response.error.is_some() {
                return Err(GuestSessionError::Service);
            }
            Ok(terminal::ShellManagementResult {
                action: EnumOrUnknown::new(action),
                affected_shell_handle: selection.shell_handle.clone(),
                applied: true,
                ..Default::default()
            })
        }
        terminal::ShellAction::SHELL_ACTION_KILL => {
            let record = tasks
                .shell_handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&selection.shell_handle)
                .ok_or(GuestSessionError::Service)?;
            if let Some(attached) = tasks.attached.as_ref() {
                let _ = attached.cancel_exec(&record.owner_key, &record.runtime_id, &tasks.boot_id);
            }
            let response = runtime.kill(record.name);
            if response.error.is_some() {
                return Err(GuestSessionError::Service);
            }
            Ok(terminal::ShellManagementResult {
                action: EnumOrUnknown::new(action),
                affected_shell_handle: selection.shell_handle.clone(),
                applied: true,
                ..Default::default()
            })
        }
        _ => Err(GuestSessionError::Service),
    }
}

async fn drive_file_transfer(
    request: guest::GuestFileTransferRequest,
    response: terminal::TerminalOpenResponse,
    binding: GuestStreamBinding,
    endpoint: ArtifactEndpoint,
    mut stream: GuestStream,
) -> Result<(), GuestSessionError> {
    let mut validator = FileTransferStreamValidator::new(&request, &response)
        .map_err(|_| GuestSessionError::Service)?;
    let first = receive_file_frame(&mut stream, &mut validator, &binding.cancellation).await?;
    if !matches!(
        first.frame,
        Some(guest::guest_file_transfer_frame::Frame::Start(_))
    ) {
        return stream.reset().await;
    }
    match endpoint.direction {
        guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_HOST_TO_GUEST => {
            let mut writer =
                AtomicArtifactWriter::prepare(&endpoint.path, &binding.resource_handle)?;
            receive_artifact(&request, &binding, &mut writer, &mut validator, &mut stream).await
        }
        guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_GUEST_TO_HOST => {
            let mut file = open_artifact(&endpoint.path, endpoint.direction)?;
            send_artifact(&request, &binding, &mut file, &mut validator, &mut stream).await
        }
        guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_UNSPECIFIED => {
            stream.reset().await
        }
    }
}

async fn receive_artifact(
    request: &guest::GuestFileTransferRequest,
    binding: &GuestStreamBinding,
    writer: &mut AtomicArtifactWriter,
    validator: &mut FileTransferStreamValidator,
    stream: &mut GuestStream,
) -> Result<(), GuestSessionError> {
    let mut digest = writer.copy_prefix(request.offset)?;
    writer
        .file
        .seek(SeekFrom::Start(request.offset))
        .map_err(|_| GuestSessionError::Service)?;
    let mut server_sequence = 0;
    send_file_frame(
        stream,
        validator,
        binding,
        &mut server_sequence,
        guest::guest_file_transfer_frame::Frame::Credit(guest::GuestFileTransferCredit {
            bytes: MAX_GUEST_FILE_CHUNK_BYTES as u32,
            next_offset: request.offset,
            ..Default::default()
        }),
    )
    .await?;
    loop {
        let frame = receive_file_frame(stream, validator, &binding.cancellation).await?;
        match frame.frame {
            Some(guest::guest_file_transfer_frame::Frame::Chunk(chunk)) => {
                writer
                    .file
                    .write_all(&chunk.data)
                    .map_err(|_| GuestSessionError::Service)?;
                digest.update(&chunk.data);
                if chunk.eof {
                    let actual = digest.finalize().to_vec();
                    if actual != chunk.final_digest
                        || (!request.expected_digest.is_empty()
                            && actual != request.expected_digest)
                    {
                        return stream.reset().await;
                    }
                    writer.commit(chunk.total_size)?;
                    send_file_frame(
                        stream,
                        validator,
                        binding,
                        &mut server_sequence,
                        guest::guest_file_transfer_frame::Frame::Complete(
                            guest::GuestFileTransferComplete {
                                total_size: chunk.total_size,
                                digest: actual,
                                ..Default::default()
                            },
                        ),
                    )
                    .await?;
                    return stream.close().await;
                }
                send_file_frame(
                    stream,
                    validator,
                    binding,
                    &mut server_sequence,
                    guest::guest_file_transfer_frame::Frame::Credit(
                        guest::GuestFileTransferCredit {
                            bytes: chunk.data.len() as u32,
                            next_offset: chunk.offset.saturating_add(chunk.data.len() as u64),
                            ..Default::default()
                        },
                    ),
                )
                .await?;
            }
            Some(guest::guest_file_transfer_frame::Frame::Cancel(_)) => {
                return stream.reset().await;
            }
            _ => return stream.reset().await,
        }
    }
}

async fn send_artifact(
    request: &guest::GuestFileTransferRequest,
    binding: &GuestStreamBinding,
    file: &mut File,
    validator: &mut FileTransferStreamValidator,
    stream: &mut GuestStream,
) -> Result<(), GuestSessionError> {
    let metadata = file.metadata().map_err(|_| GuestSessionError::Service)?;
    if !metadata.is_file() || metadata.len() != request.declared_size {
        return stream.reset().await;
    }
    let digest = hash_file(file)?;
    if !request.expected_digest.is_empty() && digest.as_slice() != request.expected_digest {
        return stream.reset().await;
    }
    file.seek(SeekFrom::Start(request.offset))
        .map_err(|_| GuestSessionError::Service)?;
    let mut offset = request.offset;
    let mut server_sequence = 0;
    let mut available_credit = 0_u64;
    loop {
        if available_credit == 0 {
            let frame = receive_file_frame(stream, validator, &binding.cancellation).await?;
            match frame.frame {
                Some(guest::guest_file_transfer_frame::Frame::Credit(credit)) => {
                    available_credit = u64::from(credit.bytes);
                }
                Some(guest::guest_file_transfer_frame::Frame::Cancel(_)) => {
                    return stream.reset().await;
                }
                _ => return stream.reset().await,
            }
        }
        let remaining = request.declared_size.saturating_sub(offset);
        let take = remaining
            .min(available_credit)
            .min(MAX_GUEST_FILE_CHUNK_BYTES as u64) as usize;
        let mut data = vec![0_u8; take];
        if take != 0 {
            file.read_exact(&mut data)
                .map_err(|_| GuestSessionError::Service)?;
        }
        let eof = offset.saturating_add(take as u64) == request.declared_size;
        send_file_frame(
            stream,
            validator,
            binding,
            &mut server_sequence,
            guest::guest_file_transfer_frame::Frame::Chunk(guest::GuestFileTransferChunk {
                offset,
                data,
                eof,
                total_size: request.declared_size,
                final_digest: if eof { digest.clone() } else { Vec::new() },
                ..Default::default()
            }),
        )
        .await?;
        offset = offset.saturating_add(take as u64);
        available_credit = available_credit.saturating_sub(take as u64);
        if eof {
            send_file_frame(
                stream,
                validator,
                binding,
                &mut server_sequence,
                guest::guest_file_transfer_frame::Frame::Complete(
                    guest::GuestFileTransferComplete {
                        total_size: request.declared_size,
                        digest,
                        ..Default::default()
                    },
                ),
            )
            .await?;
            return stream.close().await;
        }
    }
}

async fn receive_file_frame(
    stream: &mut GuestStream,
    validator: &mut FileTransferStreamValidator,
    cancellation: &Cancellation,
) -> Result<guest::GuestFileTransferFrame, GuestSessionError> {
    let input = tokio::select! {
        () = cancellation.cancelled() => return Err(GuestSessionError::Service),
        input = stream.receive() => input,
    };
    match input {
        GuestStreamInput::Message(bytes) => {
            let len = bytes.len();
            let frame = guest::GuestFileTransferFrame::parse_from_bytes(&bytes)
                .map_err(|_| GuestSessionError::Service)?;
            validator
                .accept(GuestStreamDirection::ClientToServer, &frame)
                .map_err(|_| GuestSessionError::Service)?;
            stream.consume(len).await?;
            Ok(frame)
        }
        GuestStreamInput::RemoteClosed
        | GuestStreamInput::Reset
        | GuestStreamInput::Disconnected => Err(GuestSessionError::Service),
    }
}

async fn send_file_frame(
    stream: &GuestStream,
    validator: &mut FileTransferStreamValidator,
    binding: &GuestStreamBinding,
    sequence: &mut u64,
    frame: guest::guest_file_transfer_frame::Frame,
) -> Result<(), GuestSessionError> {
    let message = guest::GuestFileTransferFrame {
        session_generation: binding.generation,
        request_id: binding.request_id.clone(),
        sequence: *sequence,
        operation_id: binding.operation_id.clone(),
        resource_handle: binding.resource_handle.clone(),
        frame: Some(frame),
        ..Default::default()
    };
    validator
        .accept(GuestStreamDirection::ServerToClient, &message)
        .map_err(|_| GuestSessionError::Service)?;
    stream.send(&message).await?;
    *sequence = sequence.checked_add(1).ok_or(GuestSessionError::Service)?;
    Ok(())
}

fn open_artifact(
    path: &Path,
    direction: guest::GuestFileTransferDirection,
) -> Result<File, GuestSessionError> {
    let flags = match direction {
        guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_HOST_TO_GUEST => {
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW
        }
        guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_GUEST_TO_HOST => {
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW
        }
        guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_UNSPECIFIED => {
            return Err(GuestSessionError::Service);
        }
    };
    let fd: OwnedFd = rustix::fs::open(path, flags, Mode::from_bits_truncate(0o600))
        .map_err(|_| GuestSessionError::Service)?;
    let file = File::from(fd);
    let metadata = file.metadata().map_err(|_| GuestSessionError::Service)?;
    if !metadata.is_file() {
        return Err(GuestSessionError::Service);
    }
    Ok(file)
}

fn hash_file(file: &mut File) -> Result<Vec<u8>, GuestSessionError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| GuestSessionError::Service)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| GuestSessionError::Service)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().to_vec())
}

async fn drive_security_key(
    request: guest::GuestSecurityKeyRequest,
    response: terminal::TerminalOpenResponse,
    binding: GuestStreamBinding,
    mut ceremony: Box<dyn SecurityKeyCeremony>,
    stream: GuestStream,
) -> Result<(), GuestSessionError> {
    let result =
        drive_security_key_inner(request, response, binding, ceremony.as_mut(), stream).await;
    if result.is_err() {
        ceremony.cancel().await;
    }
    result
}

async fn drive_security_key_inner(
    request: guest::GuestSecurityKeyRequest,
    response: terminal::TerminalOpenResponse,
    binding: GuestStreamBinding,
    ceremony: &mut dyn SecurityKeyCeremony,
    mut stream: GuestStream,
) -> Result<(), GuestSessionError> {
    let mut validator = SecurityKeyStreamValidator::new(&request, &response)
        .map_err(|_| GuestSessionError::Service)?;
    let open =
        receive_security_key_frame(&mut stream, &mut validator, &binding.cancellation).await?;
    if !matches!(
        open.frame,
        Some(guest::guest_security_key_frame::Frame::Open(_))
    ) {
        ceremony.cancel().await;
        return stream.reset().await;
    }
    let mut server_sequence = 0;
    if request.approval_required {
        send_security_key_frame(
                                    &stream,
                                    &mut validator,
                                    &binding,
                                    &mut server_sequence,
                                    guest::guest_security_key_frame::Frame::ApprovalRequest(
                                        guest::GuestSecurityKeyApprovalRequest {
                                            approval: EnumOrUnknown::new(
                                                guest::GuestSecurityKeyApprovalKind::GUEST_SECURITY_KEY_APPROVAL_KIND_USER_PRESENCE,
                                            ),
                                            ..Default::default()
                                        },
                                    ),
                                )
                                .await?;
        let approval =
            receive_security_key_frame(&mut stream, &mut validator, &binding.cancellation).await?;
        let approved = matches!(
            approval.frame,
            Some(guest::guest_security_key_frame::Frame::Approval(
                guest::GuestSecurityKeyApproval {
                    decision,
                    ..
                }
            )) if decision.enum_value().ok()
                == Some(
                    guest::GuestSecurityKeyApprovalDecision::GUEST_SECURITY_KEY_APPROVAL_DECISION_APPROVED
                )
        );
        if !approved {
            ceremony.cancel().await;
            send_security_key_complete(
                &stream,
                &mut validator,
                &binding,
                &mut server_sequence,
                guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_DENIED,
            )
            .await?;
            return stream.close().await;
        }
    }
    loop {
        let frame =
            receive_security_key_frame(&mut stream, &mut validator, &binding.cancellation).await?;
        match frame.frame {
            Some(guest::guest_security_key_frame::Frame::DeviceReport(report)) => {
                let device_report: [u8; 64] = report
                    .report
                    .try_into()
                    .map_err(|_| GuestSessionError::Service)?;
                let exchange = ceremony.exchange(device_report).await?;
                send_security_key_frame(
                    &stream,
                    &mut validator,
                    &binding,
                    &mut server_sequence,
                    guest::guest_security_key_frame::Frame::GuestReport(
                        guest::GuestSecurityKeyReport {
                            report: exchange.guest_report.to_vec(),
                            ..Default::default()
                        },
                    ),
                )
                .await?;
                let outcome = match exchange.outcome {
                    SecurityKeyExchangeOutcome::Continue => continue,
                    SecurityKeyExchangeOutcome::Succeeded => {
                        guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_SUCCEEDED
                    }
                    SecurityKeyExchangeOutcome::CtapError => {
                        guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_CTAP_ERROR
                    }
                };
                send_security_key_complete(
                    &stream,
                    &mut validator,
                    &binding,
                    &mut server_sequence,
                    outcome,
                )
                .await?;
                return stream.close().await;
            }
            Some(guest::guest_security_key_frame::Frame::Cancel(_)) => {
                ceremony.cancel().await;
                send_security_key_complete(
                    &stream,
                    &mut validator,
                    &binding,
                    &mut server_sequence,
                    guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_CANCELLED,
                )
                .await?;
                return stream.close().await;
            }
            _ => {
                ceremony.cancel().await;
                return stream.reset().await;
            }
        }
    }
}

async fn receive_security_key_frame(
    stream: &mut GuestStream,
    validator: &mut SecurityKeyStreamValidator,
    cancellation: &Cancellation,
) -> Result<guest::GuestSecurityKeyFrame, GuestSessionError> {
    let input = tokio::select! {
        () = cancellation.cancelled() => return Err(GuestSessionError::Service),
        input = stream.receive() => input,
    };
    match input {
        GuestStreamInput::Message(bytes) => {
            let len = bytes.len();
            let frame = guest::GuestSecurityKeyFrame::parse_from_bytes(&bytes)
                .map_err(|_| GuestSessionError::Service)?;
            validator
                .accept(GuestStreamDirection::ClientToServer, &frame)
                .map_err(|_| GuestSessionError::Service)?;
            stream.consume(len).await?;
            Ok(frame)
        }
        GuestStreamInput::RemoteClosed
        | GuestStreamInput::Reset
        | GuestStreamInput::Disconnected => Err(GuestSessionError::Service),
    }
}

async fn send_security_key_frame(
    stream: &GuestStream,
    validator: &mut SecurityKeyStreamValidator,
    binding: &GuestStreamBinding,
    sequence: &mut u64,
    frame: guest::guest_security_key_frame::Frame,
) -> Result<(), GuestSessionError> {
    let message = guest::GuestSecurityKeyFrame {
        session_generation: binding.generation,
        request_id: binding.request_id.clone(),
        sequence: *sequence,
        operation_id: binding.operation_id.clone(),
        resource_handle: binding.resource_handle.clone(),
        frame: Some(frame),
        ..Default::default()
    };
    validator
        .accept(GuestStreamDirection::ServerToClient, &message)
        .map_err(|_| GuestSessionError::Service)?;
    stream.send(&message).await?;
    *sequence = sequence.checked_add(1).ok_or(GuestSessionError::Service)?;
    Ok(())
}

async fn send_security_key_complete(
    stream: &GuestStream,
    validator: &mut SecurityKeyStreamValidator,
    binding: &GuestStreamBinding,
    sequence: &mut u64,
    outcome: guest::GuestSecurityKeyOutcome,
) -> Result<(), GuestSessionError> {
    send_security_key_frame(
        stream,
        validator,
        binding,
        sequence,
        guest::guest_security_key_frame::Frame::Complete(guest::GuestSecurityKeyComplete {
            outcome: EnumOrUnknown::new(outcome),
            ..Default::default()
        }),
    )
    .await
}

fn shutdown_failed(
    request: &guest::GuestShutdownRequest,
    kind: ErrorKind,
) -> guest::GuestShutdownResponse {
    let context = request.context.as_ref().expect("validated request");
    let metadata = context.metadata.as_ref().expect("validated request");
    guest::GuestShutdownResponse {
        outcome: EnumOrUnknown::new(Outcome::OUTCOME_FAILED),
        operation_id: context.operation_id.clone(),
        session_generation: metadata.session_generation,
        request_id: metadata.request_id.clone(),
        phase: EnumOrUnknown::new(guest::GuestShutdownPhase::GUEST_SHUTDOWN_PHASE_FINAL),
        final_outcome: EnumOrUnknown::new(
            guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_FAILED,
        ),
        error: MessageField::some(error_envelope(kind)),
        ..Default::default()
    }
}

fn shutdown_succeeded(
    request: &guest::GuestShutdownRequest,
    final_outcome: guest::GuestShutdownFinalOutcome,
) -> guest::GuestShutdownResponse {
    let context = request.context.as_ref().expect("validated request");
    let metadata = context.metadata.as_ref().expect("validated request");
    guest::GuestShutdownResponse {
        outcome: EnumOrUnknown::new(Outcome::OUTCOME_SUCCEEDED),
        operation_id: context.operation_id.clone(),
        session_generation: metadata.session_generation,
        request_id: metadata.request_id.clone(),
        phase: EnumOrUnknown::new(guest::GuestShutdownPhase::GUEST_SHUTDOWN_PHASE_FINAL),
        final_outcome: EnumOrUnknown::new(final_outcome),
        ..Default::default()
    }
}

fn error_envelope(kind: ErrorKind) -> common::ErrorEnvelope {
    common::ErrorEnvelope {
        kind: EnumOrUnknown::new(kind),
        retry: EnumOrUnknown::new(match kind {
            ErrorKind::ERROR_KIND_UNAVAILABLE | ErrorKind::ERROR_KIND_RESOURCE_EXHAUSTED => {
                RetryClass::RETRY_CLASS_AFTER_OBSERVATION
            }
            _ => RetryClass::RETRY_CLASS_NEVER,
        }),
        ..Default::default()
    }
}

fn terminal_signal_raw(signal: terminal::TerminalSignalKind) -> Result<u32, GuestSessionError> {
    match signal {
        terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_INTERRUPT => Ok(2),
        terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_TERMINATE => Ok(15),
        terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_SUSPEND => Ok(20),
        terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_HANGUP => Ok(1),
        terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_QUIT => Ok(3),
        terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_UNSPECIFIED => {
            Err(GuestSessionError::Service)
        }
    }
}

fn fixed_request_id(request_id: &[u8]) -> Result<[u8; 16], GuestSessionError> {
    request_id
        .try_into()
        .map_err(|_| GuestSessionError::Service)
}

fn digest_argv(argv: &[String]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b-guest-exec-argv-v2\0");
    for argument in argv {
        digest.update((argument.len() as u64).to_be_bytes());
        digest.update(argument.as_bytes());
    }
    digest.finalize().into()
}

fn decode_digest(value: &str) -> Result<Vec<u8>, GuestSessionError> {
    if value.len() != 64 {
        return Err(GuestSessionError::Service);
    }
    let mut bytes = Vec::with_capacity(32);
    for pair in value.as_bytes().chunks_exact(2) {
        let pair = std::str::from_utf8(pair).map_err(|_| GuestSessionError::Service)?;
        bytes.push(u8::from_str_radix(pair, 16).map_err(|_| GuestSessionError::Service)?);
    }
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn random_token() -> Result<String, GuestSessionError> {
    let mut bytes = [0_u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|_| GuestSessionError::IdentityUnavailable)?;
    if bytes == [0; 16] {
        return Err(GuestSessionError::IdentityUnavailable);
    }
    Ok(hex(&bytes))
}

fn read_boot_id() -> Result<String, GuestSessionError> {
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|_| GuestSessionError::Service)?;
    let boot_id = boot_id.trim();
    if boot_id.len() != 36
        || !boot_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err(GuestSessionError::Service);
    }
    Ok(boot_id.to_owned())
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn executable_ready(path: &Path) -> bool {
    path.is_absolute()
        && std::fs::symlink_metadata(path).is_ok_and(|metadata| {
            !metadata.file_type().is_symlink()
                && metadata.is_file()
                && metadata.permissions().mode() & 0o111 != 0
        })
}

fn directory_ready(path: &Path) -> bool {
    path.is_absolute()
        && std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| !metadata.file_type().is_symlink() && metadata.is_dir())
}

fn pam_login_ready() -> bool {
    std::fs::symlink_metadata("/etc/pam.d/login")
        .is_ok_and(|metadata| !metadata.file_type().is_symlink() && metadata.is_file())
}

fn classify_nonroot_uid(user: &str) -> Option<u32> {
    match classify_workload_user(user) {
        WorkloadUserUid::NonRoot(uid) => Some(uid),
        WorkloadUserUid::Root | WorkloadUserUid::Unresolved => None,
    }
}

fn unix_socket_ready(path: &Path, expected_uid: Option<u32>) -> bool {
    expected_uid.is_some_and(|uid| {
        path.is_absolute()
            && std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                !metadata.file_type().is_symlink()
                    && metadata.file_type().is_socket()
                    && metadata.uid() == uid
            })
    })
}

fn artifact_ready(path: &Path) -> bool {
    path.is_absolute()
        && path.parent().is_some_and(directory_ready)
        && std::fs::symlink_metadata(path).is_ok_and(|metadata| {
            !metadata.file_type().is_symlink()
                && metadata.is_file()
                && metadata.uid() == rustix::process::geteuid().as_raw()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use tokio::sync::Notify;

    const GENERATION: u64 = 9;
    const REQUEST_ID: [u8; 16] = [0x11; 16];

    fn context() -> guest::GuestOperationContext {
        guest::GuestOperationContext {
            metadata: MessageField::some(common::RequestMetadata {
                request_id: REQUEST_ID.to_vec(),
                idempotency_key: vec![0x22; 16],
                issued_at_unix_ms: 1_000,
                expires_at_unix_ms: 2_000,
                session_generation: GENERATION,
                ..Default::default()
            }),
            scope: MessageField::some(common::IdentityScope {
                realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
                workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
                ..Default::default()
            }),
            operation_id: "operation-1".to_owned(),
            request_digest: vec![0x33; 32],
            ..Default::default()
        }
    }

    fn response(handle: &str) -> terminal::TerminalOpenResponse {
        terminal::TerminalOpenResponse {
            outcome: EnumOrUnknown::new(Outcome::OUTCOME_ACCEPTED),
            operation_id: "operation-1".to_owned(),
            stream_id: "stream-256".to_owned(),
            session_generation: GENERATION,
            request_id: REQUEST_ID.to_vec(),
            resource_handle: handle.to_owned(),
            ..Default::default()
        }
    }

    struct FakeShutdown {
        calls: AtomicUsize,
        failures: AtomicUsize,
        block: bool,
        started: Notify,
        release: Notify,
    }

    #[async_trait]
    impl ShutdownBackend for FakeShutdown {
        fn ready(&self) -> bool {
            true
        }

        async fn request(
            &self,
            _: guest::GuestPowerAction,
            _: Duration,
        ) -> Result<(), GuestSessionError> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            self.started.notify_waiters();
            if self.block {
                self.release.notified().await;
            }
            if self
                .failures
                .fetch_update(
                    AtomicOrdering::SeqCst,
                    AtomicOrdering::SeqCst,
                    |remaining| remaining.checked_sub(1),
                )
                .is_ok()
            {
                Err(GuestSessionError::Service)
            } else {
                Ok(())
            }
        }
    }

    fn test_operations(shutdown: Arc<dyn ShutdownBackend>) -> ProductionGuestOperations {
        ProductionGuestOperations {
            workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
            capabilities: vec![guest::GuestCapability::GUEST_CAPABILITY_SHUTDOWN],
            tasks: ProductionTaskContext {
                attached: None,
                detached: None,
                boot_id: "boot-test".to_owned(),
                records: Arc::new(Mutex::new(BTreeMap::new())),
                adopted_cancel_sequences: Arc::new(Mutex::new(BTreeMap::new())),
                configured_launches: Arc::new(BTreeMap::new()),
                configured_launch_realm_id: None,
                configured_launch_workload_digest: None,
                shell: None,
                shell_handles: Arc::new(Mutex::new(BTreeMap::new())),
                artifacts: Arc::new(BTreeMap::new()),
                security_key: None,
            },
            shutdown: Some(shutdown),
            activation: ActivationRuntime::production("bbbbbbbbbbbbbbbbbbba".to_owned(), None),
            shutdown_inflight: Mutex::new(BTreeMap::new()),
            shutdown_success: Mutex::new(BTreeMap::new()),
        }
    }

    fn shutdown_request(operation_id: &str, deadline_unix_ms: u64) -> guest::GuestShutdownRequest {
        guest::GuestShutdownRequest {
            context: MessageField::some(guest::GuestOperationContext {
                metadata: MessageField::some(common::RequestMetadata {
                    request_id: REQUEST_ID.to_vec(),
                    idempotency_key: vec![0x22; 16],
                    issued_at_unix_ms: unix_time_ms(),
                    expires_at_unix_ms: deadline_unix_ms,
                    session_generation: GENERATION,
                    ..Default::default()
                }),
                scope: MessageField::some(common::IdentityScope {
                    realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
                    workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
                    ..Default::default()
                }),
                operation_id: operation_id.to_owned(),
                request_digest: vec![0x44; 32],
                ..Default::default()
            }),
            action: EnumOrUnknown::new(guest::GuestPowerAction::GUEST_POWER_ACTION_POWER_OFF),
            deadline_unix_ms,
            ..Default::default()
        }
    }
    fn file_frame(
        sequence: u64,
        frame: guest::guest_file_transfer_frame::Frame,
    ) -> guest::GuestFileTransferFrame {
        guest::GuestFileTransferFrame {
            session_generation: GENERATION,
            request_id: REQUEST_ID.to_vec(),
            sequence,
            operation_id: "operation-1".to_owned(),
            resource_handle: "artifact-1".to_owned(),
            frame: Some(frame),
            ..Default::default()
        }
    }

    #[test]
    fn file_transfer_requires_credit_and_continuous_final_digest() {
        let payload = b"payload";
        let digest = Sha256::digest(payload).to_vec();
        let request = guest::GuestFileTransferRequest {
            context: MessageField::some(context()),
            artifact: EnumOrUnknown::new(guest::GuestArtifactId::GUEST_ARTIFACT_ID_GUEST_CONFIG),
            configured_intent_id: "guest-config".to_owned(),
            direction: EnumOrUnknown::new(
                guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_HOST_TO_GUEST,
            ),
            declared_size: payload.len() as u64,
            expected_digest: digest.clone(),
            ..Default::default()
        };
        let mut validator =
            FileTransferStreamValidator::new(&request, &response("artifact-1")).unwrap();
        validator
            .accept(
                GuestStreamDirection::ClientToServer,
                &file_frame(
                    0,
                    guest::guest_file_transfer_frame::Frame::Start(guest::GuestFileTransferStart {
                        artifact: request.artifact,
                        configured_intent_id: request.configured_intent_id.clone(),
                        direction: request.direction,
                        declared_size: payload.len() as u64,
                        expected_digest: digest.clone(),
                        ..Default::default()
                    }),
                ),
            )
            .unwrap();
        validator
            .accept(
                GuestStreamDirection::ServerToClient,
                &file_frame(
                    0,
                    guest::guest_file_transfer_frame::Frame::Credit(
                        guest::GuestFileTransferCredit {
                            bytes: payload.len() as u32,
                            ..Default::default()
                        },
                    ),
                ),
            )
            .unwrap();
        validator
            .accept(
                GuestStreamDirection::ClientToServer,
                &file_frame(
                    1,
                    guest::guest_file_transfer_frame::Frame::Chunk(guest::GuestFileTransferChunk {
                        data: payload.to_vec(),
                        eof: true,
                        total_size: payload.len() as u64,
                        final_digest: digest.clone(),
                        ..Default::default()
                    }),
                ),
            )
            .unwrap();
        validator
            .accept(
                GuestStreamDirection::ServerToClient,
                &file_frame(
                    1,
                    guest::guest_file_transfer_frame::Frame::Complete(
                        guest::GuestFileTransferComplete {
                            total_size: payload.len() as u64,
                            digest,
                            ..Default::default()
                        },
                    ),
                ),
            )
            .unwrap();
        assert!(validator.is_terminal());
    }

    fn security_frame(
        sequence: u64,
        frame: guest::guest_security_key_frame::Frame,
    ) -> guest::GuestSecurityKeyFrame {
        guest::GuestSecurityKeyFrame {
            session_generation: GENERATION,
            request_id: REQUEST_ID.to_vec(),
            sequence,
            operation_id: "operation-1".to_owned(),
            resource_handle: "ceremony-1".to_owned(),
            frame: Some(frame),
            ..Default::default()
        }
    }

    #[test]
    fn security_key_approval_denial_is_terminal_and_redacted() {
        let request = guest::GuestSecurityKeyRequest {
            context: MessageField::some(context()),
            action: EnumOrUnknown::new(
                guest::GuestSecurityKeyAction::GUEST_SECURITY_KEY_ACTION_START,
            ),
            device_handle: "device-1".to_owned(),
            ceremony: EnumOrUnknown::new(
                guest::GuestSecurityKeyCeremonyKind::GUEST_SECURITY_KEY_CEREMONY_KIND_U2F,
            ),
            approval_required: true,
            ..Default::default()
        };
        let mut validator =
            SecurityKeyStreamValidator::new(&request, &response("ceremony-1")).unwrap();
        validator
            .accept(
                GuestStreamDirection::ClientToServer,
                &security_frame(
                    0,
                    guest::guest_security_key_frame::Frame::Open(guest::GuestSecurityKeyOpen {
                        action: request.action,
                        device_handle: request.device_handle.clone(),
                        ceremony_handle: "ceremony-1".to_owned(),
                        ceremony: request.ceremony,
                        ..Default::default()
                    }),
                ),
            )
            .unwrap();
        validator
            .accept(
                GuestStreamDirection::ServerToClient,
                &security_frame(
                    0,
                    guest::guest_security_key_frame::Frame::ApprovalRequest(
                        guest::GuestSecurityKeyApprovalRequest {
                            approval: EnumOrUnknown::new(
                                guest::GuestSecurityKeyApprovalKind::GUEST_SECURITY_KEY_APPROVAL_KIND_USER_PRESENCE,
                            ),
                            ..Default::default()
                        },
                    ),
                ),
            )
            .unwrap();
        validator
            .accept(
                GuestStreamDirection::ClientToServer,
                &security_frame(
                    1,
                    guest::guest_security_key_frame::Frame::Approval(
                        guest::GuestSecurityKeyApproval {
                            decision: EnumOrUnknown::new(
                                guest::GuestSecurityKeyApprovalDecision::GUEST_SECURITY_KEY_APPROVAL_DECISION_DENIED,
                            ),
                            ..Default::default()
                        },
                    ),
                ),
            )
            .unwrap();
        validator
            .accept(
                GuestStreamDirection::ServerToClient,
                &security_frame(
                    1,
                    guest::guest_security_key_frame::Frame::Complete(
                        guest::GuestSecurityKeyComplete {
                            outcome: EnumOrUnknown::new(
                                guest::GuestSecurityKeyOutcome::GUEST_SECURITY_KEY_OUTCOME_DENIED,
                            ),
                            ..Default::default()
                        },
                    ),
                ),
            )
            .unwrap();
        assert!(validator.is_terminal());
        let rendered = format!("{validator:?}");
        assert!(!rendered.contains("device-1"));
        assert!(!rendered.contains("ceremony-1"));
    }

    #[test]
    fn artifact_debug_never_reveals_configured_path() {
        let endpoint = ArtifactEndpoint {
            path: PathBuf::from("/run/secret-artifact"),
            direction:
                guest::GuestFileTransferDirection::GUEST_FILE_TRANSFER_DIRECTION_HOST_TO_GUEST,
        };
        let rendered = format!("{endpoint:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("secret-artifact"));
    }

    #[test]
    fn atomic_artifact_writer_rolls_back_and_replaces_in_same_directory() {
        let base = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target"))
            .join(format!("artifact-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let target = base.join("guest-config");
        std::fs::write(&target, b"live").unwrap();
        {
            let mut writer =
                AtomicArtifactWriter::prepare(&target, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
            writer.file.write_all(b"candidate").unwrap();
        }
        assert_eq!(std::fs::read(&target).unwrap(), b"live");
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 1);
        {
            let mut writer =
                AtomicArtifactWriter::prepare(&target, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
            writer.file.write_all(b"committed").unwrap();
            writer.commit(9).unwrap();
        }
        assert_eq!(std::fs::read(&target).unwrap(), b"committed");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn readiness_rejects_syntax_only_paths_and_missing_artifacts() {
        assert!(!executable_ready(Path::new("/definitely/missing/d2b")));
        assert!(!directory_ready(Path::new("/definitely/missing")));
        assert!(!artifact_ready(Path::new("/definitely/missing/artifact")));
        assert!(!unix_socket_ready(
            Path::new("/definitely/missing/socket"),
            Some(1000)
        ));
    }

    #[test]
    fn output_eof_is_observed_once_per_stream() {
        let mut eof = OutputEofState::new(false);
        assert!(eof.should_poll(ExecStream::Stdout));
        assert!(eof.should_poll(ExecStream::Stderr));
        eof.observe(ExecStream::Stdout);
        assert!(!eof.should_poll(ExecStream::Stdout));
        assert!(eof.should_poll(ExecStream::Stderr));
        eof.observe(ExecStream::Stdout);
        assert!(!eof.complete());
        eof.observe(ExecStream::Stderr);
        assert!(eof.complete());
        assert!(!eof.should_poll(ExecStream::Stderr));
        assert_eq!(
            terminal_signal_raw(terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_QUIT).unwrap(),
            3
        );
        assert_eq!(
            terminal_signal_raw(terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_SUSPEND)
                .unwrap(),
            20
        );
    }

    #[tokio::test]
    async fn shutdown_failure_and_expiry_remain_retryable() {
        let backend = Arc::new(FakeShutdown {
            calls: AtomicUsize::new(0),
            failures: AtomicUsize::new(1),
            block: false,
            started: Notify::new(),
            release: Notify::new(),
        });
        let operations = test_operations(backend.clone());
        let failed = operations
            .shutdown(shutdown_request("shutdown-failure", unix_time_ms() + 1_000))
            .await
            .unwrap();
        assert_eq!(
            failed.final_outcome.enum_value_or_default(),
            guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_FAILED
        );
        let retried = operations
            .shutdown(shutdown_request("shutdown-failure", unix_time_ms() + 1_000))
            .await
            .unwrap();
        assert_eq!(
            retried.final_outcome.enum_value_or_default(),
            guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_COMPLETED
        );
        let expired = operations
            .shutdown(shutdown_request("shutdown-expired", unix_time_ms()))
            .await
            .unwrap();
        assert_eq!(
            expired.final_outcome.enum_value_or_default(),
            guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_FAILED
        );
        let expired_retry = operations
            .shutdown(shutdown_request("shutdown-expired", unix_time_ms() + 1_000))
            .await
            .unwrap();
        assert_eq!(
            expired_retry.final_outcome.enum_value_or_default(),
            guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_COMPLETED
        );
        assert_eq!(backend.calls.load(AtomicOrdering::SeqCst), 3);
    }

    #[tokio::test]
    async fn concurrent_shutdown_duplicate_waits_for_the_same_completion() {
        let backend = Arc::new(FakeShutdown {
            calls: AtomicUsize::new(0),
            failures: AtomicUsize::new(0),
            block: true,
            started: Notify::new(),
            release: Notify::new(),
        });
        let operations = Arc::new(test_operations(backend.clone()));
        let request = shutdown_request("shutdown-concurrent", unix_time_ms() + 5_000);
        let first_operations = Arc::clone(&operations);
        let first_request = request.clone();
        let first =
            tokio::spawn(async move { first_operations.shutdown(first_request).await.unwrap() });
        backend.started.notified().await;
        let second_operations = Arc::clone(&operations);
        let second =
            tokio::spawn(async move { second_operations.shutdown(request).await.unwrap() });
        tokio::task::yield_now().await;
        backend.release.notify_waiters();
        let first = first.await.unwrap();
        let second = second.await.unwrap();
        for response in [first, second] {
            assert_eq!(
                response.final_outcome.enum_value_or_default(),
                guest::GuestShutdownFinalOutcome::GUEST_SHUTDOWN_FINAL_OUTCOME_COMPLETED
            );
        }
        assert_eq!(backend.calls.load(AtomicOrdering::SeqCst), 1);
    }
}
