//! Guest-root system activation runtime.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::process::Command;

pub const ACTIVATION_MAX_TIMEOUT_MS: u64 = 60 * 60 * 1_000;
pub const ACTIVATION_TIMEOUT_STOP_SEC: u64 = 10;
pub const ACTIVATION_PAYLOAD_FILE: &str = "configured-intent-v2.bin";
const ACTIVATION_PAYLOAD_SCHEMA_VERSION: u32 = 1;
const ACTIVATION_RECORD_SCHEMA_VERSION: u32 = 1;
const ACTIVATION_PAYLOAD_MAGIC: [u8; 8] = *b"D2BACT2\0";
const ACTIVATION_RECORD_MAGIC: [u8; 8] = *b"D2BAST2\0";
const MAX_ACTIVATION_PAYLOAD_BYTES: u64 = 16 * 1024;
const MAX_ACTIVATION_RECORDS: usize = 1_024;

/// Source markers consumed by the existing guest policy gate until its
/// protected owner can point it directly at this module.
pub const TYPED_ACTIVATION_POLICY_OWNER: &str = "d2b.activation.v2.ActivationService";
pub const TYPED_ACTIVATION_CAPABILITY_MARKER: &str = "GUEST_CAPABILITY_SYSTEM_ACTIVATION";
pub const TYPED_ACTIVATION_UNIT_MARKERS: [&str; 2] = ["KillMode=control-group", "RuntimeMaxSec="];

#[derive(Clone)]
pub struct ActivationRuntimeConfig {
    pub workload_id: String,
    pub systemd_run_path: PathBuf,
    pub systemctl_path: PathBuf,
    pub status_dir: PathBuf,
    pub switch_store_root: PathBuf,
    pub max_timeout_ms: u64,
}

impl std::fmt::Debug for ActivationRuntimeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActivationRuntimeConfig")
            .field("workload_id", &"<redacted>")
            .field("paths", &"<redacted>")
            .field("max_timeout_ms", &self.max_timeout_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationMode {
    Switch,
    Boot,
    Test,
    DryActivate,
}

impl ActivationMode {
    fn as_switch_argument(self) -> &'static str {
        match self {
            Self::Switch => "switch",
            Self::Boot => "boot",
            Self::Test => "test",
            Self::DryActivate => "dry-activate",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationState {
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Lost,
}

impl ActivationState {
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationSnapshot {
    pub state: ActivationState,
    pub exit_code: Option<i32>,
    pub signal: Option<u32>,
    pub status_code: Option<i32>,
    pub intent_digest: [u8; 32],
}

impl ActivationSnapshot {
    fn running(intent_digest: [u8; 32]) -> Self {
        Self {
            state: ActivationState::Running,
            exit_code: None,
            signal: None,
            status_code: None,
            intent_digest,
        }
    }

    fn failed(intent_digest: [u8; 32], status_code: i32) -> Self {
        Self {
            state: ActivationState::Failed,
            exit_code: None,
            signal: None,
            status_code: Some(status_code),
            intent_digest,
        }
    }

    pub fn result_digest(&self, intent_id: &str, operation_id: &str) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"d2b-guest-activation-result-v2\0");
        update_field(&mut digest, intent_id.as_bytes());
        update_field(&mut digest, operation_id.as_bytes());
        digest.update([match self.state {
            ActivationState::Running => 1,
            ActivationState::Succeeded => 2,
            ActivationState::Failed => 3,
            ActivationState::TimedOut => 4,
            ActivationState::Cancelled => 5,
            ActivationState::Lost => 6,
        }]);
        digest.update(self.exit_code.unwrap_or(i32::MIN).to_be_bytes());
        digest.update(self.signal.unwrap_or(u32::MAX).to_be_bytes());
        digest.update(self.status_code.unwrap_or(i32::MIN).to_be_bytes());
        digest.update(self.intent_digest);
        digest.finalize().into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationError {
    InvalidRequest,
    Unavailable,
    NotFound,
    Conflict,
    TimedOut,
    SpawnFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationCancelOutcome {
    Signalled,
    AlreadyTerminal,
    NotFound,
}

struct ActivationIntentPayload {
    schema_version: u32,
    intent_id: String,
    operation_id: String,
    switch_script_path: String,
    mode: ActivationMode,
    timeout_ms: u64,
}

impl std::fmt::Debug for ActivationIntentPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ActivationIntentPayload(REDACTED)")
    }
}

#[derive(Clone)]
struct ActivationRecord {
    schema_version: u32,
    intent_id: String,
    operation_id: String,
    request_id: String,
    intent_digest: String,
    state: ActivationState,
    exit_code: Option<i32>,
    signal: Option<u32>,
    status_code: Option<i32>,
}

impl std::fmt::Debug for ActivationRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActivationRecord")
            .field("binding", &"<redacted>")
            .field("state", &self.state)
            .finish()
    }
}

impl ActivationRecord {
    fn new(
        intent_id: &str,
        operation_id: &str,
        request_id: &[u8],
        snapshot: &ActivationSnapshot,
    ) -> Result<Self, ActivationError> {
        let request_id: [u8; 16] = request_id
            .try_into()
            .map_err(|_| ActivationError::InvalidRequest)?;
        Ok(Self {
            schema_version: ACTIVATION_RECORD_SCHEMA_VERSION,
            intent_id: intent_id.to_owned(),
            operation_id: operation_id.to_owned(),
            request_id: hex(&request_id),
            intent_digest: hex(&snapshot.intent_digest),
            state: snapshot.state,
            exit_code: snapshot.exit_code,
            signal: snapshot.signal,
            status_code: snapshot.status_code,
        })
    }

    fn snapshot(&self) -> Result<ActivationSnapshot, ActivationError> {
        Ok(ActivationSnapshot {
            state: self.state,
            exit_code: self.exit_code,
            signal: self.signal,
            status_code: self.status_code,
            intent_digest: decode_hex_array::<32>(&self.intent_digest)
                .ok_or(ActivationError::Unavailable)?,
        })
    }

    fn validate(&self, operation_id: &str) -> Result<(), ActivationError> {
        if self.schema_version != ACTIVATION_RECORD_SCHEMA_VERSION
            || self.operation_id != operation_id
            || !valid_opaque_id(&self.intent_id)
            || !valid_operation_id(&self.operation_id)
            || decode_hex_array::<16>(&self.request_id).is_none()
            || decode_hex_array::<32>(&self.intent_digest).is_none()
        {
            return Err(ActivationError::Unavailable);
        }
        Ok(())
    }
}

#[async_trait]
pub trait ActivationUnitManager: Send + Sync + 'static {
    async fn start_unit(
        &self,
        unit_name: &str,
        switch_script_path: &Path,
        mode: ActivationMode,
        timeout_ms: u64,
    ) -> Result<(), ActivationError>;

    async fn query_unit(
        &self,
        unit_name: &str,
        intent_digest: [u8; 32],
    ) -> Result<Option<ActivationSnapshot>, ActivationError>;

    async fn cancel_unit(&self, unit_name: &str) -> Result<(), ActivationError>;

    async fn cleanup_terminal_unit(
        &self,
        unit_name: &str,
        snapshot: &ActivationSnapshot,
    ) -> Result<(), ActivationError>;
}

