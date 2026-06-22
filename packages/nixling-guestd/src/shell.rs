use std::collections::BTreeMap;
use std::sync::Mutex;

use nixling_ipc::guest_proto as pb;
use protobuf::{EnumOrUnknown, MessageField};

pub const DEFAULT_SHELL_SESSIONS_PER_VM: u32 = 8;
pub const DEFAULT_SHELL_ATTACHED_SESSIONS_PER_VM: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRuntimeConfig {
    pub default_name: String,
    pub max_sessions: u32,
    pub max_attached: u32,
}

impl ShellRuntimeConfig {
    pub fn disabled() -> Self {
        Self {
            default_name: "default".to_owned(),
            max_sessions: DEFAULT_SHELL_SESSIONS_PER_VM,
            max_attached: DEFAULT_SHELL_ATTACHED_SESSIONS_PER_VM,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellSession {
    name: String,
    session_id: String,
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

#[derive(Default)]
struct ShellRuntimeState {
    sessions: BTreeMap<String, ShellSession>,
    next_session: u64,
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
        let name = request
            .name
            .as_deref()
            .unwrap_or(&self.config.default_name)
            .to_owned();
        let result = self.attach_inner(name.clone(), request.force);
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
        if let Some(existing_view) = state.sessions.get(&name) {
            let was_attached = existing_view.attached;
            if was_attached && !force {
                return Err(ShellRuntimeError::AlreadyAttached);
            }
            if !was_attached && attached_count >= self.config.max_attached as usize {
                return Err(ShellRuntimeError::AttachCapacityExceeded);
            }
            let existing = state
                .sessions
                .get_mut(&name)
                .expect("existing session remains present");
            existing.attached = true;
            existing.killed = false;
            return Ok((existing.clone(), was_attached && force));
        }
        if state.sessions.len() >= self.config.max_sessions as usize {
            return Err(ShellRuntimeError::CapacityExceeded);
        }
        if attached_count >= self.config.max_attached as usize {
            return Err(ShellRuntimeError::AttachCapacityExceeded);
        }
        state.next_session = state.next_session.saturating_add(1);
        let session = ShellSession {
            name: name.clone(),
            session_id: format!("shell-{:016x}", state.next_session),
            attached: true,
            killed: false,
        };
        state.sessions.insert(name, session.clone());
        Ok((session, false))
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
        let Some(mut session) = state.sessions.remove(&name) else {
            return shell_kill_error(ShellRuntimeError::NotFound, name);
        };
        session.attached = false;
        session.killed = true;
        let mut response = pb::ShellKillResponse::new();
        response.name = name;
        response.killed = true;
        response.state = EnumOrUnknown::new(pb::ShellState::SHELL_STATE_KILLED);
        response
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
    response.resolved_name = name;
    response.state = EnumOrUnknown::new(pb::ShellState::SHELL_STATE_FEATURE_DISABLED);
    response.error = MessageField::some(shell_error(error));
    response
}

fn shell_detach_error(error: ShellRuntimeError, name: String) -> pb::ShellDetachResponse {
    let mut response = pb::ShellDetachResponse::new();
    response.resolved_name = name;
    response.error = MessageField::some(shell_error(error));
    response
}

fn shell_kill_error(error: ShellRuntimeError, name: String) -> pb::ShellKillResponse {
    let mut response = pb::ShellKillResponse::new();
    response.name = name;
    response.state = EnumOrUnknown::new(pb::ShellState::SHELL_STATE_FEATURE_DISABLED);
    response.error = MessageField::some(shell_error(error));
    response
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

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_runtime() -> ShellRuntime {
        ShellRuntime::enabled(ShellRuntimeConfig {
            default_name: "default".to_owned(),
            max_sessions: 2,
            max_attached: 1,
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
        assert!(
            runtime
                .attach(pb::ShellAttachRequest::new())
                .error
                .is_none()
        );
        let mut request = pb::ShellAttachRequest::new();
        request.force = true;
        let forced = runtime.attach(request);
        assert!(forced.force_evicted);
        assert!(forced.error.is_none());
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
}
