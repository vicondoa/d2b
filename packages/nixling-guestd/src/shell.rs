use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

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
        }
    }
}

pub trait ShellDaemonManager: Send + Sync {
    fn ensure_ready(
        &self,
        config: &ShellRuntimeConfig,
    ) -> Result<String, pb::GuestControlErrorKind>;
}

pub trait ShellHelperSpawner: Send + Sync {
    fn spawn_helper(&self, name: &str) -> Result<String, pb::GuestControlErrorKind>;
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
    daemon_instance_id: String,
    guest_boot_id: String,
    owner_key: Vec<u8>,
    attached: bool,
    killed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellRuntimeError {
    InvalidName,
    CapacityExceeded,
    AttachCapacityExceeded,
    AlreadyAttached,
    NotFound,
    Disabled,
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

    fn drain_since(&self, after_seq: u64, limit: usize) -> ShellEventBatch {
        let mut events = Vec::new();
        if self.dropped_events > 0
            && limit > 0
            && self
                .events
                .front()
                .is_some_and(|oldest| oldest.seq > after_seq.saturating_add(1))
        {
            events.push(ShellEvent {
                seq: after_seq.saturating_add(1),
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
        if !self.enabled {
            return Err(ShellRuntimeError::Disabled);
        }
        validate_shell_name(&name).map_err(|_| ShellRuntimeError::InvalidName)?;
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
            existing.killed = false;
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
        let session = ShellSession {
            name: name.clone(),
            session_id: format!("shell-{:016x}", state.next_session),
            shell_session_instance_id: format!("shell-instance-{:016x}", state.next_session),
            daemon_instance_id: self.config.daemon_instance_id.clone(),
            guest_boot_id: self.config.guest_boot_id.clone(),
            owner_key,
            attached: true,
            killed: false,
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
        for session in state.sessions.values().filter(|session| !session.killed) {
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
            return shell_detach_error(ShellRuntimeError::NotFound, resolved);
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
            return shell_kill_error(ShellRuntimeError::NotFound, name);
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

    pub fn drain_events_since(&self, after_seq: u64, limit: usize) -> ShellEventBatch {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.drain_events_since(after_seq, limit)
    }
}

impl ShellRuntimeState {
    fn drain_events_since(&self, after_seq: u64, limit: usize) -> ShellEventBatch {
        self.events.drain_since(after_seq, limit)
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
    response.state = EnumOrUnknown::new(pb::ShellState::SHELL_STATE_FEATURE_DISABLED);
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
    response.state = EnumOrUnknown::new(pb::ShellState::SHELL_STATE_FEATURE_DISABLED);
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
    wire.remediation = EnumOrUnknown::new(pb::HealthRemediation::HEALTH_REMEDIATION_RETRY);
    wire
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

        let batch = runtime.drain_events_since(0, 16);
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

        let batch = queue.drain_since(0, 8);
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
        assert_eq!(batch.cursor, 3);
    }

    #[test]
    fn event_queue_cursor_advances_only_to_returned_event() {
        let mut queue = ShellEventQueue::new(8);
        queue.push(ShellEventKind::AttachCreated);
        queue.push(ShellEventKind::Detached);
        queue.push(ShellEventKind::Killed);

        let batch = queue.drain_since(0, 2);
        assert_eq!(batch.events.len(), 2);
        assert_eq!(batch.cursor, 2);
        let next = queue.drain_since(batch.cursor, 8);
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