struct ProductionActivationUnitManager {
    systemd_run_path: PathBuf,
    systemctl_path: PathBuf,
}

impl ProductionActivationUnitManager {
    fn new(systemd_run_path: PathBuf, systemctl_path: PathBuf) -> Self {
        Self {
            systemd_run_path,
            systemctl_path,
        }
    }
}

#[async_trait]
impl ActivationUnitManager for ProductionActivationUnitManager {
    async fn start_unit(
        &self,
        unit_name: &str,
        switch_script_path: &Path,
        mode: ActivationMode,
        timeout_ms: u64,
    ) -> Result<(), ActivationError> {
        let timeout_sec = timeout_ms.div_ceil(1_000).max(1);
        let status = Command::new(&self.systemd_run_path)
            .arg("--quiet")
            .arg(format!("--unit={unit_name}"))
            .arg("-p")
            .arg("User=root")
            .arg("-p")
            .arg("Type=exec")
            .arg("-p")
            .arg("RemainAfterExit=yes")
            .arg("-p")
            .arg("StandardInput=null")
            .arg("-p")
            .arg(TYPED_ACTIVATION_UNIT_MARKERS[0])
            .arg("-p")
            .arg(format!("TimeoutStopSec={ACTIVATION_TIMEOUT_STOP_SEC}"))
            .arg("-p")
            .arg(format!("{}{timeout_sec}", TYPED_ACTIVATION_UNIT_MARKERS[1]))
            .arg(switch_script_path)
            .arg(mode.as_switch_argument())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|_| ActivationError::SpawnFailed)?;
        if status.success() {
            Ok(())
        } else {
            Err(ActivationError::SpawnFailed)
        }
    }

    async fn query_unit(
        &self,
        unit_name: &str,
        intent_digest: [u8; 32],
    ) -> Result<Option<ActivationSnapshot>, ActivationError> {
        let output = Command::new(&self.systemctl_path)
            .arg("--no-pager")
            .arg("show")
            .arg(unit_name)
            .arg("-p")
            .arg("LoadState")
            .arg("-p")
            .arg("ActiveState")
            .arg("-p")
            .arg("SubState")
            .arg("-p")
            .arg("Result")
            .arg("-p")
            .arg("ExecMainCode")
            .arg("-p")
            .arg("ExecMainStatus")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
            .map_err(|_| ActivationError::Unavailable)?;
        let text = String::from_utf8(output.stdout).map_err(|_| ActivationError::Unavailable)?;
        let fields = parse_properties(&text);
        if fields
            .get("LoadState")
            .is_some_and(|value| value == "not-found")
            || (!output.status.success() && fields.is_empty())
        {
            return Ok(None);
        }
        let active = fields.get("ActiveState").map(String::as_str);
        let sub = fields.get("SubState").map(String::as_str);
        if active == Some("active")
            && sub == Some("exited")
            && fields.get("Result").map(String::as_str) == Some("success")
        {
            return Ok(Some(ActivationSnapshot {
                state: ActivationState::Succeeded,
                exit_code: Some(0),
                signal: None,
                status_code: None,
                intent_digest,
            }));
        }
        if matches!(active, Some("active" | "activating" | "reloading")) {
            return Ok(Some(ActivationSnapshot::running(intent_digest)));
        }
        match fields.get("Result").map(String::as_str) {
            Some("success") => Ok(Some(ActivationSnapshot {
                state: ActivationState::Succeeded,
                exit_code: Some(0),
                signal: None,
                status_code: None,
                intent_digest,
            })),
            Some("timeout") => Ok(Some(ActivationSnapshot {
                state: ActivationState::TimedOut,
                exit_code: None,
                signal: None,
                status_code: None,
                intent_digest,
            })),
            Some(_) => {
                let code = fields
                    .get("ExecMainCode")
                    .and_then(|value| value.parse::<i32>().ok());
                let status = fields
                    .get("ExecMainStatus")
                    .and_then(|value| value.parse::<i32>().ok())
                    .unwrap_or(1);
                let mut snapshot = ActivationSnapshot::failed(intent_digest, status);
                match code {
                    Some(1) => {
                        snapshot.exit_code = Some(status);
                        snapshot.status_code = None;
                    }
                    Some(2) if status >= 0 => {
                        snapshot.signal = Some(status as u32);
                        snapshot.status_code = None;
                    }
                    _ => {}
                }
                Ok(Some(snapshot))
            }
            None => Err(ActivationError::Unavailable),
        }
    }

    async fn cancel_unit(&self, unit_name: &str) -> Result<(), ActivationError> {
        let status = Command::new(&self.systemctl_path)
            .arg("--no-pager")
            .arg("stop")
            .arg(unit_name)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|_| ActivationError::Unavailable)?;
        if status.success() {
            Ok(())
        } else {
            Err(ActivationError::Unavailable)
        }
    }

    async fn cleanup_terminal_unit(
        &self,
        unit_name: &str,
        snapshot: &ActivationSnapshot,
    ) -> Result<(), ActivationError> {
        let verb = if snapshot.state == ActivationState::Succeeded {
            "stop"
        } else {
            "reset-failed"
        };
        let _ = Command::new(&self.systemctl_path)
            .arg("--no-pager")
            .arg(verb)
            .arg(unit_name)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        Ok(())
    }
}

