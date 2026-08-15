//! Pool/session controller lifecycle without persistent Provider state.

use std::collections::BTreeMap;

use crate::{
    Authorizer, ShellPool, ShellSession, ShellTerminalError, Subject,
    resources::validate_name,
    service::supervisor::{SessionCapability, SessionSupervisor},
    session::SupervisorIdentity,
};

/// A validated request to create one pool-derived shell session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSessionRequest {
    pool_name: String,
    session_name: String,
    output_ring_capacity: Option<u64>,
}

impl OpenSessionRequest {
    /// Construct a bounded session request.
    pub fn new(
        pool_name: impl Into<String>,
        session_name: impl Into<String>,
        output_ring_capacity: Option<u64>,
    ) -> Result<Self, ShellTerminalError> {
        let pool_name = pool_name.into();
        let session_name = session_name.into();
        validate_name(&pool_name, 63)?;
        validate_name(&session_name, 32)?;
        Ok(Self {
            pool_name,
            session_name,
            output_ring_capacity,
        })
    }
}

/// A controller response carrying a session, its generation, and a one-shot capability.
#[derive(Debug, Clone)]
pub struct OpenSessionResult {
    session: ShellSession,
    pool: ShellPool,
    supervisor_generation: u64,
    capability: SessionCapability,
}

impl OpenSessionResult {
    /// Borrow the newly-created session.
    pub const fn session(&self) -> &ShellSession {
        &self.session
    }

    /// Return the generation required by every supervisor request.
    pub const fn supervisor_generation(&self) -> u64 {
        self.supervisor_generation
    }

    /// Return the current request's one-shot supervisor capability.
    pub const fn capability(&self) -> SessionCapability {
        self.capability
    }

    /// Build an in-memory supervisor model once the process adapter proves identity.
    pub fn start_supervisor(
        &self,
        identity: SupervisorIdentity,
    ) -> Result<SessionSupervisor, ShellTerminalError> {
        if identity.generation() != self.supervisor_generation {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        Ok(SessionSupervisor::new(
            self.session.clone(),
            self.pool.clone(),
            identity,
        ))
    }
}

/// Bounded controller state reconstructed from resource objects on restart.
#[derive(Debug, Default)]
pub struct ShellTerminalController {
    pools: BTreeMap<String, ShellPool>,
    sessions: BTreeMap<String, ShellSession>,
    next_capability: u64,
}

impl ShellTerminalController {
    /// Insert a reconciled pool into the bounded controller projection.
    pub fn insert_pool(&mut self, pool: ShellPool) -> Result<(), ShellTerminalError> {
        if self.pools.contains_key(pool.name()) {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        self.pools.insert(pool.name().to_owned(), pool);
        Ok(())
    }

    /// Create a session after authorizing the current request and enforcing pool capacity.
    pub fn open_session(
        &mut self,
        subject: &Subject,
        request: OpenSessionRequest,
    ) -> Result<OpenSessionResult, ShellTerminalError> {
        let pool = self
            .pools
            .get(&request.pool_name)
            .ok_or(ShellTerminalError::CapacityExceeded)?;
        Authorizer::authorize(subject, pool)?;
        if self.session_count(pool.name()) >= pool.active_session_capacity() {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        let resource_name = format!("{}-{}", pool.name(), request.session_name);
        if self.sessions.contains_key(&resource_name) {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        let session = ShellSession::from_pool(
            pool,
            resource_name.clone(),
            request.session_name,
            request.output_ring_capacity,
        )?;
        self.next_capability = self.next_capability.saturating_add(1);
        let result = OpenSessionResult {
            session: session.clone(),
            pool: pool.clone(),
            supervisor_generation: 1,
            capability: SessionCapability::new(self.next_capability, 1),
        };
        self.sessions.insert(resource_name, session);
        Ok(result)
    }

    /// Return the number of sessions belonging to one pool.
    pub fn session_count(&self, pool_name: &str) -> u32 {
        self.sessions
            .values()
            .filter(|session| session.pool_name() == pool_name)
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    /// Return whether the Provider correctly declares no persistent state set.
    pub const fn provider_state_is_empty(&self) -> bool {
        true
    }
}
