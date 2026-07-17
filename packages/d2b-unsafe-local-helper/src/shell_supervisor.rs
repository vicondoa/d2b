use std::collections::BTreeMap;
use std::fmt;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Duration;

use crate::output_ring::{
    DEFAULT_RING_BYTES, OutputBudget, OutputRing, RingRead, RingReservation, RingReservationError,
};
use crate::supervisor_protocol::{ShellState, valid_id};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellSupervisorError {
    LegacyEntrypointDisabled,
    ScopeOwnershipMismatch,
    ReservationExhausted,
    AlreadyExists,
    NotFound,
    AlreadyAttached,
    AttachmentMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeValidationError;

pub(crate) fn run_shell_supervisor() -> Result<(), ShellSupervisorError> {
    Err(ShellSupervisorError::LegacyEntrypointDisabled)
}

#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedTransientScope {
    resource_id: String,
    unit_name: String,
    invocation_id: String,
    control_group: String,
    owner_uid: u32,
    session_generation: u64,
}

impl VerifiedTransientScope {
    pub fn new(
        resource_id: String,
        unit_name: String,
        invocation_id: String,
        control_group: String,
        owner_uid: u32,
        session_generation: u64,
    ) -> Result<Self, ScopeValidationError> {
        let expected_unit_name = format!("d2b-shell-{resource_id}.scope");
        let valid_scope = valid_id(&resource_id)
            && unit_name == expected_unit_name
            && unit_name.len() <= 192
            && opaque_token(&invocation_id)
            && control_group.starts_with('/')
            && control_group.len() <= 256
            && !control_group.split('/').any(|part| part == "..")
            && owner_uid != 0
            && session_generation != 0;
        if !valid_scope {
            return Err(ScopeValidationError);
        }
        Ok(Self {
            resource_id,
            unit_name,
            invocation_id,
            control_group,
            owner_uid,
            session_generation,
        })
    }

    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    pub fn owner_uid(&self) -> u32 {
        self.owner_uid
    }

    pub fn session_generation(&self) -> u64 {
        self.session_generation
    }

    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        self == other
    }
}

impl fmt::Debug for VerifiedTransientScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedTransientScope(<redacted>)")
    }
}

fn opaque_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeOwnership {
    Exact,
    Ambiguous,
    Mismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeProcessState {
    Running,
    Exited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeInspection {
    pub ownership: ScopeOwnership,
    pub process_state: ScopeProcessState,
}

struct TerminalAttachment {
    stream_id: String,
    fd: OwnedFd,
}

impl fmt::Debug for TerminalAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TerminalAttachment(<redacted>)")
    }
}

pub(crate) struct ShellSupervisor {
    scope: VerifiedTransientScope,
    ring: Arc<OutputRing>,
    terminal: Option<TerminalAttachment>,
    state: ShellState,
}

impl fmt::Debug for ShellSupervisor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellSupervisor")
            .field("scope", &"<redacted>")
            .field("state", &self.state)
            .field("attached", &self.terminal.is_some())
            .finish()
    }
}

impl ShellSupervisor {
    fn new(
        scope: VerifiedTransientScope,
        ring: OutputRing,
        ownership: ScopeOwnership,
    ) -> Result<Self, ShellSupervisorError> {
        let state = match ownership {
            ScopeOwnership::Exact => ShellState::Running,
            ScopeOwnership::Ambiguous => ShellState::Degraded,
            ScopeOwnership::Mismatch => {
                return Err(ShellSupervisorError::ScopeOwnershipMismatch);
            }
        };
        Ok(Self {
            scope,
            ring: Arc::new(ring),
            terminal: None,
            state,
        })
    }

    pub(crate) fn scope(&self) -> &VerifiedTransientScope {
        &self.scope
    }

    pub(crate) fn state(&self) -> ShellState {
        if self.terminal.is_some() {
            ShellState::Attached
        } else {
            self.state
        }
    }

    pub(crate) fn reconcile(&mut self, inspection: ScopeInspection) {
        self.state = match (inspection.ownership, inspection.process_state) {
            (ScopeOwnership::Exact, ScopeProcessState::Running) => ShellState::Running,
            (ScopeOwnership::Exact, ScopeProcessState::Exited) => ShellState::Exited,
            (ScopeOwnership::Ambiguous, _) | (ScopeOwnership::Mismatch, _) => ShellState::Degraded,
        };
        if self.state != ShellState::Running {
            self.detach_any();
        }
    }

    pub(crate) fn attach(
        &mut self,
        stream_id: String,
        fd: OwnedFd,
    ) -> Result<(), ShellSupervisorError> {
        if self.state != ShellState::Running {
            return Err(ShellSupervisorError::ScopeOwnershipMismatch);
        }
        if self.terminal.is_some() {
            return Err(ShellSupervisorError::AlreadyAttached);
        }
        self.terminal = Some(TerminalAttachment { stream_id, fd });
        Ok(())
    }

    pub(crate) fn detach(&mut self, stream_id: &str) -> Result<(), ShellSupervisorError> {
        match self.terminal.as_ref() {
            Some(terminal) if terminal.stream_id == stream_id => {
                self.detach_any();
                Ok(())
            }
            Some(_) => Err(ShellSupervisorError::AttachmentMismatch),
            None => Ok(()),
        }
    }

    pub(crate) fn detach_any(&mut self) {
        if let Some(attachment) = self.terminal.take() {
            drop(attachment.fd);
        }
    }