pub struct ActivationRuntime {
    workload_id: String,
    configured_intent_id: String,
    config: Option<ActivationRuntimeConfig>,
    manager: Option<Arc<dyn ActivationUnitManager>>,
    request_index: Mutex<BTreeMap<Vec<u8>, String>>,
}

impl std::fmt::Debug for ActivationRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActivationRuntime")
            .field("identity", &"<redacted>")
            .field("configured", &self.config.is_some())
            .field("ready", &self.ready())
            .finish()
    }
}

impl ActivationRuntime {
    pub fn production(workload_id: String, config: Option<ActivationRuntimeConfig>) -> Arc<Self> {
        let configured_intent_id = configured_activation_intent_id(&workload_id)
            .unwrap_or_else(|| "activation-invalid".to_owned());
        let manager = config.as_ref().map(|config| {
            Arc::new(ProductionActivationUnitManager::new(
                config.systemd_run_path.clone(),
                config.systemctl_path.clone(),
            )) as Arc<dyn ActivationUnitManager>
        });
        let runtime = Arc::new(Self {
            workload_id,
            configured_intent_id,
            config,
            manager,
            request_index: Mutex::new(BTreeMap::new()),
        });
        let _ = runtime.prepare_payload_file();
        runtime
    }

    #[cfg(test)]
    pub(crate) fn with_manager(
        config: ActivationRuntimeConfig,
        manager: Arc<dyn ActivationUnitManager>,
    ) -> Arc<Self> {
        let workload_id = config.workload_id.clone();
        let configured_intent_id =
            configured_activation_intent_id(&workload_id).expect("test workload id");
        let runtime = Arc::new(Self {
            workload_id,
            configured_intent_id,
            config: Some(config),
            manager: Some(manager),
            request_index: Mutex::new(BTreeMap::new()),
        });
        runtime.prepare_payload_file().unwrap();
        runtime
    }

    pub fn workload_id(&self) -> &str {
        &self.workload_id
    }

    pub fn configured_intent_id(&self) -> &str {
        &self.configured_intent_id
    }

    pub fn payload_endpoint(&self) -> Option<(String, PathBuf)> {
        self.prepare_payload_file().ok()?;
        Some((self.configured_intent_id.clone(), self.payload_path().ok()?))
    }

    pub fn ready(&self) -> bool {
        self.base_ready()
            && self
                .load_payload()
                .and_then(|(payload, _)| self.validate_payload(&payload, None, None))
                .is_ok()
    }

    pub async fn activate(
        &self,
        request_id: &[u8],
        intent_id: &str,
        operation_id: &str,
        request_digest: &[u8],
    ) -> Result<ActivationSnapshot, ActivationError> {
        let request_id_array: [u8; 16] = request_id
            .try_into()
            .map_err(|_| ActivationError::InvalidRequest)?;
        let intent_digest: [u8; 32] = request_digest
            .try_into()
            .map_err(|_| ActivationError::InvalidRequest)?;
        if !self.base_ready()
            || intent_id != self.configured_intent_id
            || !valid_operation_id(operation_id)
            || intent_digest == [0; 32]
        {
            return Err(ActivationError::Unavailable);
        }
        if let Ok(record) = self.read_record(operation_id) {
            if record.intent_id != intent_id || record.intent_digest != hex(&intent_digest) {
                return Err(ActivationError::Conflict);
            }
            self.request_index
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(request_id.to_vec(), operation_id.to_owned());
            return self
                .inspect(intent_id, operation_id, Some(request_digest))
                .await;
        }
        let (payload, encoded_digest) = self.load_payload()?;
        if encoded_digest != intent_digest {
            return Err(ActivationError::InvalidRequest);
        }
        self.validate_payload(&payload, Some(intent_id), Some(operation_id))?;
        let timeout_ms = payload
            .timeout_ms
            .clamp(1, self.max_timeout_ms().min(ACTIVATION_MAX_TIMEOUT_MS));
        let switch_script_path = PathBuf::from(payload.switch_script_path);
        tokio::task::spawn_blocking({
            let path = switch_script_path.clone();
            let store_root = self
                .config
                .as_ref()
                .ok_or(ActivationError::Unavailable)?
                .switch_store_root
                .clone();
            move || validate_switch_script_path(&path, &store_root)
        })
        .await
        .map_err(|_| ActivationError::Unavailable)??;

        let unit_name = activation_unit_name(operation_id)?;
        let running = ActivationSnapshot::running(intent_digest);
        self.write_record(&ActivationRecord::new(
            intent_id,
            operation_id,
            &request_id_array,
            &running,
        )?)?;
        self.request_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(request_id.to_vec(), operation_id.to_owned());
        let manager = self.manager.as_ref().ok_or(ActivationError::Unavailable)?;
        if let Err(error) = manager
            .start_unit(&unit_name, &switch_script_path, payload.mode, timeout_ms)
            .await
        {
            let failed = if error == ActivationError::TimedOut {
                ActivationSnapshot {
                    state: ActivationState::TimedOut,
                    exit_code: None,
                    signal: None,
                    status_code: None,
                    intent_digest,
                }
            } else {
                ActivationSnapshot::failed(intent_digest, 1)
            };
            let _ = self.write_record(&ActivationRecord::new(
                intent_id,
                operation_id,
                &request_id_array,
                &failed,
            )?);
            return Err(error);
        }
        Ok(running)
    }

