use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;

use nixling_constellation_core as constellation;
use nixling_ipc::guest_proto as pb;
use protobuf::{EnumOrUnknown, MessageField};

pub const DEFAULT_SHELL_SESSIONS_PER_VM: u32 = 8;
pub const DEFAULT_SHELL_ATTACHED_SESSIONS_PER_VM: u32 = 1;
const EVENT_QUEUE_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRuntimeConfig {
    pub default_name: String,
    pub max_sessions: u32,
    pub max_attached: u32,
    pub workload_user: Option<String>,
    pub workload_uid: Option<u32>,
    pub guest_boot_id: String,
    pub guestd_instance_id: String,
    pub daemon_instance_id: String,
    pub runner_path: PathBuf,
    pub systemctl_path: PathBuf,
    pub socket_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellCoreAdaptError {
    Disabled,
    InvalidGeneration,
    InvalidName,
    InvalidOpaqueId,
    InvalidCursor,
}

impl ShellRuntimeConfig {
    pub fn disabled() -> Self {
        Self {
            default_name: "default".to_owned(),
            max_sessions: DEFAULT_SHELL_SESSIONS_PER_VM,
            max_attached: DEFAULT_SHELL_ATTACHED_SESSIONS_PER_VM,
            workload_user: None,
            workload_uid: None,
            guest_boot_id: String::new(),
            guestd_instance_id: String::new(),
            daemon_instance_id: String::new(),
            runner_path: PathBuf::new(),
            systemctl_path: PathBuf::new(),
            socket_path: PathBuf::new(),
        }
    }
}

pub trait ShellDaemonManager: Send + Sync {
    fn ensure_ready(&self, config: &ShellRuntimeConfig) -> Result<String, ShellRuntimeError>;
}

pub trait ShellHelperSpawner: Send + Sync {
    fn spawn_helper(&self, name: &str) -> Result<String, ShellRuntimeError>;
}

pub trait ShellClock: Send + Sync {
    fn now_ms(&self) -> u64;
}

pub trait ShellEventSink: Send + Sync {
    fn publish(&self, event: ShellEvent);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellSession {
    name: String,
    session_id: String,
    shell_session_instance_id: String,
    guestd_instance_id: String,
    daemon_instance_id: String,
    guest_boot_id: String,
    owner_key: Vec<u8>,
    attached: bool,
    terminal_exec_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellRuntimeError {
    InvalidName,
    CapacityExceeded,
    AttachCapacityExceeded,
    AlreadyAttached,
    NotFound,
    Disabled,
    PoolUnavailable,
}

impl ShellRuntimeError {
    fn wire(self) -> pb::GuestControlErrorKind {
        match self {
            Self::InvalidName => {
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_INVALID_NAME
            }
            Self::CapacityExceeded => {
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_CAPACITY_EXCEEDED
            }
            Self::AttachCapacityExceeded => {
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_ATTACH_CAPACITY_EXCEEDED
            }
            Self::AlreadyAttached => {
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_ALREADY_ATTACHED
            }
            Self::NotFound => pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_NOT_FOUND,
            Self::Disabled => {
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_SHELL_DISABLED
            }
            Self::PoolUnavailable => {
                pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_POOL_UNAVAILABLE
            }
        }
    }

    fn remediation(self) -> pb::HealthRemediation {
        match self {
            Self::Disabled | Self::PoolUnavailable => {
                pb::HealthRemediation::HEALTH_REMEDIATION_CHECK_GUESTD_SERVICE
            }
            Self::CapacityExceeded | Self::AttachCapacityExceeded => {
                pb::HealthRemediation::HEALTH_REMEDIATION_REDUCE_LOAD
            }
            Self::InvalidName | Self::AlreadyAttached | Self::NotFound => {
                pb::HealthRemediation::HEALTH_REMEDIATION_NONE
            }
        }
    }

    fn shell_state(self) -> pb::ShellState {
        match self {
            Self::Disabled => pb::ShellState::SHELL_STATE_FEATURE_DISABLED,
            Self::PoolUnavailable => pb::ShellState::SHELL_STATE_POOL_UNAVAILABLE,
            Self::AlreadyAttached => pb::ShellState::SHELL_STATE_ATTACHED,
            Self::InvalidName
            | Self::CapacityExceeded
            | Self::AttachCapacityExceeded
            | Self::NotFound => pb::ShellState::SHELL_STATE_UNSPECIFIED,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellEventKind {
    AttachCreated,
    AttachReused,
    ForceEvicted,
    Detached,
    Killed,
    ReconciliationGap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellEvent {
    pub seq: u64,
    pub kind: ShellEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellEventBatch {
    pub events: Vec<ShellEvent>,
    pub dropped_events: u64,
    pub cursor: u64,
}

#[derive(Debug)]
struct ShellEventQueue {
    events: VecDeque<ShellEvent>,
    next_seq: u64,
    dropped_events: u64,
    capacity: usize,
}

impl ShellEventQueue {
    fn new(capacity: usize) -> Self {
        Self {
            events: VecDeque::new(),
            next_seq: 1,
            dropped_events: 0,
            capacity,
        }
    }

    fn push(&mut self, kind: ShellEventKind) {
        if self.events.len() == self.capacity {
            self.events.pop_front();
            self.dropped_events = self.dropped_events.saturating_add(1);
        }
        let event = ShellEvent {
            seq: self.next_seq,
            kind,
        };
        self.next_seq = self.next_seq.saturating_add(1);
        self.events.push_back(event);
    }

    fn read_since(&self, after_seq: u64, limit: usize) -> ShellEventBatch {
        let mut events = Vec::new();
        if self.dropped_events > 0
            && limit > 0
            && self
                .events
                .front()
                .is_some_and(|oldest| oldest.seq > after_seq.saturating_add(1))
        {
            let gap_seq = self
                .events
                .front()
                .map(|oldest| oldest.seq.saturating_sub(1))
                .unwrap_or_else(|| after_seq.saturating_add(1));
            events.push(ShellEvent {
                seq: gap_seq,
                kind: ShellEventKind::ReconciliationGap,
            });
        }
        events.extend(
            self.events
                .iter()
                .copied()
                .filter(|event| event.seq > after_seq)
                .take(limit.saturating_sub(events.len())),
        );
        let cursor = events.last().map(|event| event.seq).unwrap_or(after_seq);
        ShellEventBatch {
            events,
            dropped_events: self.dropped_events,
            cursor,
        }
    }
}

#[derive(Default)]
struct ShellRuntimeState {
    sessions: BTreeMap<String, ShellSession>,
    next_session: u64,
    events: ShellEventQueue,
}

impl Default for ShellEventQueue {
    fn default() -> Self {
        Self::new(EVENT_QUEUE_CAPACITY)
    }
}

pub struct ShellRuntime {
    enabled: bool,
    config: ShellRuntimeConfig,
    state: Mutex<ShellRuntimeState>,
}

impl ShellRuntime {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            config: ShellRuntimeConfig::disabled(),
            state: Mutex::new(ShellRuntimeState::default()),
        }
    }

    pub fn enabled(config: ShellRuntimeConfig) -> Self {
        assert_low_cardinality_boundary();
        Self {
            enabled: true,
            config,
            state: Mutex::new(ShellRuntimeState::default()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn limits(&self) -> (u32, u32) {
        (self.config.max_sessions, self.config.max_attached)
    }

    pub fn runner_path(&self) -> Option<PathBuf> {
        self.enabled.then(|| self.config.runner_path.clone())
    }

    pub fn systemctl_path(&self) -> Option<PathBuf> {
        self.enabled.then(|| self.config.systemctl_path.clone())
    }

    pub fn socket_path(&self) -> Option<PathBuf> {
        self.enabled.then(|| self.config.socket_path.clone())
    }

    pub fn guest_boot_id(&self) -> Option<String> {
        self.enabled.then(|| self.config.guest_boot_id.clone())
    }

    pub fn default_name(&self) -> String {
        self.config.default_name.clone()
    }

    pub fn resolve_name(&self, name: Option<String>) -> Result<String, ShellRuntimeError> {
        let resolved = name.unwrap_or_else(|| self.config.default_name.clone());
        validate_shell_name(&resolved).map_err(|_| ShellRuntimeError::InvalidName)?;
        if !self.enabled {
            return Err(ShellRuntimeError::Disabled);
        }
        Ok(resolved)
    }

    pub fn session_attached(&self, name: &str) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .sessions
            .get(name)
            .is_some_and(|session| session.attached)
    }

    pub fn session_exists(&self, name: &str) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.sessions.contains_key(name)
    }

    pub fn attached_snapshot(&self) -> Vec<(String, Vec<u8>)> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .sessions
            .values()
            .filter(|session| session.attached)
            .map(|session| (session.name.clone(), session.owner_key.clone()))
            .collect()
    }

    pub fn restore_failed_attach(
        &self,
        resolved_name: &str,
        session_id: &str,
        existed_before: bool,
        attached_before: &[(String, Vec<u8>)],
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !existed_before
            && state
                .sessions
                .get(resolved_name)
                .is_some_and(|session| session.session_id == session_id)
        {
            state.sessions.remove(resolved_name);
        }
        for session in state.sessions.values_mut() {
            session.attached = false;
        }
        for (name, owner_key) in attached_before {
            if let Some(session) = state.sessions.get_mut(name) {
                session.attached = true;
                session.owner_key = owner_key.clone();
            }
        }
    }

    pub fn attach(&self, request: pb::ShellAttachRequest) -> pb::ShellAttachResponse {
        self.attach_with_owner(request, Vec::new())
    }

    pub fn attach_with_owner(
        &self,
        request: pb::ShellAttachRequest,
        owner_key: Vec<u8>,
    ) -> pb::ShellAttachResponse {
        let name = request
            .name
            .as_deref()
            .unwrap_or(&self.config.default_name)
            .to_owned();
        let result = self.attach_inner(name.clone(), request.force, owner_key);
        match result {
            Ok((session, force_evicted)) => {
                let mut response = pb::ShellAttachResponse::new();
                response.session_id = Some(session.session_id);
                response.resolved_name = session.name;
                response.state = EnumOrUnknown::new(if session.attached {
                    pb::ShellState::SHELL_STATE_ATTACHED
                } else {
                    pb::ShellState::SHELL_STATE_DETACHED
                });
                response.force_evicted = force_evicted;
                response.effective_limits = MessageField::some(shell_effective_limits(self));
                response
            }
            Err(error) => shell_attach_error(error, name),
        }
    }

    fn attach_inner(
        &self,
        name: String,
        force: bool,
        owner_key: Vec<u8>,
    ) -> Result<(ShellSession, bool), ShellRuntimeError> {
        validate_shell_name(&name).map_err(|_| ShellRuntimeError::InvalidName)?;
        if !self.enabled {
            return Err(ShellRuntimeError::Disabled);
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let attached_count = state
            .sessions
            .values()
            .filter(|session| session.attached)
            .count();
        let mut force_evicted = false;
        if let Some(existing_view) = state.sessions.get(&name) {
            let was_attached = existing_view.attached;
            if was_attached && !force {
                return Err(ShellRuntimeError::AlreadyAttached);
            }
            if !was_attached && attached_count >= self.config.max_attached as usize {
                if force {
                    if let Some((_victim_name, victim)) = state
                        .sessions
                        .iter_mut()
                        .find(|(candidate, session)| candidate.as_str() != name && session.attached)
                    {
                        victim.attached = false;
                        state.events.push(ShellEventKind::ForceEvicted);
                        force_evicted = true;
                    } else {
                        return Err(ShellRuntimeError::AttachCapacityExceeded);
                    }
                } else {
                    return Err(ShellRuntimeError::AttachCapacityExceeded);
                }
            }
            let existing = state
                .sessions
                .get_mut(&name)
                .expect("existing session remains present");
            existing.attached = true;
            existing.owner_key = owner_key;
            let cloned = existing.clone();
            if was_attached && force {
                state.events.push(ShellEventKind::ForceEvicted);
            }
            state.events.push(ShellEventKind::AttachReused);
            return Ok((cloned, (was_attached && force) || force_evicted));
        }
        if state.sessions.len() >= self.config.max_sessions as usize {
            return Err(ShellRuntimeError::CapacityExceeded);
        }
        if attached_count >= self.config.max_attached as usize {
            if force {
                if let Some((_victim_name, victim)) = state
                    .sessions
                    .iter_mut()
                    .find(|(_candidate, session)| session.attached)
                {
                    victim.attached = false;
                    state.events.push(ShellEventKind::ForceEvicted);
                    force_evicted = true;
                } else {
                    return Err(ShellRuntimeError::AttachCapacityExceeded);
                }
            } else {
                return Err(ShellRuntimeError::AttachCapacityExceeded);
            }
        }
        state.next_session = state.next_session.saturating_add(1);
        let session_id = generate_opaque_id("shell").ok_or(ShellRuntimeError::PoolUnavailable)?;
        let shell_session_instance_id =
            generate_opaque_id("shinst").ok_or(ShellRuntimeError::PoolUnavailable)?;
        let session = ShellSession {
            name: name.clone(),
            session_id,
            shell_session_instance_id,
            guestd_instance_id: self.config.guestd_instance_id.clone(),
            daemon_instance_id: self.config.daemon_instance_id.clone(),
            guest_boot_id: self.config.guest_boot_id.clone(),
            owner_key,
            attached: true,
            terminal_exec_id: None,
        };
        state.sessions.insert(name, session.clone());
        state.events.push(ShellEventKind::AttachCreated);
        Ok((session, force_evicted))
    }

    pub fn list(&self) -> pb::ShellListResponse {
        if !self.enabled {
            let mut response = pb::ShellListResponse::new();
            response.default_name = self.config.default_name.clone();
            response.error = MessageField::some(shell_error(ShellRuntimeError::Disabled));
            return response;
        }
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut response = pb::ShellListResponse::new();
        response.default_name = self.config.default_name.clone();
        for session in state.sessions.values() {
            let mut entry = pb::ShellListEntry::new();
            entry.name = session.name.clone();
            entry.state = EnumOrUnknown::new(if session.attached {
                pb::ShellState::SHELL_STATE_ATTACHED
            } else {
                pb::ShellState::SHELL_STATE_DETACHED
            });
            entry.attached = session.attached;
            entry.is_default = session.name == self.config.default_name;
            response.sessions.push(entry);
        }
        response
    }

    pub fn detach(&self, name: Option<String>) -> pb::ShellDetachResponse {
        let resolved = name.unwrap_or_else(|| self.config.default_name.clone());
        if let Err(error) =
            validate_shell_name(&resolved).map_err(|_| ShellRuntimeError::InvalidName)
        {
            return shell_detach_error(error, resolved);
        }
        if !self.enabled {
            return shell_detach_error(ShellRuntimeError::Disabled, resolved);
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(session) = state.sessions.get_mut(&resolved) else {
            let mut response = pb::ShellDetachResponse::new();
            response.resolved_name = resolved;
            response.detached = false;
            response.cause =
                EnumOrUnknown::new(pb::ShellCloseCause::SHELL_CLOSE_CAUSE_CLIENT_DETACH);
            return response;
        };
        let was_attached = session.attached;
        session.attached = false;
        if was_attached {
            state.events.push(ShellEventKind::Detached);
        }
        let mut response = pb::ShellDetachResponse::new();
        response.resolved_name = resolved;
        response.detached = was_attached;
        response.cause = EnumOrUnknown::new(pb::ShellCloseCause::SHELL_CLOSE_CAUSE_CLIENT_DETACH);
        response
    }

    pub fn close_attach(&self, session_id: &str) -> pb::ShellDetachResponse {
        if !self.enabled {
            return shell_detach_error(
                ShellRuntimeError::Disabled,
                self.config.default_name.clone(),
            );
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for session in state.sessions.values_mut() {
            if session.session_id == session_id {
                let name = session.name.clone();
                let was_attached = session.attached;
                session.attached = false;
                if was_attached {
                    state.events.push(ShellEventKind::Detached);
                }
                let mut response = pb::ShellDetachResponse::new();
                response.resolved_name = name;
                response.detached = was_attached;
                response.cause =
                    EnumOrUnknown::new(pb::ShellCloseCause::SHELL_CLOSE_CAUSE_CLIENT_DETACH);
                return response;
            }
        }
        shell_detach_error(
            ShellRuntimeError::NotFound,
            self.config.default_name.clone(),
        )
    }

    pub fn set_terminal_exec_id(
        &self,
        session_id: &str,
        exec_id: String,
    ) -> Result<(), ShellRuntimeError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(session) = state
            .sessions
            .values_mut()
            .find(|session| session.session_id == session_id)
        else {
            return Err(ShellRuntimeError::NotFound);
        };
        session.terminal_exec_id = Some(exec_id);
        Ok(())
    }

    pub fn terminal_exec_id(&self, session_id: &str) -> Result<String, ShellRuntimeError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .sessions
            .values()
            .find(|session| session.session_id == session_id)
            .and_then(|session| session.terminal_exec_id.clone())
            .ok_or(ShellRuntimeError::NotFound)
    }

    pub fn kill(&self, name: String) -> pb::ShellKillResponse {
        if let Err(error) = validate_shell_name(&name).map_err(|_| ShellRuntimeError::InvalidName) {
            return shell_kill_error(error, name);
        }
        if !self.enabled {
            return shell_kill_error(ShellRuntimeError::Disabled, name);
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(session) = state.sessions.remove(&name) else {
            let mut response = pb::ShellKillResponse::new();
            response.name = name;
            response.killed = false;
            response.state = EnumOrUnknown::new(pb::ShellState::SHELL_STATE_KILLED);
            return response;
        };
        let _was_attached = session.attached;
        state.events.push(ShellEventKind::Killed);
        let mut response = pb::ShellKillResponse::new();
        response.name = name;
        response.killed = true;
        response.state = EnumOrUnknown::new(pb::ShellState::SHELL_STATE_KILLED);
        response
    }

    pub fn close_connection(&self, owner_key: &[u8]) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut changed = false;
        for session in state.sessions.values_mut() {
            if session.attached && session.owner_key == owner_key {
                session.attached = false;
                changed = true;
            }
        }
        if changed {
            state.events.push(ShellEventKind::Detached);
        }
    }

    pub fn read_events_since(&self, after_seq: u64, limit: usize) -> ShellEventBatch {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.read_events_since(after_seq, limit)
    }

    pub fn core_generation(&self) -> Result<constellation::ShellGeneration, ShellCoreAdaptError> {
        if !self.enabled {
            return Err(ShellCoreAdaptError::Disabled);
        }
        shell_generation_from_config(&self.config)
    }

    pub fn list_core(&self) -> Result<constellation::ShellListResponse, ShellCoreAdaptError> {
        let generation = self.core_generation()?;
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let summaries = state
            .sessions
            .values()
            .map(|session| shell_summary_to_core(session, &self.config.default_name))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(constellation::ShellListResponse {
            generation,
            summaries,
        })
    }

    pub fn events_core_since(
        &self,
        after_seq: u64,
        limit: usize,
    ) -> Result<constellation::ShellEventBatch, ShellCoreAdaptError> {
        let generation = self.core_generation()?;
        let batch = self.read_events_since(after_seq, limit);
        let mut reconciliation_gap = false;
        let events = batch
            .events
            .iter()
            .map(|event| {
                if event.kind == ShellEventKind::ReconciliationGap {
                    reconciliation_gap = true;
                }
                shell_event_to_core(*event, &generation)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(constellation::ShellEventBatch {
            generation,
            events,
            cursor: shell_event_cursor(batch.cursor)?,
            dropped_events: batch.dropped_events,
            reconciliation_gap,
        })
    }
}

impl ShellRuntimeState {
    fn read_events_since(&self, after_seq: u64, limit: usize) -> ShellEventBatch {
        self.events.read_since(after_seq, limit)
    }
}

pub fn shell_effective_limits(runtime: &ShellRuntime) -> pb::GuestEffectiveLimits {
    let (max_sessions, max_attached) = runtime.limits();
    let mut limits = pb::GuestEffectiveLimits::new();
    limits.shell_sessions_per_vm = max_sessions;
    limits.shell_attached_sessions_per_vm = max_attached;
    limits
}

fn shell_attach_error(error: ShellRuntimeError, name: String) -> pb::ShellAttachResponse {
    let mut response = pb::ShellAttachResponse::new();
    response.resolved_name = safe_error_name(error, name);
    response.state = EnumOrUnknown::new(error.shell_state());
    response.error = MessageField::some(shell_error(error));
    response
}

fn shell_detach_error(error: ShellRuntimeError, name: String) -> pb::ShellDetachResponse {
    let mut response = pb::ShellDetachResponse::new();
    response.resolved_name = safe_error_name(error, name);
    response.error = MessageField::some(shell_error(error));
    response
}

fn shell_kill_error(error: ShellRuntimeError, name: String) -> pb::ShellKillResponse {
    let mut response = pb::ShellKillResponse::new();
    response.name = safe_error_name(error, name);
    response.state = EnumOrUnknown::new(error.shell_state());
    response.error = MessageField::some(shell_error(error));
    response
}

fn safe_error_name(error: ShellRuntimeError, name: String) -> String {
    if matches!(error, ShellRuntimeError::InvalidName) {
        "<invalid>".to_owned()
    } else {
        name
    }
}

fn shell_error(error: ShellRuntimeError) -> pb::GuestControlError {
    let mut wire = pb::GuestControlError::new();
    wire.kind = EnumOrUnknown::new(error.wire());
    wire.remediation = EnumOrUnknown::new(error.remediation());
    wire
}

fn generate_opaque_id(prefix: &str) -> Option<String> {
    let mut bytes = [0_u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .ok()?;
    let mut out = String::with_capacity(prefix.len() + 1 + bytes.len() * 2);
    out.push_str(prefix);
    out.push('-');
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    Some(out)
}

fn shell_generation_from_config(
    config: &ShellRuntimeConfig,
) -> Result<constellation::ShellGeneration, ShellCoreAdaptError> {
    Ok(constellation::ShellGeneration {
        guest_boot_id: constellation::ProtocolToken::parse(config.guest_boot_id.clone())
            .map_err(|_| ShellCoreAdaptError::InvalidGeneration)?,
        guestd_instance_id: constellation::ProtocolToken::parse(config.guestd_instance_id.clone())
            .map_err(|_| ShellCoreAdaptError::InvalidGeneration)?,
        shell_daemon_instance_id: constellation::ProtocolToken::parse(
            config.daemon_instance_id.clone(),
        )
        .map_err(|_| ShellCoreAdaptError::InvalidGeneration)?,
    })
}

fn shell_summary_to_core(
    session: &ShellSession,
    default_name: &str,
) -> Result<constellation::ShellSummary, ShellCoreAdaptError> {
    Ok(constellation::ShellSummary {
        name: constellation::ShellName::parse(session.name.clone())
            .map_err(|_| ShellCoreAdaptError::InvalidName)?,
        state: if session.attached {
            constellation::ShellState::Attached
        } else {
            constellation::ShellState::Detached
        },
        generation: constellation::ShellGeneration {
            guest_boot_id: constellation::ProtocolToken::parse(session.guest_boot_id.clone())
                .map_err(|_| ShellCoreAdaptError::InvalidGeneration)?,
            guestd_instance_id: constellation::ProtocolToken::parse(
                session.guestd_instance_id.clone(),
            )
            .map_err(|_| ShellCoreAdaptError::InvalidGeneration)?,
            shell_daemon_instance_id: constellation::ProtocolToken::parse(
                session.daemon_instance_id.clone(),
            )
            .map_err(|_| ShellCoreAdaptError::InvalidGeneration)?,
        },
        session_instance_id: Some(
            constellation::ShellSessionInstanceId::parse(session.shell_session_instance_id.clone())
                .map_err(|_| ShellCoreAdaptError::InvalidOpaqueId)?,
        ),
        attached: session.attached,
        is_default: session.name == default_name,
        last_cause: None,
    })
}

fn shell_event_cursor(seq: u64) -> Result<constellation::StreamCursor, ShellCoreAdaptError> {
    constellation::StreamCursor::parse(format!("shell-event-{seq}"))
        .map_err(|_| ShellCoreAdaptError::InvalidCursor)
}

fn shell_event_to_core(
    event: ShellEvent,
    generation: &constellation::ShellGeneration,
) -> Result<constellation::ShellEventSummary, ShellCoreAdaptError> {
    let (state, cause) = match event.kind {
        ShellEventKind::AttachCreated | ShellEventKind::AttachReused => (
            constellation::ShellState::Attached,
            constellation::ShellCause::Unknown,
        ),
        ShellEventKind::ForceEvicted => (
            constellation::ShellState::Detached,
            constellation::ShellCause::ForceDetach,
        ),
        ShellEventKind::Detached => (
            constellation::ShellState::Detached,
            constellation::ShellCause::AdminDetach,
        ),
        ShellEventKind::Killed => (
            constellation::ShellState::Exited,
            constellation::ShellCause::AdminKill,
        ),
        ShellEventKind::ReconciliationGap => (
            constellation::ShellState::Lost,
            constellation::ShellCause::ReconciliationGap,
        ),
    };
    Ok(constellation::ShellEventSummary {
        cursor: shell_event_cursor(event.seq)?,
        generation: generation.clone(),
        state,
        cause,
    })
}

fn validate_shell_name(name: &str) -> Result<(), ()> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err(());
    }
    let first = bytes[0];
    if !(first.is_ascii_alphanumeric() || first == b'_') {
        return Err(());
    }
    if bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(())
    }
}

fn assert_low_cardinality_boundary() {
    // User-provided shell names and generated session ids are intentionally
    // stored only as runtime state. Future metrics/audit exporters must not turn
    // them into labels or structured tags.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_runtime() -> ShellRuntime {
        ShellRuntime::enabled(ShellRuntimeConfig {
            default_name: "default".to_owned(),
            max_sessions: 2,
            max_attached: 1,
            workload_user: Some("alice".to_owned()),
            workload_uid: Some(1000),
            guest_boot_id: "boot-1".to_owned(),
            guestd_instance_id: "guestd-1".to_owned(),
            daemon_instance_id: "daemon-1".to_owned(),
            runner_path: PathBuf::new(),
            systemctl_path: PathBuf::new(),
            socket_path: PathBuf::new(),
        })
    }

    #[test]
    fn attach_uses_default_and_lists_session() {
        let runtime = enabled_runtime();
        let response = runtime.attach(pb::ShellAttachRequest::new());
        assert_eq!(response.resolved_name, "default");
        assert!(response.session_id.is_some());
        assert!(response.error.is_none());

        let listed = runtime.list();
        assert_eq!(listed.default_name, "default");
        assert_eq!(listed.sessions.len(), 1);
        assert!(listed.sessions[0].attached);
        assert!(listed.sessions[0].is_default);
    }

    #[test]
    fn attach_uses_opaque_unpredictable_session_ids() {
        let runtime = enabled_runtime();
        let first = runtime
            .attach(pb::ShellAttachRequest::new())
            .session_id
            .expect("first session id");
        assert!(first.starts_with("shell-"));
        assert_ne!(first, "shell-0000000000000001");
        assert!(constellation::ShellAttachId::parse(first.clone()).is_ok());

        let _ = runtime.detach(None);
        let mut other = pb::ShellAttachRequest::new();
        other.name = Some("other".to_owned());
        let second = runtime.attach(other).session_id.expect("second session id");
        assert!(second.starts_with("shell-"));
        assert_ne!(first, second);
        assert_ne!(second, "shell-0000000000000002");
    }

    #[test]
    fn attach_without_force_rejects_existing_attached_session() {
        let runtime = enabled_runtime();
        assert!(
            runtime
                .attach(pb::ShellAttachRequest::new())
                .error
                .is_none()
        );
        let rejected = runtime.attach(pb::ShellAttachRequest::new());
        assert_eq!(
            rejected.error.unwrap().kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_ALREADY_ATTACHED
        );
    }

    #[test]
    fn force_attach_reuses_victim_slot() {
        let runtime = enabled_runtime();
        runtime.attach(pb::ShellAttachRequest::new());
        let mut req = pb::ShellAttachRequest::new();
        req.name = Some("other".to_owned());
        req.force = true;
        let res = runtime.attach(req);
        assert!(
            res.force_evicted,
            "expected force_evicted when evicting another session on new creation"
        );
        assert_eq!(res.resolved_name, "other");
        let listed = runtime.list();
        assert!(
            listed
                .sessions
                .iter()
                .any(|entry| entry.name == "default" && !entry.attached)
        );
        assert!(
            listed
                .sessions
                .iter()
                .any(|entry| entry.name == "other" && entry.attached)
        );
        let mut req2 = pb::ShellAttachRequest::new();
        req2.name = Some("default".to_owned());
        req2.force = true;
        let res2 = runtime.attach(req2);
        assert!(
            res2.force_evicted,
            "expected force_evicted when evicting another session on existing unattached"
        );
    }

    #[test]
    fn close_connection_releases_owned_attachments() {
        let runtime = enabled_runtime();
        let mut request = pb::ShellAttachRequest::new();
        request.name = Some("owned".to_owned());
        let owner = vec![7, 7, 7];
        let attached = runtime.attach_with_owner(request, owner.clone());
        assert!(attached.error.is_none());

        runtime.close_connection(&owner);
        let listed = runtime.list();
        let entry = listed
            .sessions
            .iter()
            .find(|entry| entry.name == "owned")
            .expect("owned session listed");
        assert!(!entry.attached);
    }

    #[test]
    fn detach_and_kill_are_bounded() {
        let runtime = enabled_runtime();
        assert!(
            runtime
                .attach(pb::ShellAttachRequest::new())
                .error
                .is_none()
        );
        let detached = runtime.detach(None);
        assert_eq!(detached.resolved_name, "default");
        assert!(detached.detached);
        let killed = runtime.kill("default".to_owned());
        assert!(killed.killed);
        assert_eq!(
            killed.state.enum_value().unwrap(),
            pb::ShellState::SHELL_STATE_KILLED
        );
        let detached_again = runtime.detach(None);
        assert!(detached_again.error.is_none());
        assert!(!detached_again.detached);
        let killed_again = runtime.kill("default".to_owned());
        assert!(killed_again.error.is_none());
        assert!(!killed_again.killed);
        assert_eq!(
            killed_again.state.enum_value().unwrap(),
            pb::ShellState::SHELL_STATE_KILLED
        );
    }

    #[test]
    fn close_attach_detaches_by_session_id() {
        let runtime = enabled_runtime();
        let attached = runtime.attach(pb::ShellAttachRequest::new());
        let session_id = attached.session_id.expect("session id");
        let closed = runtime.close_attach(&session_id);
        assert_eq!(closed.resolved_name, "default");
        assert!(closed.detached);

        let listed = runtime.list();
        assert_eq!(listed.sessions.len(), 1);
        assert!(!listed.sessions[0].attached);
    }

    #[test]
    fn disabled_runtime_returns_shell_disabled_errors() {
        let runtime = ShellRuntime::disabled();
        assert_eq!(
            runtime
                .attach(pb::ShellAttachRequest::new())
                .error
                .unwrap()
                .kind
                .enum_value()
                .unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_SHELL_DISABLED
        );
        let mut invalid_attach = pb::ShellAttachRequest::new();
        invalid_attach.name = Some("\u{1b}[31m".to_owned());
        let invalid = runtime.attach(invalid_attach);
        assert_eq!(invalid.resolved_name, "<invalid>");
        assert_eq!(
            invalid.error.unwrap().kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_INVALID_NAME
        );
        assert_eq!(
            runtime.list().error.unwrap().kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_SHELL_DISABLED
        );
        assert_eq!(
            runtime
                .detach(None)
                .error
                .unwrap()
                .kind
                .enum_value()
                .unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_SHELL_DISABLED
        );
        assert_eq!(
            runtime
                .kill("default".to_owned())
                .error
                .unwrap()
                .kind
                .enum_value()
                .unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_GUEST_SHELL_DISABLED
        );
    }

    #[test]
    fn admission_caps_are_enforced() {
        let runtime = enabled_runtime();
        let first = runtime.attach(pb::ShellAttachRequest::new());
        assert!(first.error.is_none());
        let mut second = pb::ShellAttachRequest::new();
        second.name = Some("other".to_owned());
        let rejected = runtime.attach(second);
        assert_eq!(
            rejected.error.unwrap().kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_ATTACH_CAPACITY_EXCEEDED
        );
        assert_eq!(
            rejected.state.enum_value().unwrap(),
            pb::ShellState::SHELL_STATE_UNSPECIFIED
        );
        let duplicate = runtime.attach(pb::ShellAttachRequest::new());
        assert_eq!(
            duplicate.state.enum_value().unwrap(),
            pb::ShellState::SHELL_STATE_ATTACHED
        );
    }

    #[test]
    fn terminal_shell_errors_have_actionable_remediation() {
        let runtime = enabled_runtime();
        let mut invalid = pb::ShellAttachRequest::new();
        invalid.name = Some("bad/name".to_owned());
        let invalid = runtime.attach(invalid);
        let invalid_error = invalid.error.unwrap();
        assert_eq!(
            invalid_error.kind.enum_value().unwrap(),
            pb::GuestControlErrorKind::GUEST_CONTROL_ERROR_KIND_SHELL_INVALID_NAME
        );
        assert_eq!(
            invalid_error.remediation.enum_value().unwrap(),
            pb::HealthRemediation::HEALTH_REMEDIATION_NONE
        );
        assert_eq!(
            invalid.state.enum_value().unwrap(),
            pb::ShellState::SHELL_STATE_UNSPECIFIED
        );

        let disabled = ShellRuntime::disabled().attach(pb::ShellAttachRequest::new());
        assert_eq!(
            disabled.error.unwrap().remediation.enum_value().unwrap(),
            pb::HealthRemediation::HEALTH_REMEDIATION_CHECK_GUESTD_SERVICE
        );

        let first = runtime.attach(pb::ShellAttachRequest::new());
        assert!(first.error.is_none());
        let mut other = pb::ShellAttachRequest::new();
        other.name = Some("other".to_owned());
        let capacity = runtime.attach(other);
        assert_eq!(
            capacity.error.unwrap().remediation.enum_value().unwrap(),
            pb::HealthRemediation::HEALTH_REMEDIATION_REDUCE_LOAD
        );
    }

    #[test]
    fn event_queue_reports_bounded_lifecycle_events() {
        let runtime = enabled_runtime();
        let attached = runtime.attach(pb::ShellAttachRequest::new());
        let mut force = pb::ShellAttachRequest::new();
        force.force = true;
        let forced = runtime.attach(force);
        let _ = runtime.detach(None);
        let _ = runtime.kill("default".to_owned());

        assert!(attached.error.is_none());
        assert!(forced.force_evicted);

        let batch = runtime.read_events_since(0, 16);
        let kinds: Vec<ShellEventKind> = batch.events.iter().map(|event| event.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ShellEventKind::AttachCreated,
                ShellEventKind::ForceEvicted,
                ShellEventKind::AttachReused,
                ShellEventKind::Detached,
                ShellEventKind::Killed,
            ]
        );
        assert_eq!(batch.dropped_events, 0);
        assert_eq!(batch.cursor, 5);
    }

    #[test]
    fn event_queue_drops_oldest_and_reports_gap_count() {
        let mut queue = ShellEventQueue::new(2);
        queue.push(ShellEventKind::AttachCreated);
        queue.push(ShellEventKind::Detached);
        queue.push(ShellEventKind::Killed);

        let batch = queue.read_since(0, 8);
        assert_eq!(batch.dropped_events, 1);
        assert_eq!(
            batch
                .events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![
                ShellEventKind::ReconciliationGap,
                ShellEventKind::Detached,
                ShellEventKind::Killed
            ]
        );
        assert_eq!(batch.events[0].seq, 1);
        assert_eq!(batch.cursor, 3);
    }

    #[test]
    fn event_queue_cursor_advances_only_to_returned_event() {
        let mut queue = ShellEventQueue::new(8);
        queue.push(ShellEventKind::AttachCreated);
        queue.push(ShellEventKind::Detached);
        queue.push(ShellEventKind::Killed);

        let batch = queue.read_since(0, 2);
        assert_eq!(batch.events.len(), 2);
        assert_eq!(batch.cursor, 2);
        let next = queue.read_since(batch.cursor, 8);
        assert_eq!(
            next.events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![ShellEventKind::Killed]
        );
        assert_eq!(next.cursor, 3);
    }

    #[test]
    fn core_shell_adapter_maps_generation_summaries_and_event_gaps() {
        let runtime = enabled_runtime();
        let attached = runtime.attach(pb::ShellAttachRequest::new());
        assert!(attached.error.is_none());

        let listed = runtime.list_core().expect("core list");
        assert_eq!(
            listed.generation.guest_boot_id,
            constellation::ProtocolToken::parse("boot-1").unwrap()
        );
        assert_eq!(listed.summaries.len(), 1);
        assert!(listed.summaries[0].attached);
        assert!(listed.summaries[0].session_instance_id.is_some());

        let mut queue = ShellEventQueue::new(1);
        queue.push(ShellEventKind::AttachCreated);
        queue.push(ShellEventKind::Killed);
        let state = ShellRuntimeState {
            sessions: BTreeMap::new(),
            next_session: 0,
            events: queue,
        };
        let gap = state.read_events_since(0, 8);
        let generation = runtime.core_generation().unwrap();
        let events = gap
            .events
            .iter()
            .map(|event| shell_event_to_core(*event, &generation))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            events[0].cause,
            constellation::ShellCause::ReconciliationGap
        );
        assert_eq!(events[0].state, constellation::ShellState::Lost);

        let core_batch = runtime.events_core_since(0, 16).expect("core event batch");
        assert_eq!(core_batch.cursor.as_str(), "shell-event-1");
        assert!(!core_batch.reconciliation_gap);
    }

    #[test]
    fn core_shell_adapter_fails_closed_for_disabled_or_bad_generation() {
        assert_eq!(
            ShellRuntime::disabled().list_core().unwrap_err(),
            ShellCoreAdaptError::Disabled
        );
        let mut config = ShellRuntimeConfig::disabled();
        config.workload_user = Some("alice".to_owned());
        config.workload_uid = Some(1000);
        config.guest_boot_id = "bad token with spaces".to_owned();
        config.guestd_instance_id = "guestd-1".to_owned();
        config.daemon_instance_id = "daemon-1".to_owned();
        let runtime = ShellRuntime::enabled(config);
        assert_eq!(
            runtime.core_generation().unwrap_err(),
            ShellCoreAdaptError::InvalidGeneration
        );
    }

    #[test]
    fn invalid_names_are_redacted_in_error_payloads() {
        let runtime = enabled_runtime();
        let mut attach = pb::ShellAttachRequest::new();
        attach.name = Some("\u{1b}[31m".to_owned());
        assert_eq!(runtime.attach(attach).resolved_name, "<invalid>");
        assert_eq!(
            runtime.detach(Some("bad/name".to_owned())).resolved_name,
            "<invalid>"
        );
        assert_eq!(runtime.kill("bad/name".to_owned()).name, "<invalid>");
    }
}