    pub(crate) fn append_output(&self, bytes: &[u8]) {
        self.ring.append(bytes);
    }

    pub(crate) fn read_output(
        &self,
        cursor: u64,
        max_len: usize,
        wait: bool,
        timeout: Duration,
    ) -> RingRead {
        self.ring.read(cursor, max_len, wait, timeout)
    }

    pub(crate) fn close(&mut self) {
        self.detach_any();
        self.ring.close();
        self.state = ShellState::Exited;
    }
}

pub(crate) struct ShellRegistry {
    supervisors: BTreeMap<String, ShellSupervisor>,
    budget: OutputBudget,
}

impl fmt::Debug for ShellRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ShellRegistry")
            .field("supervisor_count", &self.supervisors.len())
            .field("budget", &self.budget)
            .finish()
    }
}

impl ShellRegistry {
    pub(crate) fn new(total_output_bytes: usize) -> Self {
        Self {
            supervisors: BTreeMap::new(),
            budget: OutputBudget::new(total_output_bytes),
        }
    }

    #[cfg(test)]
    pub(crate) fn insert(
        &mut self,
        scope: VerifiedTransientScope,
        output_ring_bytes: usize,
        ownership: ScopeOwnership,
    ) -> Result<(), ShellSupervisorError> {
        let reservation = self.reserve(scope.resource_id(), output_ring_bytes)?;
        self.insert_reserved(scope, reservation, ownership)
    }

    pub(crate) fn reserve(
        &self,
        resource_id: &str,
        output_ring_bytes: usize,
    ) -> Result<RingReservation, ShellSupervisorError> {
        if self.supervisors.contains_key(resource_id) {
            return Err(ShellSupervisorError::AlreadyExists);
        }
        let capacity = if output_ring_bytes == 0 {
            DEFAULT_RING_BYTES
        } else {
            output_ring_bytes
        };
        self.budget.reserve(capacity).map_err(|error| match error {
            RingReservationError::InvalidCapacity | RingReservationError::Exhausted => {
                ShellSupervisorError::ReservationExhausted
            }
        })
    }

    pub(crate) fn insert_reserved(
        &mut self,
        scope: VerifiedTransientScope,
        reservation: RingReservation,
        ownership: ScopeOwnership,
    ) -> Result<(), ShellSupervisorError> {
        if self.supervisors.contains_key(scope.resource_id()) {
            return Err(ShellSupervisorError::AlreadyExists);
        }
        let key = scope.resource_id().to_owned();
        self.supervisors.insert(
            key,
            ShellSupervisor::new(scope, OutputRing::new(reservation), ownership)?,
        );
        Ok(())
    }

    pub(crate) fn get(&self, resource_id: &str) -> Result<&ShellSupervisor, ShellSupervisorError> {
        self.supervisors
            .get(resource_id)
            .ok_or(ShellSupervisorError::NotFound)
    }

    pub(crate) fn get_mut(
        &mut self,
        resource_id: &str,
    ) -> Result<&mut ShellSupervisor, ShellSupervisorError> {
        self.supervisors
            .get_mut(resource_id)
            .ok_or(ShellSupervisorError::NotFound)
    }

    pub(crate) fn remove(
        &mut self,
        resource_id: &str,
    ) -> Result<ShellSupervisor, ShellSupervisorError> {
        self.supervisors
            .remove(resource_id)
            .ok_or(ShellSupervisorError::NotFound)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &ShellSupervisor)> {
        self.supervisors.iter()
    }

    pub(crate) fn detach_all(&mut self) {
        for supervisor in self.supervisors.values_mut() {
            supervisor.detach_any();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(resource_id: &str) -> VerifiedTransientScope {
        VerifiedTransientScope::new(
            resource_id.into(),
            format!("d2b-shell-{resource_id}.scope"),
            "00112233445566778899aabbccddeeff".into(),
            format!("/user.slice/{resource_id}"),
            nix::unistd::getuid().as_raw(),
            7,
        )
        .unwrap()
    }

    #[test]
    fn ambiguous_adoption_is_preserved_degraded() {
        let mut registry = ShellRegistry::new(DEFAULT_RING_BYTES);
        registry
            .insert(
                scope("alpha"),
                DEFAULT_RING_BYTES,
                ScopeOwnership::Ambiguous,
            )
            .unwrap();
        assert_eq!(registry.get("alpha").unwrap().state(), ShellState::Degraded);
    }

    #[test]
    fn disconnect_detaches_without_removing_or_closing_shell() {
        let mut registry = ShellRegistry::new(DEFAULT_RING_BYTES);
        registry
            .insert(scope("alpha"), DEFAULT_RING_BYTES, ScopeOwnership::Exact)
            .unwrap();
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        registry
            .get_mut("alpha")
            .unwrap()
            .attach("terminal".into(), stream.into())
            .unwrap();
        registry.detach_all();
        assert_eq!(registry.get("alpha").unwrap().state(), ShellState::Running);
    }

    #[test]
    fn debug_redacts_scope_and_terminal_identifiers() {
        let canary = "private-shell-canary";
        let scope = VerifiedTransientScope::new(
            "alpha".into(),
            "d2b-shell-alpha.scope".into(),
            canary.into(),
            format!("/user.slice/{canary}"),
            nix::unistd::getuid().as_raw(),
            7,
        )
        .unwrap();
        assert!(!format!("{scope:?}").contains(canary));
    }
}