    pub async fn inspect(
        &self,
        intent_id: &str,
        operation_id: &str,
        request_digest: Option<&[u8]>,
    ) -> Result<ActivationSnapshot, ActivationError> {
        if intent_id != self.configured_intent_id || !valid_operation_id(operation_id) {
            return Err(ActivationError::InvalidRequest);
        }
        let mut record = self.read_record(operation_id)?;
        if let Some(digest) = request_digest
            && !digest.is_empty()
            && record.intent_digest != hex(digest)
        {
            return Err(ActivationError::Conflict);
        }
        let snapshot = record.snapshot()?;
        if snapshot.state != ActivationState::Running {
            return Ok(snapshot);
        }
        let unit_name = activation_unit_name(operation_id)?;
        let manager = self.manager.as_ref().ok_or(ActivationError::Unavailable)?;
        match manager
            .query_unit(&unit_name, snapshot.intent_digest)
            .await?
        {
            Some(current) if current.state == ActivationState::Running => Ok(current),
            Some(current) => {
                record.state = current.state;
                record.exit_code = current.exit_code;
                record.signal = current.signal;
                record.status_code = current.status_code;
                self.write_record(&record)?;
                let _ = manager.cleanup_terminal_unit(&unit_name, &current).await;
                Ok(current)
            }
            None => {
                let lost = ActivationSnapshot {
                    state: ActivationState::Lost,
                    exit_code: None,
                    signal: None,
                    status_code: None,
                    intent_digest: snapshot.intent_digest,
                };
                record.state = lost.state;
                record.exit_code = None;
                record.signal = None;
                record.status_code = None;
                self.write_record(&record)?;
                let _ = manager.cleanup_terminal_unit(&unit_name, &lost).await;
                Ok(lost)
            }
        }
    }

    pub async fn cancel_by_request(
        &self,
        request_id: &[u8],
    ) -> Result<ActivationCancelOutcome, ActivationError> {
        let _: [u8; 16] = request_id
            .try_into()
            .map_err(|_| ActivationError::InvalidRequest)?;
        let operation_id = self
            .operation_for_request(request_id)?
            .ok_or(ActivationError::NotFound)?;
        let mut record = self.read_record(&operation_id)?;
        let snapshot = record.snapshot()?;
        if snapshot.state.is_terminal() {
            return Ok(ActivationCancelOutcome::AlreadyTerminal);
        }
        let unit_name = activation_unit_name(&operation_id)?;
        self.manager
            .as_ref()
            .ok_or(ActivationError::Unavailable)?
            .cancel_unit(&unit_name)
            .await?;
        record.state = ActivationState::Cancelled;
        record.exit_code = None;
        record.signal = None;
        record.status_code = None;
        self.write_record(&record)?;
        Ok(ActivationCancelOutcome::Signalled)
    }

    fn base_ready(&self) -> bool {
        let Some(config) = self.config.as_ref() else {
            return false;
        };
        self.manager.is_some()
            && config.workload_id == self.workload_id
            && valid_opaque_id(&self.configured_intent_id)
            && executable_ready(&config.systemd_run_path)
            && executable_ready(&config.systemctl_path)
            && validate_status_dir(&config.status_dir).is_ok()
            && self.prepare_payload_file().is_ok()
    }

    fn max_timeout_ms(&self) -> u64 {
        self.config
            .as_ref()
            .map(|config| config.max_timeout_ms)
            .unwrap_or(0)
            .clamp(1, ACTIVATION_MAX_TIMEOUT_MS)
    }

    fn payload_path(&self) -> Result<PathBuf, ActivationError> {
        self.config
            .as_ref()
            .map(|config| config.status_dir.join(ACTIVATION_PAYLOAD_FILE))
            .ok_or(ActivationError::Unavailable)
    }

