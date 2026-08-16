//! Pool/session controller lifecycle without persistent Provider state.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use crate::{
    Authorizer, ShellPool, ShellSession, ShellTerminalError, Subject,
    resources::validate_name,
    service::supervisor::{PoolAttachmentBudget, SessionCapability, SessionSupervisor},
    session::{AdoptionDecision, SupervisorCandidate, SupervisorIdentity, adopt_supervisor},
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
    supervisor_generation: u64,
    capability: SessionCapability,
    attachment_budget: Arc<PoolAttachmentBudget>,
    generation: Arc<Mutex<u64>>,
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
    pub fn capability(&self) -> SessionCapability {
        self.capability.clone()
    }

    /// Build an in-memory supervisor model once the process adapter proves identity.
    pub fn start_supervisor(
        &self,
        identity: SupervisorIdentity,
    ) -> Result<SessionSupervisor, ShellTerminalError> {
        if identity.generation() != self.supervisor_generation {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        if *self
            .generation
            .lock()
            .map_err(|_| ShellTerminalError::StaleSessionGeneration)?
            != self.supervisor_generation
        {
            return Err(ShellTerminalError::StaleSessionGeneration);
        }
        Ok(SessionSupervisor::new(
            self.session.clone(),
            identity,
            Arc::clone(&self.attachment_budget),
            Arc::clone(&self.generation),
        ))
    }
}

/// Bounded controller state reconstructed from resource objects on restart.
#[derive(Debug, Default)]
pub struct ShellTerminalController {
    pools: BTreeMap<String, ShellPool>,
    attachment_budgets: BTreeMap<String, Arc<PoolAttachmentBudget>>,
    sessions: BTreeMap<String, ShellSession>,
    session_generations: BTreeMap<String, Arc<Mutex<u64>>>,
    next_capability: u64,
}

impl ShellTerminalController {
    /// Insert a reconciled pool into the bounded controller projection.
    pub fn insert_pool(&mut self, pool: ShellPool) -> Result<(), ShellTerminalError> {
        self.restore_pool(pool, 0)
    }

    /// Restore a pool with the authoritative count of adopted attachments.
    ///
    /// Restored occupancy blocks new streams until the next status reconcile
    /// proves capacity. This intentionally favors refusal over potentially
    /// exceeding a pool's attachment limit after controller restart.
    pub fn restore_pool(
        &mut self,
        pool: ShellPool,
        attached_streams: u32,
    ) -> Result<(), ShellTerminalError> {
        if self.pools.contains_key(pool.name()) {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        let pool_name = pool.name().to_owned();
        self.attachment_budgets.insert(
            pool_name.clone(),
            Arc::new(PoolAttachmentBudget::restored(
                pool.spec().max_attached(),
                attached_streams,
            )?),
        );
        self.pools.insert(pool_name, pool);
        Ok(())
    }

    /// Replace restart-restored occupancy with the latest pool status.
    pub fn reconcile_pool_attachments(
        &self,
        pool_name: &str,
        attached_streams: u32,
    ) -> Result<(), ShellTerminalError> {
        self.attachment_budgets
            .get(pool_name)
            .ok_or(ShellTerminalError::CapacityExceeded)?
            .reconcile_retained_attachments(attached_streams)
    }

    /// Restore a reconciled session before the controller admits new sessions.
    ///
    /// The session remains counted for capacity even when the supervisor is
    /// missing or ambiguous, preventing a restart from recreating a resource
    /// name while its earlier process may still exist.
    pub fn restore_session(
        &mut self,
        session: ShellSession,
        expected_identity: &SupervisorIdentity,
        candidates: &[SupervisorCandidate],
    ) -> Result<AdoptionDecision, ShellTerminalError> {
        if !self.pools.contains_key(session.pool_name())
            || self.sessions.contains_key(session.name())
        {
            return Err(ShellTerminalError::CapacityExceeded);
        }
        let decision = adopt_supervisor(session.name(), expected_identity, candidates);
        let session_name = session.name().to_owned();
        self.session_generations.insert(
            session_name.clone(),
            Arc::new(Mutex::new(expected_identity.generation())),
        );
        self.sessions.insert(session_name, session);
        Ok(decision)
    }

    /// Advance one reconciled session after its prior supervisor is retired.
    pub fn restart_supervisor(
        &mut self,
        subject: &Subject,
        session_name: &str,
    ) -> Result<OpenSessionResult, ShellTerminalError> {
        let session = self
            .sessions
            .get(session_name)
            .cloned()
            .ok_or(ShellTerminalError::CapacityExceeded)?;
        let pool = self
            .pools
            .get(session.pool_name())
            .ok_or(ShellTerminalError::CapacityExceeded)?;
        Authorizer::authorize(subject, pool)?;
        let generation = Arc::clone(
            self.session_generations
                .get(session_name)
                .ok_or(ShellTerminalError::StaleSessionGeneration)?,
        );
        let mut current = generation
            .lock()
            .map_err(|_| ShellTerminalError::StaleSessionGeneration)?;
        *current = current
            .checked_add(1)
            .ok_or(ShellTerminalError::StaleSessionGeneration)?;
        let supervisor_generation = *current;
        drop(current);
        self.next_capability = self.next_capability.saturating_add(1);
        let attachment_budget = Arc::clone(
            self.attachment_budgets
                .get(session.pool_name())
                .ok_or(ShellTerminalError::CapacityExceeded)?,
        );
        Ok(OpenSessionResult {
            capability: SessionCapability::new(
                self.next_capability,
                supervisor_generation,
                session.name().to_owned(),
            ),
            session,
            supervisor_generation,
            attachment_budget,
            generation,
        })
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
        let attachment_budget = Arc::clone(
            self.attachment_budgets
                .get(&request.pool_name)
                .ok_or(ShellTerminalError::CapacityExceeded)?,
        );
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
        let generation = Arc::new(Mutex::new(1));
        self.next_capability = self.next_capability.saturating_add(1);
        let result = OpenSessionResult {
            session: session.clone(),
            supervisor_generation: 1,
            capability: SessionCapability::new(self.next_capability, 1, resource_name.clone()),
            attachment_budget,
            generation: Arc::clone(&generation),
        };
        self.sessions.insert(resource_name, session);
        self.session_generations
            .insert(result.session.name().to_owned(), generation);
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