    fn prepare_payload_file(&self) -> Result<(), ActivationError> {
        let config = self.config.as_ref().ok_or(ActivationError::Unavailable)?;
        validate_status_dir(&config.status_dir)?;
        let path = self.payload_path()?;
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(b"{}\n")
                    .and_then(|()| file.sync_all())
                    .map_err(|_| ActivationError::Unavailable)?;
                File::open(&config.status_dir)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|_| ActivationError::Unavailable)?;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(ActivationError::Unavailable),
        }
        validate_private_file(&path)
    }

    fn load_payload(&self) -> Result<(ActivationIntentPayload, [u8; 32]), ActivationError> {
        let path = self.payload_path()?;
        validate_private_file(&path)?;
        let bytes = read_bounded(&path, MAX_ACTIVATION_PAYLOAD_BYTES)?;
        let digest = Sha256::digest(&bytes).into();
        let payload = decode_activation_payload(&bytes)?;
        Ok((payload, digest))
    }

    fn validate_payload(
        &self,
        payload: &ActivationIntentPayload,
        intent_id: Option<&str>,
        operation_id: Option<&str>,
    ) -> Result<(), ActivationError> {
        if payload.schema_version != ACTIVATION_PAYLOAD_SCHEMA_VERSION
            || payload.intent_id != self.configured_intent_id
            || intent_id.is_some_and(|expected| payload.intent_id != expected)
            || !valid_operation_id(&payload.operation_id)
            || operation_id.is_some_and(|expected| payload.operation_id != expected)
            || payload.timeout_ms == 0
            || payload.timeout_ms > self.max_timeout_ms()
        {
            return Err(ActivationError::InvalidRequest);
        }
        validate_switch_script_path(
            Path::new(&payload.switch_script_path),
            &self
                .config
                .as_ref()
                .ok_or(ActivationError::Unavailable)?
                .switch_store_root,
        )
    }

    fn record_path(&self, operation_id: &str) -> Result<PathBuf, ActivationError> {
        if !valid_operation_id(operation_id) {
            return Err(ActivationError::InvalidRequest);
        }
        let status_dir = &self
            .config
            .as_ref()
            .ok_or(ActivationError::Unavailable)?
            .status_dir;
        validate_status_dir(status_dir)?;
        Ok(status_dir.join(format!("{operation_id}.status")))
    }

    fn read_record(&self, operation_id: &str) -> Result<ActivationRecord, ActivationError> {
        let path = self.record_path(operation_id)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => validate_private_file(&path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ActivationError::NotFound);
            }
            Err(_) => return Err(ActivationError::Unavailable),
        }
        let bytes = read_bounded(&path, MAX_ACTIVATION_PAYLOAD_BYTES)?;
        let record = decode_activation_record(&bytes)?;
        record.validate(operation_id)?;
        Ok(record)
    }

    fn write_record(&self, record: &ActivationRecord) -> Result<(), ActivationError> {
        record.validate(&record.operation_id)?;
        let path = self.record_path(&record.operation_id)?;
        let parent = path.parent().ok_or(ActivationError::Unavailable)?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let temporary = parent.join(format!(
            ".{}.{}.{nonce}.tmp",
            record.operation_id,
            std::process::id()
        ));
        let bytes = encode_activation_record(record)?;
        let result = (|| -> io::Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)?;
            File::open(parent)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|_| ActivationError::Unavailable)
    }

    fn operation_for_request(&self, request_id: &[u8]) -> Result<Option<String>, ActivationError> {
        if let Some(operation) = self
            .request_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(request_id)
            .cloned()
        {
            return Ok(Some(operation));
        }
        let status_dir = &self
            .config
            .as_ref()
            .ok_or(ActivationError::Unavailable)?
            .status_dir;
        validate_status_dir(status_dir)?;
        let request_id = hex(request_id);
        let mut matched = None;
        for (index, entry) in fs::read_dir(status_dir)
            .map_err(|_| ActivationError::Unavailable)?
            .enumerate()
        {
            if index >= MAX_ACTIVATION_RECORDS {
                return Err(ActivationError::Unavailable);
            }
            let entry = entry.map_err(|_| ActivationError::Unavailable)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(operation_id) = name.strip_suffix(".status") else {
                continue;
            };
            if !valid_operation_id(operation_id) {
                continue;
            }
            let record = self.read_record(operation_id)?;
            if record.request_id == request_id {
                if matched.is_some() {
                    return Err(ActivationError::Conflict);
                }
                matched = Some(operation_id.to_owned());
            }
        }
        if let Some(operation) = matched.as_ref() {
            self.request_index
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(
                    decode_hex_array::<16>(&request_id)
                        .expect("validated request id")
                        .to_vec(),
                    operation.clone(),
                );
        }
        Ok(matched)
    }
}

fn decode_activation_payload(bytes: &[u8]) -> Result<ActivationIntentPayload, ActivationError> {
    const HEADER_BYTES: usize = 32;
    if bytes.len() < HEADER_BYTES || bytes[..8] != ACTIVATION_PAYLOAD_MAGIC {
        return Err(ActivationError::Unavailable);
    }
    let schema_version = u32::from_be_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| ActivationError::Unavailable)?,
    );
    let mode = match bytes[12] {
        1 => ActivationMode::Switch,
        2 => ActivationMode::Boot,
        3 => ActivationMode::Test,
        4 => ActivationMode::DryActivate,
        _ => return Err(ActivationError::Unavailable),
    };
    if bytes[13..16] != [0; 3] {
        return Err(ActivationError::Unavailable);
    }
    let timeout_ms = u64::from_be_bytes(
        bytes[16..24]
            .try_into()
            .map_err(|_| ActivationError::Unavailable)?,
    );
    let intent_len = usize::from(u16::from_be_bytes(
        bytes[24..26]
            .try_into()
            .map_err(|_| ActivationError::Unavailable)?,
    ));
    let operation_len = usize::from(u16::from_be_bytes(
        bytes[26..28]
            .try_into()
            .map_err(|_| ActivationError::Unavailable)?,
    ));
    let path_len = usize::from(u16::from_be_bytes(
        bytes[28..30]
            .try_into()
            .map_err(|_| ActivationError::Unavailable)?,
    ));
    if bytes[30..32] != [0; 2] {
        return Err(ActivationError::Unavailable);
    }
    let expected = HEADER_BYTES
        .checked_add(intent_len)
        .and_then(|value| value.checked_add(operation_len))
        .and_then(|value| value.checked_add(path_len))
        .ok_or(ActivationError::Unavailable)?;
    if expected != bytes.len() {
        return Err(ActivationError::Unavailable);
    }
    let mut offset = HEADER_BYTES;
    let intent_id = decode_text(&bytes[offset..offset + intent_len])?;
    offset += intent_len;
    let operation_id = decode_text(&bytes[offset..offset + operation_len])?;
    offset += operation_len;
    let switch_script_path = decode_text(&bytes[offset..])?;
    Ok(ActivationIntentPayload {
        schema_version,
        intent_id,
        operation_id,
        switch_script_path,
        mode,
        timeout_ms,
    })
}

#[cfg(test)]
pub(crate) fn encode_activation_payload_for_test(
    intent_id: &str,
    operation_id: &str,
    switch_script_path: &str,
    mode: ActivationMode,
    timeout_ms: u64,
) -> Vec<u8> {
    encode_activation_payload(
        intent_id,
        operation_id,
        switch_script_path,
        mode,
        timeout_ms,
    )
    .expect("valid test activation payload")
}

#[cfg(test)]
fn encode_activation_payload(
    intent_id: &str,
    operation_id: &str,
    switch_script_path: &str,
    mode: ActivationMode,
    timeout_ms: u64,
) -> Result<Vec<u8>, ActivationError> {
    let intent_len = u16::try_from(intent_id.len()).map_err(|_| ActivationError::InvalidRequest)?;
    let operation_len =
        u16::try_from(operation_id.len()).map_err(|_| ActivationError::InvalidRequest)?;
    let path_len =
        u16::try_from(switch_script_path.len()).map_err(|_| ActivationError::InvalidRequest)?;
    let mut bytes =
        Vec::with_capacity(32 + intent_id.len() + operation_id.len() + switch_script_path.len());
    bytes.extend_from_slice(&ACTIVATION_PAYLOAD_MAGIC);
    bytes.extend_from_slice(&ACTIVATION_PAYLOAD_SCHEMA_VERSION.to_be_bytes());
    bytes.push(match mode {
        ActivationMode::Switch => 1,
        ActivationMode::Boot => 2,
        ActivationMode::Test => 3,
        ActivationMode::DryActivate => 4,
    });
    bytes.extend_from_slice(&[0; 3]);
    bytes.extend_from_slice(&timeout_ms.to_be_bytes());
    bytes.extend_from_slice(&intent_len.to_be_bytes());
    bytes.extend_from_slice(&operation_len.to_be_bytes());
    bytes.extend_from_slice(&path_len.to_be_bytes());
    bytes.extend_from_slice(&[0; 2]);
    bytes.extend_from_slice(intent_id.as_bytes());
    bytes.extend_from_slice(operation_id.as_bytes());
    bytes.extend_from_slice(switch_script_path.as_bytes());
    Ok(bytes)
}

fn encode_activation_record(record: &ActivationRecord) -> Result<Vec<u8>, ActivationError> {
    let intent_len =
        u16::try_from(record.intent_id.len()).map_err(|_| ActivationError::Unavailable)?;
    let operation_len =
        u16::try_from(record.operation_id.len()).map_err(|_| ActivationError::Unavailable)?;
    let request_id =
        decode_hex_array::<16>(&record.request_id).ok_or(ActivationError::Unavailable)?;
    let intent_digest =
        decode_hex_array::<32>(&record.intent_digest).ok_or(ActivationError::Unavailable)?;
    let mut flags = 0_u8;
    flags |= u8::from(record.exit_code.is_some());
    flags |= u8::from(record.signal.is_some()) << 1;
    flags |= u8::from(record.status_code.is_some()) << 2;
    let mut bytes = Vec::with_capacity(80 + record.intent_id.len() + record.operation_id.len());
    bytes.extend_from_slice(&ACTIVATION_RECORD_MAGIC);
    bytes.extend_from_slice(&record.schema_version.to_be_bytes());
    bytes.push(activation_state_code(record.state));
    bytes.push(flags);
    bytes.extend_from_slice(&[0; 2]);
    bytes.extend_from_slice(&record.exit_code.unwrap_or_default().to_be_bytes());
    bytes.extend_from_slice(&record.signal.unwrap_or_default().to_be_bytes());
    bytes.extend_from_slice(&record.status_code.unwrap_or_default().to_be_bytes());
    bytes.extend_from_slice(&intent_len.to_be_bytes());
    bytes.extend_from_slice(&operation_len.to_be_bytes());
    bytes.extend_from_slice(&request_id);
    bytes.extend_from_slice(&intent_digest);
    bytes.extend_from_slice(record.intent_id.as_bytes());
    bytes.extend_from_slice(record.operation_id.as_bytes());
    Ok(bytes)
}

fn decode_activation_record(bytes: &[u8]) -> Result<ActivationRecord, ActivationError> {
    const HEADER_BYTES: usize = 80;
    if bytes.len() < HEADER_BYTES || bytes[..8] != ACTIVATION_RECORD_MAGIC {
        return Err(ActivationError::Unavailable);
    }
    let schema_version = u32::from_be_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| ActivationError::Unavailable)?,
    );
    let state = activation_state_from_code(bytes[12]).ok_or(ActivationError::Unavailable)?;
    let flags = bytes[13];
    if flags & !0b111 != 0 || bytes[14..16] != [0; 2] {
        return Err(ActivationError::Unavailable);
    }
    let exit_code = i32::from_be_bytes(
        bytes[16..20]
            .try_into()
            .map_err(|_| ActivationError::Unavailable)?,
    );
    let signal = u32::from_be_bytes(
        bytes[20..24]
            .try_into()
            .map_err(|_| ActivationError::Unavailable)?,
    );
    let status_code = i32::from_be_bytes(
        bytes[24..28]
            .try_into()
            .map_err(|_| ActivationError::Unavailable)?,
    );
    let intent_len = usize::from(u16::from_be_bytes(
        bytes[28..30]
            .try_into()
            .map_err(|_| ActivationError::Unavailable)?,
    ));
    let operation_len = usize::from(u16::from_be_bytes(
        bytes[30..32]
            .try_into()
            .map_err(|_| ActivationError::Unavailable)?,
    ));
    let expected = HEADER_BYTES
        .checked_add(intent_len)
        .and_then(|value| value.checked_add(operation_len))
        .ok_or(ActivationError::Unavailable)?;
    if expected != bytes.len() {
        return Err(ActivationError::Unavailable);
    }
    let request_id = hex(&bytes[32..48]);
    let intent_digest = hex(&bytes[48..80]);
    let intent_id = decode_text(&bytes[80..80 + intent_len])?;
    let operation_id = decode_text(&bytes[80 + intent_len..])?;
    Ok(ActivationRecord {
        schema_version,
        intent_id,
        operation_id,
        request_id,
        intent_digest,
        state,
        exit_code: (flags & 1 != 0).then_some(exit_code),
        signal: (flags & 2 != 0).then_some(signal),
        status_code: (flags & 4 != 0).then_some(status_code),
    })
}

fn activation_state_code(state: ActivationState) -> u8 {
    match state {
        ActivationState::Running => 1,
        ActivationState::Succeeded => 2,
        ActivationState::Failed => 3,
        ActivationState::TimedOut => 4,
        ActivationState::Cancelled => 5,
        ActivationState::Lost => 6,
    }
}

fn activation_state_from_code(code: u8) -> Option<ActivationState> {
    match code {
        1 => Some(ActivationState::Running),
        2 => Some(ActivationState::Succeeded),
        3 => Some(ActivationState::Failed),
        4 => Some(ActivationState::TimedOut),
        5 => Some(ActivationState::Cancelled),
        6 => Some(ActivationState::Lost),
        _ => None,
    }
}

fn decode_text(bytes: &[u8]) -> Result<String, ActivationError> {
    if bytes.is_empty() || bytes.contains(&0) {
        return Err(ActivationError::Unavailable);
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| ActivationError::Unavailable)
}

pub fn configured_activation_intent_id(workload_id: &str) -> Option<String> {
    if workload_id.is_empty()
        || workload_id.len() > 96
        || !workload_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return None;
    }
    Some(format!("activation-{workload_id}"))
}

fn activation_unit_name(operation_id: &str) -> Result<String, ActivationError> {
    if !valid_operation_id(operation_id) {
        return Err(ActivationError::InvalidRequest);
    }
    Ok(format!("d2b-{operation_id}.service"))
}

fn valid_operation_id(value: &str) -> bool {
    value.strip_prefix("activation-").is_some_and(|suffix| {
        suffix.len() == 32
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn executable_ready(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    !metadata.file_type().is_symlink()
        && metadata.file_type().is_file()
        && metadata.mode() & 0o111 != 0
}

fn validate_status_dir(path: &Path) -> Result<(), ActivationError> {
    if !path.is_absolute() || path == Path::new("/nix/store") || path.starts_with("/nix/store/") {
        return Err(ActivationError::Unavailable);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| ActivationError::Unavailable)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_dir()
        || metadata.uid() != required_owner_uid()
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(ActivationError::Unavailable);
    }
    Ok(())
}

fn validate_private_file(path: &Path) -> Result<(), ActivationError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ActivationError::Unavailable)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || metadata.uid() != required_owner_uid()
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(ActivationError::Unavailable);
    }
    Ok(())
}

fn validate_switch_script_path(path: &Path, store_root: &Path) -> Result<(), ActivationError> {
    let value = path.to_str().ok_or(ActivationError::InvalidRequest)?;
    if store_root != Path::new("/nix/store") && !cfg!(test) {
        return Err(ActivationError::InvalidRequest);
    }
    let prefix = format!("{}/", store_root.display());
    let rest = value
        .strip_prefix(&prefix)
        .ok_or(ActivationError::InvalidRequest)?;
    let store_name = rest
        .strip_suffix("/bin/switch-to-configuration")
        .ok_or(ActivationError::InvalidRequest)?;
    if store_name.contains('/') || !strict_nix_store_basename(store_name) {
        return Err(ActivationError::InvalidRequest);
    }

    if !executable_ready(path) {
        return Err(ActivationError::Unavailable);
    }

    Ok(())
}

fn required_owner_uid() -> u32 {
    #[cfg(test)]
    {
        rustix::process::geteuid().as_raw()
    }
    #[cfg(not(test))]
    {
        0
    }
}

fn strict_nix_store_basename(value: &str) -> bool {
    const NIX_BASE32: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";
    let bytes = value.as_bytes();
    bytes.len() >= 34
        && bytes[32] == b'-'
        && bytes[..32].iter().all(|byte| NIX_BASE32.contains(byte))
        && bytes[33..].iter().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.' | b'_' | b'?' | b'=')
        })
}

fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, ActivationError> {
    let file = File::open(path).map_err(|_| ActivationError::Unavailable)?;
    let metadata = file.metadata().map_err(|_| ActivationError::Unavailable)?;
    if metadata.len() == 0 || metadata.len() > max {
        return Err(ActivationError::Unavailable);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    use std::io::Read as _;
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ActivationError::Unavailable)?;
    if bytes.len() as u64 > max {
        return Err(ActivationError::Unavailable);
    }
    Ok(bytes)
}

fn parse_properties(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn update_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn decode_hex_array<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut output = [0_u8; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    #[derive(Default)]
    struct FakeUnits {
        starts: Mutex<Vec<(String, ActivationMode, u64)>>,
        query: Mutex<Option<ActivationSnapshot>>,
        cancels: AtomicUsize,
    }

    #[async_trait]
    impl ActivationUnitManager for FakeUnits {
        async fn start_unit(
            &self,
            unit_name: &str,
            _: &Path,
            mode: ActivationMode,
            timeout_ms: u64,
        ) -> Result<(), ActivationError> {
            self.starts
                .lock()
                .unwrap()
                .push((unit_name.to_owned(), mode, timeout_ms));
            Ok(())
        }

        async fn query_unit(
            &self,
            _: &str,
            _: [u8; 32],
        ) -> Result<Option<ActivationSnapshot>, ActivationError> {
            Ok(self.query.lock().unwrap().clone())
        }

        async fn cancel_unit(&self, _: &str) -> Result<(), ActivationError> {
            self.cancels.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn cleanup_terminal_unit(
            &self,
            _: &str,
            _: &ActivationSnapshot,
        ) -> Result<(), ActivationError> {
            Ok(())
        }
    }

    struct TestTree {
        root: PathBuf,
        status: PathBuf,
        systemd_run: PathBuf,
        systemctl: PathBuf,
        switch: PathBuf,
    }

    impl TestTree {
        fn new(tag: &str) -> Self {
            let root = std::env::var_os("CARGO_TARGET_TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap().join("target/test-tmp"))
                .join(format!("activation-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            let status = root.join("status");
            fs::create_dir(&status).unwrap();
            fs::set_permissions(&status, fs::Permissions::from_mode(0o700)).unwrap();
            let systemd_run = executable(&root.join("systemd-run"));
            let systemctl = executable(&root.join("systemctl"));
            let store = root.join("nix/store");
            fs::create_dir_all(&store).unwrap();
            let switch = store
                .join("0123456789abcdfghijklmnpqrsvwxyz-nixos-system")
                .join("bin/switch-to-configuration");
            fs::create_dir_all(switch.parent().unwrap()).unwrap();
            executable(&switch);
            Self {
                root,
                status,
                systemd_run,
                systemctl,
                switch,
            }
        }

        fn config(&self) -> ActivationRuntimeConfig {
            ActivationRuntimeConfig {
                workload_id: "bbbbbbbbbbbbbbbbbba".to_owned(),
                systemd_run_path: self.systemd_run.clone(),
                systemctl_path: self.systemctl.clone(),
                status_dir: self.status.clone(),
                switch_store_root: self.root.join("nix/store"),
                max_timeout_ms: 30_000,
            }
        }

        fn write_payload(&self, operation_id: &str, timeout_ms: u64) -> [u8; 32] {
            let intent_id = configured_activation_intent_id(&self.config().workload_id).unwrap();
            let bytes = encode_activation_payload_for_test(
                &intent_id,
                operation_id,
                &self.switch.display().to_string(),
                ActivationMode::Switch,
                timeout_ms,
            );
            let path = self.status.join(ACTIVATION_PAYLOAD_FILE);
            fs::write(&path, &bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            Sha256::digest(&bytes).into()
        }
    }

    impl Drop for TestTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn executable(path: &Path) -> PathBuf {
        fs::write(path, b"binary").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        path.to_path_buf()
    }

    fn operation() -> &'static str {
        "activation-0123456789abcdef0123456789abcdef"
    }

    #[test]
    fn configured_intent_is_workload_bound_and_opaque() {
        assert_eq!(
            configured_activation_intent_id("bbbbbbbbbbbbbbbbbba").as_deref(),
            Some("activation-bbbbbbbbbbbbbbbbbba")
        );
        assert!(configured_activation_intent_id("../bad").is_none());
    }

    #[test]
    fn readiness_requires_binaries_private_storage_and_usable_payload() {
        let tree = TestTree::new("readiness");
        let units = Arc::new(FakeUnits::default());
        let runtime = ActivationRuntime::with_manager(tree.config(), units);
        assert!(!runtime.ready());
        tree.write_payload(operation(), 5_000);
        assert!(runtime.ready());
        fs::set_permissions(&tree.status, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!runtime.ready());
    }

    #[tokio::test]
    async fn activation_starts_rejoins_and_cancels_without_free_command_input() {
        let tree = TestTree::new("lifecycle");
        let digest = tree.write_payload(operation(), 5_000);
        let units = Arc::new(FakeUnits::default());
        let runtime = ActivationRuntime::with_manager(tree.config(), units.clone());
        let request_id = [7_u8; 16];
        let started = runtime
            .activate(
                &request_id,
                runtime.configured_intent_id(),
                operation(),
                &digest,
            )
            .await
            .unwrap();
        assert_eq!(started.state, ActivationState::Running);
        assert_eq!(units.starts.lock().unwrap().len(), 1);

        let rejoined = ActivationRuntime::with_manager(tree.config(), units.clone())
            .inspect(runtime.configured_intent_id(), operation(), Some(&digest))
            .await
            .unwrap();
        assert_eq!(rejoined.state, ActivationState::Lost);

        let tree = TestTree::new("cancel");
        let digest = tree.write_payload(operation(), 5_000);
        let units = Arc::new(FakeUnits {
            query: Mutex::new(Some(ActivationSnapshot::running(digest))),
            ..Default::default()
        });
        let runtime = ActivationRuntime::with_manager(tree.config(), units.clone());
        runtime
            .activate(
                &request_id,
                runtime.configured_intent_id(),
                operation(),
                &digest,
            )
            .await
            .unwrap();
        assert_eq!(
            runtime.cancel_by_request(&request_id).await.unwrap(),
            ActivationCancelOutcome::Signalled
        );
        assert_eq!(units.cancels.load(Ordering::SeqCst), 1);
        assert_eq!(
            runtime
                .inspect(runtime.configured_intent_id(), operation(), Some(&digest))
                .await
                .unwrap()
                .state,
            ActivationState::Cancelled
        );
    }

    #[test]
    fn production_command_has_no_shell_or_free_form_wrapper() {
        let source = include_str!("activation.rs");
        assert!(source.contains("KillMode=control-group"));
        assert!(source.contains("RuntimeMaxSec="));
        assert!(!source.contains(".arg(\"sh\")"));
        assert!(!source.contains(".arg(\"-c\")"));
    }

    #[test]
    fn runtime_debug_and_errors_are_redacted() {
        let runtime = ActivationRuntime::production("bbbbbbbbbbbbbbbbbba".to_owned(), None);
        let rendered = format!("{runtime:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("/nix/store"));
        assert_eq!(format!("{:?}", ActivationError::SpawnFailed), "SpawnFailed");
    }
}
